package dev.gattify.plugin

import app.tauri.plugin.JSObject
import org.json.JSONArray
import org.json.JSONObject
import java.util.Base64

internal fun encodeBase64(bytes: ByteArray): String = Base64.getEncoder().encodeToString(bytes)

internal fun decodeBase64(value: String, field: String): ByteArray = try {
  Base64.getDecoder().decode(value)
} catch (_: IllegalArgumentException) {
  throw invalidArgument("$field is not valid base64")
}

/** Puts [value], or JSON null. `JSONObject.put` with a Kotlin null removes the key instead. */
private fun JSObject.putNullable(key: String, value: Any?): JSObject = put(key, value ?: JSONObject.NULL)

/** A scan result, in plain values so that the JSON builder runs without Android classes. */
internal class DeviceReport(
  val id: String,
  val name: String?,
  val rssi: Int?,
  val serviceUuids: List<String>,
  val localName: String?,
  val serviceData: List<Pair<String, ByteArray>>,
  val manufacturerData: List<Pair<Int, ByteArray>>,
  val connectable: Boolean?,
  val observedAtMillis: Long,
  val scanId: String,
)

internal class CharacteristicReport(val handle: String, val uuid: String, val properties: Properties)

internal class ServiceReport(val handle: String, val uuid: String, val characteristics: List<CharacteristicReport>)

internal class PermissionOutcomes(val scan: String, val connect: String, val advertise: String)

/** The facts that the capability report depends on. */
internal class CapabilityFacts(val hasAdapter: Boolean, val adapterOn: Boolean, val hasAdvertiser: Boolean)

/** The replies of `execute`: `{ kind }` or `{ kind, payload }`. */
internal object Replies {
  fun empty(): JSObject = JSObject().put("kind", "empty")

  private fun reply(kind: String, payload: Any): JSObject = JSObject().put("kind", kind).put("payload", payload)

  fun state(state: String) = reply("state", state)

  fun capabilities(facts: CapabilityFacts): JSObject {
    val central = if (facts.hasAdapter) "supported" to "available" else "unsupported" to "noAdapter"
    val peripheral = when {
      !facts.hasAdapter -> "unsupported" to "noAdapter"
      // The advertiser getter returns null while the adapter is off, so support is unknown then.
      !facts.adapterOn -> "unknown" to "bluetoothOff"
      facts.hasAdvertiser -> "supported" to "available"
      else -> "unsupported" to "noAdvertiser"
    }
    val payload = JSObject().put("central", support(central))
    for (key in listOf("peripheral", "advertising", "targetedNotify", "simultaneousRoles")) {
      payload.put(key, support(peripheral))
    }
    payload.put("background", support("unsupported" to "foregroundOnlyContract"))
    payload.putNullable("maxConnections", null)
    payload.putNullable("maxAdvertisingDataLength", if (facts.hasAdapter) LEGACY_ADVERTISING_DATA_LENGTH else null)
    return reply("capabilities", payload)
  }

  /** A `Support` from a level and a reason. */
  private fun support(levelAndReason: Pair<String, String>): JSObject = JSObject()
    .put("level", levelAndReason.first)
    .put("reason", levelAndReason.second)
    .putNullable("description", null)

  fun permissions(outcomes: PermissionOutcomes) = reply(
    "permissions",
    JSObject().put("scan", outcomes.scan).put("connect", outcomes.connect).put("advertise", outcomes.advertise),
  )

  fun scanStarted(scanId: String) = reply("scanStarted", JSObject().put("scanId", scanId))

  fun connected(connectionId: String, mtu: Int) =
    reply("connected", JSObject().put("connectionId", connectionId).put("limits", linkLimits(mtu)))

  fun linkLimits(mtu: Int): JSObject {
    val length = valueLength(mtu)
    return JSObject()
      .put("writeWithResponse", length)
      .put("writeWithoutResponse", length)
      .put("notification", length)
      .put("attMtu", mtu)
  }

