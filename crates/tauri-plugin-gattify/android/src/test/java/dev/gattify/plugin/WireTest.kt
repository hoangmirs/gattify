package dev.gattify.plugin

import app.tauri.plugin.JSObject
import org.json.JSONObject
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class WireTest {
  /** Round-trips through a string, as the Tauri receiver parses it. */
  private fun json(value: JSObject): JSONObject = JSONObject(value.toString())

  private val uuid = "80ff87c3-8e84-4914-aedc-0d6a3ba5534d"

  @Test
  fun `empty has no payload key`() {
    val reply = json(Replies.empty())
    assertEquals("empty", reply.getString("kind"))
    assertFalse(reply.has("payload"))
  }

  @Test
  fun `link limits are the MTU minus 3, at most 512`() {
    val cases = mapOf(23 to 20, 185 to 182, 247 to 244, 515 to 512, 517 to 512)
    for ((mtu, length) in cases) {
      val limits = json(Replies.linkLimits(mtu))
      for (key in listOf("writeWithResponse", "writeWithoutResponse", "notification")) {
        assertEquals("$key at $mtu", length, limits.getInt(key))
      }
      assertEquals(mtu, limits.getInt("attMtu"))
    }
  }

  @Test
  fun `connected carries the connection and its limits`() {
    val reply = json(Replies.connected("connection-2", 185))
    assertEquals("connected", reply.getString("kind"))
    val payload = reply.getJSONObject("payload")
    assertEquals("connection-2", payload.getString("connectionId"))
    assertEquals(182, payload.getJSONObject("limits").getInt("writeWithResponse"))
  }

  @Test
  fun `services list handles, UUIDs and properties`() {
    val reply = json(
      Replies.services(
        listOf(
          ServiceReport(
            "connection-2/service-1",
            uuid,
            listOf(
              CharacteristicReport(
                "connection-2/characteristic-1",
                "b1e10f10-6a2c-4a62-8e9e-2c938fa30101",
                Properties(read = true, write = false, writeWithoutResponse = false, notify = true, indicate = false),
              ),
            ),
          ),
        ),
      ),
    )
    assertEquals("services", reply.getString("kind"))
    val service = reply.getJSONArray("payload").getJSONObject(0)
    assertEquals("connection-2/service-1", service.getString("handle"))
    assertEquals(uuid, service.getString("uuid"))
    val characteristic = service.getJSONArray("characteristics").getJSONObject(0)
    assertEquals("connection-2/characteristic-1", characteristic.getString("handle"))
    val properties = characteristic.getJSONObject("properties")
    assertEquals(
      mapOf("read" to true, "write" to false, "writeWithoutResponse" to false, "notify" to true, "indicate" to false),
      properties.keySet().associateWith { properties.getBoolean(it) },
    )
  }

  @Test
  fun `small replies use the contract field names`() {
    assertEquals("AQI=", json(Replies.bytes(byteArrayOf(1, 2))).getJSONObject("payload").getString("valueBase64"))
    assertEquals("scan-1", json(Replies.scanStarted("scan-1")).getJSONObject("payload").getString("scanId"))
    assertEquals(
      "subscription-4",
      json(Replies.subscriptionStarted("subscription-4")).getJSONObject("payload").getString("subscriptionId"),
    )
    assertEquals("server-1", json(Replies.serverCreated("server-1")).getJSONObject("payload").getString("serverId"))

    val advertising = json(Replies.advertisingStarted(localNameIncluded = true, localNameTruncated = false))
    assertEquals("advertisingStarted", advertising.getString("kind"))
    assertTrue(advertising.getJSONObject("payload").getBoolean("localNameIncluded"))
    assertFalse(advertising.getJSONObject("payload").getBoolean("localNameTruncated"))

    val resources = json(Replies.resources(1, 2, 3, 4)).getJSONObject("payload")
    assertEquals(listOf(1, 2, 3, 4), listOf("scans", "connections", "subscriptions", "servers").map(resources::getInt))
  }

  @Test
  fun `an event goes inside the owner envelope`() {
    val envelope = json(Events.envelope("webview:main", Events.scanStopped("scan-1")))
    assertEquals(setOf("ownerId", "event"), envelope.keySet())
    assertEquals("webview:main", envelope.getString("ownerId"))
    val event = envelope.getJSONObject("event")
    assertEquals("scanStopped", event.getString("kind"))
    assertEquals("scan-1", event.getJSONObject("payload").getString("scanId"))
  }

  @Test
  fun `scanResult carries every DiscoveredDevice field`() {
    val device = DeviceReport(
      id = "device-3",
      name = "Host A",
      rssi = -58,
      serviceUuids = listOf(uuid),
      localName = null,
      serviceData = listOf(uuid to "Host A".toByteArray()),
      manufacturerData = listOf(76 to byteArrayOf(1, 2)),
      connectable = true,
      observedAtMillis = 1_757_664_000_000,
      scanId = "scan-1",
    )
    val event = json(Events.scanResult(device))
    assertEquals("scanResult", event.getString("kind"))
    val payload = event.getJSONObject("payload").getJSONObject("device")
    assertEquals("device-3", payload.getString("id"))
    assertEquals("Host A", payload.getString("name"))
    assertEquals(-58, payload.getInt("rssi"))
    assertEquals(uuid, payload.getJSONArray("serviceUuids").getString(0))
    assertEquals(1_757_664_000_000, payload.getLong("observedAtMillis"))
    assertEquals("scan-1", payload.getString("scanId"))
    val advertisement = payload.getJSONObject("advertisement")
    assertTrue(advertisement.has("localName") && advertisement.isNull("localName"))
    assertTrue(advertisement.getBoolean("connectable"))
    val serviceData = advertisement.getJSONArray("serviceData").getJSONObject(0)
    assertEquals(uuid, serviceData.getString("serviceUuid"))
    assertEquals("SG9zdCBB", serviceData.getString("bytesBase64"))
    val manufacturer = advertisement.getJSONArray("manufacturerData").getJSONObject(0)
    assertEquals(76, manufacturer.getInt("companyId"))
    assertEquals("AQI=", manufacturer.getString("bytesBase64"))
  }

  @Test
  fun `scanResult keeps absent optional fields as null and lists as empty`() {
    val device = DeviceReport("device-1", null, null, emptyList(), null, emptyList(), emptyList(), null, 0, "scan-2")
    val payload = json(Events.scanResult(device)).getJSONObject("payload").getJSONObject("device")
    assertTrue(payload.has("name") && payload.isNull("name"))
    assertTrue(payload.has("rssi") && payload.isNull("rssi"))
    assertEquals(0, payload.getJSONArray("serviceUuids").length())
    val advertisement = payload.getJSONObject("advertisement")
    assertEquals(0, advertisement.getJSONArray("serviceData").length())
    assertEquals(0, advertisement.getJSONArray("manufacturerData").length())
    assertTrue(advertisement.has("connectable") && advertisement.isNull("connectable"))
  }

  @Test
  fun `subscriptionChanged carries the size, or null when unsubscribed`() {
    val on = json(Events.subscriptionChanged("server-1", "central-1", "peer/tx", 182)).getJSONObject("payload")
    assertEquals("server-1", on.getString("serverId"))
    assertEquals("central-1", on.getString("peerId"))
    assertEquals("peer/tx", on.getString("characteristicKey"))
    assertTrue(on.getBoolean("subscribed"))
    assertEquals(182, on.getInt("maxValueLength"))

    val off = json(Events.subscriptionChanged("server-1", "central-1", "peer/tx", null)).getJSONObject("payload")
    assertFalse(off.getBoolean("subscribed"))
    assertTrue(off.has("maxValueLength") && off.isNull("maxValueLength"))
  }

  @Test
  fun `the other events use the contract field names`() {
    val write = json(Events.serverWrite("server-1", "central-2", "peer/rx", byteArrayOf(9))).getJSONObject("payload")
    assertEquals(
      mapOf("serverId" to "server-1", "peerId" to "central-2", "characteristicKey" to "peer/rx", "valueBase64" to "CQ=="),
      write.keySet().associateWith(write::getString),
    )
    val value = json(Events.characteristicValue("subscription-1", byteArrayOf(0))).getJSONObject("payload")
    assertEquals("subscription-1", value.getString("subscriptionId"))
    assertEquals("AA==", value.getString("valueBase64"))
    assertEquals(
      "connection-1",
      json(Events.connectionClosed("connection-1")).getJSONObject("payload").getString("connectionId"),
    )
    assertEquals("poweredOff", json(Events.adapterStateChanged("poweredOff")).getJSONObject("payload").getString("state"))
    val loss = json(Events.criticalStateLoss("server-1", "bluetoothOff")).getJSONObject("payload")
    assertEquals("server-1", loss.getString("resourceId"))
    assertEquals("bluetoothOff", loss.getString("reason"))
  }

  @Test
  fun `a string with quotes and control characters stays valid JSON`() {
    val tricky = "a\"b\\c\n é😀"
    val device = DeviceReport("device-1", tricky, null, emptyList(), tricky, emptyList(), emptyList(), null, 0, "scan-1")
    val payload = json(Events.envelope("webview:main", Events.scanResult(device)))
      .getJSONObject("event").getJSONObject("payload").getJSONObject("device")
    assertEquals(tricky, payload.getString("name"))
  }

  @Test
  fun `base64 decodes the standard alphabet and rejects anything else`() {
    assertArrayEquals(byteArrayOf(1, 2), decodeBase64("AQI=", "valueBase64"))
    assertArrayEquals(ByteArray(0), decodeBase64("", "valueBase64"))
    try {
      decodeBase64("not base64!", "valueBase64")
      fail("expected a rejection")
    } catch (e: BleException) {
      assertEquals("invalidArgument", e.code)
    }
  }
}
