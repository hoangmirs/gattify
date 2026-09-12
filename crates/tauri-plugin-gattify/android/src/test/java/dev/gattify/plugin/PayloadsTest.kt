package dev.gattify.plugin

import org.json.JSONObject
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class PayloadsTest {
  private fun rejects(expectedCode: String = "invalidArgument", block: () -> Unit) {
    try {
      block()
      fail("expected $expectedCode")
    } catch (e: BleException) {
      assertEquals(e.message, expectedCode, e.code)
    }
  }

  private val characteristic = """
    { "instanceKey": "info", "uuid": "b1e10f10-6a2c-4a62-8e9e-2c938fa30101",
      "properties": { "read": true, "write": false, "writeWithoutResponse": false, "notify": false, "indicate": false },
      "initialValueBase64": "AQ==", "maxValueLength": 1 }
  """

  private fun server(characteristics: String = characteristic, services: String? = null) = JSONObject(
    services ?: """{ "services": [{ "instanceKey": "peer", "uuid": "80ff87c3-8e84-4914-aedc-0d6a3ba5534d",
      "primary": true, "characteristics": [$characteristics] }] }""",
  )

  @Test
  fun `parseRequest reads the execute arguments`() {
    val request = parseRequest(
      JSONObject(
        """{ "operationId": "op-1", "ownerId": "webview:main", "deadlineMillis": 5000,
          "command": { "kind": "stopScan", "payload": { "scanId": "scan-1" } } }""",
      ),
    )
    assertEquals("op-1", request.operationId)
    assertEquals("webview:main", request.ownerId)
    assertEquals(5000L, request.deadlineMillis)
    assertEquals("stopScan", request.kind)
    assertEquals("scan-1", request.payload().getString("scanId"))
  }

  @Test
  fun `parseRequest accepts a null deadline and a command without a payload`() {
    val request = parseRequest(
      JSONObject("""{ "operationId": "op-2", "ownerId": "gattify-peer:main", "deadlineMillis": null, "command": { "kind": "getState" } }"""),
    )
    assertNull(request.deadlineMillis)
    assertEquals("getState", request.kind)
    rejects { request.payload() }
  }

  @Test
  fun `parseRequest requires the operation and the owner`() {
    rejects { parseRequest(JSONObject("""{ "ownerId": "webview:main", "command": { "kind": "getState" } }""")) }
    rejects { parseRequest(JSONObject("""{ "operationId": "op-1", "command": { "kind": "getState" } }""")) }
    rejects { parseRequest(JSONObject("""{ "operationId": 7, "ownerId": "webview:main" }""")) }
  }

  @Test
  fun `parseRequest keeps a missing kind for the unsupported rejection`() {
    val request = parseRequest(JSONObject("""{ "operationId": "op-1", "ownerId": "webview:main" }"""))
    assertNull(request.kind)
  }

  @Test
  fun `decodeScan expands and deduplicates the filter`() {
    val spec = decodeScan(
      JSONObject("""{ "serviceUuids": ["180D", "0000180d-0000-1000-8000-00805f9b34fb"], "timeoutMs": 3000 }"""),
    )
    assertEquals(listOf("0000180d-0000-1000-8000-00805f9b34fb"), spec.serviceUuids.map(::uuidString))
    assertEquals(3000L, spec.timeoutMs)
    assertNull(decodeScan(JSONObject("""{ "serviceUuids": [], "timeoutMs": null }""")).timeoutMs)
    rejects { decodeScan(JSONObject("""{ "serviceUuids": ["nope"] }""")) }
    rejects { decodeScan(JSONObject("""{ "serviceUuids": "180d" }""")) }
  }

  @Test
  fun `decodeServer builds characteristic keys, properties and values`() {
    val services = decodeServer(server())
    assertEquals(1, services.size)
    assertEquals("80ff87c3-8e84-4914-aedc-0d6a3ba5534d", uuidString(services[0].uuid))
    assertTrue(services[0].primary)
    val info = services[0].characteristics.single()
    assertEquals("peer/info", info.key)
    assertEquals("b1e10f10-6a2c-4a62-8e9e-2c938fa30101", uuidString(info.uuid))
    assertTrue(info.properties.read)
    assertFalse(info.properties.writable)
    assertArrayEquals(byteArrayOf(1), info.initialValue)
    assertEquals(1, info.maxValueLength)
  }

  @Test
  fun `decodeServer treats a missing initial value as empty`() {
    val spec = decodeServer(server(characteristic.replace(""""initialValueBase64": "AQ==",""", "")))
    assertArrayEquals(ByteArray(0), spec[0].characteristics[0].initialValue)
    val nullValue = decodeServer(server(characteristic.replace("\"AQ==\"", "null")))
    assertArrayEquals(ByteArray(0), nullValue[0].characteristics[0].initialValue)
  }

  @Test
  fun `decodeServer rejects a malformed definition`() {
    rejects { decodeServer(server(characteristic.replace("AQ==", "not base64!"))) }
    rejects { decodeServer(server(characteristic.replace("\"AQ==\"", "\"AQI=\""))) }
    rejects { decodeServer(server(characteristic.replace("\"maxValueLength\": 1", "\"maxValueLength\": 0"))) }
    rejects { decodeServer(server(characteristic.replace("b1e10f10-6a2c-4a62-8e9e-2c938fa30101", "xyz"))) }
    rejects { decodeServer(server("$characteristic, $characteristic")) }
    rejects { decodeServer(server(characteristic.replace("\"read\": true", "\"read\": 1"))) }
    rejects { decodeServer(server(services = """{ "services": [] }""")) }
  }

  @Test
  fun `decodeAdvertising reads the options`() {
    val spec = decodeAdvertising(
      JSONObject("""{ "serviceUuid": "80ff87c3-8e84-4914-aedc-0d6a3ba5534d", "localName": "Host", "localNameOptional": true }"""),
    )
    assertEquals("80ff87c3-8e84-4914-aedc-0d6a3ba5534d", uuidString(spec.serviceUuid))
    assertEquals("Host", spec.localName)
    assertTrue(spec.localNameOptional)
    val unnamed = decodeAdvertising(
      JSONObject("""{ "serviceUuid": "80ff87c3-8e84-4914-aedc-0d6a3ba5534d", "localName": null, "localNameOptional": false }"""),
    )
    assertNull(unnamed.localName)
  }

  @Test
  fun `decodeWriteType and decodePermissionAsk read the wire values`() {
    assertTrue(decodeWriteType("withResponse"))
    assertFalse(decodeWriteType("withoutResponse"))
    rejects { decodeWriteType("sometimes") }
    val ask = decodePermissionAsk(JSONObject("""{ "scan": true, "connect": false, "advertise": true }"""))
    assertTrue(ask.scan)
    assertFalse(ask.connect)
    assertTrue(ask.advertise)
    rejects { decodePermissionAsk(JSONObject("""{ "scan": true }""")) }
  }
}