  fun services(services: List<ServiceReport>): JSObject {
    val list = JSONArray()
    for (service in services) {
      val characteristics = JSONArray()
      for (characteristic in service.characteristics) {
        characteristics.put(
          JSObject()
            .put("handle", characteristic.handle)
            .put("uuid", characteristic.uuid)
            .put("properties", characteristic.properties.toJson()),
        )
      }
      list.put(
        JSObject().put("handle", service.handle).put("uuid", service.uuid).put("characteristics", characteristics),
      )
    }
    return reply("services", list)
  }

  fun bytes(value: ByteArray) = reply("bytes", JSObject().put("valueBase64", encodeBase64(value)))

  fun subscriptionStarted(subscriptionId: String) =
    reply("subscriptionStarted", JSObject().put("subscriptionId", subscriptionId))

  fun serverCreated(serverId: String) = reply("serverCreated", JSObject().put("serverId", serverId))

  fun advertisingStarted(localNameIncluded: Boolean, localNameTruncated: Boolean) = reply(
    "advertisingStarted",
    JSObject().put("localNameIncluded", localNameIncluded).put("localNameTruncated", localNameTruncated),
  )

  fun resources(scans: Int, connections: Int, subscriptions: Int, servers: Int) = reply(
    "resources",
    JSObject().put("scans", scans).put("connections", connections).put("subscriptions", subscriptions)
      .put("servers", servers),
  )
}

/** The events of the channel: `{ kind, payload }`, sent inside `{ ownerId, event }`. */
internal object Events {
  fun envelope(ownerId: String, event: JSObject): JSObject = JSObject().put("ownerId", ownerId).put("event", event)

  private fun event(kind: String, payload: JSObject): JSObject = JSObject().put("kind", kind).put("payload", payload)

  fun adapterStateChanged(state: String) = event("adapterStateChanged", JSObject().put("state", state))

  fun scanResult(device: DeviceReport): JSObject {
    val serviceData = JSONArray()
    for ((uuid, bytes) in device.serviceData) {
      serviceData.put(JSObject().put("serviceUuid", uuid).put("bytesBase64", encodeBase64(bytes)))
    }
    val manufacturerData = JSONArray()
    for ((companyId, bytes) in device.manufacturerData) {
      manufacturerData.put(JSObject().put("companyId", companyId).put("bytesBase64", encodeBase64(bytes)))
    }
    val advertisement = JSObject()
      .putNullable("localName", device.localName)
      .put("serviceData", serviceData)
      .put("manufacturerData", manufacturerData)
      .putNullable("connectable", device.connectable)
    val payload = JSObject()
      .put("id", device.id)
      .putNullable("name", device.name)
      .putNullable("rssi", device.rssi)
      .put("serviceUuids", JSONArray(device.serviceUuids))
      .put("advertisement", advertisement)
      .put("observedAtMillis", device.observedAtMillis)
      .put("scanId", device.scanId)
    return event("scanResult", JSObject().put("device", payload))
  }

  fun scanStopped(scanId: String) = event("scanStopped", JSObject().put("scanId", scanId))

  fun connectionClosed(connectionId: String) = event("connectionClosed", JSObject().put("connectionId", connectionId))

  fun characteristicValue(subscriptionId: String, value: ByteArray) = event(
    "characteristicValue",
    JSObject().put("subscriptionId", subscriptionId).put("valueBase64", encodeBase64(value)),
  )

  fun serverWrite(serverId: String, peerId: String, characteristicKey: String, value: ByteArray) = event(
    "serverWrite",
    JSObject()
      .put("serverId", serverId)
      .put("peerId", peerId)
      .put("characteristicKey", characteristicKey)
      .put("valueBase64", encodeBase64(value)),
  )

  fun subscriptionChanged(serverId: String, peerId: String, characteristicKey: String, maxValueLength: Int?) = event(
    "subscriptionChanged",
    JSObject()
      .put("serverId", serverId)
      .put("peerId", peerId)
      .put("characteristicKey", characteristicKey)
      .put("subscribed", maxValueLength != null)
      .putNullable("maxValueLength", maxValueLength),
  )

  fun criticalStateLoss(resourceId: String, reason: String) =
    event("criticalStateLoss", JSObject().put("resourceId", resourceId).put("reason", reason))
}

/** A legacy advertisement holds 31 bytes. */
internal const val LEGACY_ADVERTISING_DATA_LENGTH = 31
