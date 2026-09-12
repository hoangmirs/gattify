package dev.gattify.plugin

import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

/** Every command kind of the bridge contract. */
internal val COMMAND_KINDS = setOf(
  "getState", "getCapabilities", "checkPermissions", "requestPermissions",
  "startScan", "stopScan",
  "connect", "disconnect", "discoverServices", "read", "write", "subscribe", "unsubscribe",
  "createServer", "closeServer", "startAdvertising", "stopAdvertising", "setValue", "notify",
  "cancel", "closeOwner", "debugResources",
)

/** One `execute` call: `{ operationId, ownerId, deadlineMillis, command }`. */
internal class Request(
  val operationId: String,
  val ownerId: String,
  val deadlineMillis: Long?,
  val kind: String?,
  private val payload: JSONObject?,
) {
  fun payload(): JSONObject = payload ?: throw invalidArgument("$kind needs a payload")
}

internal fun parseRequest(args: JSONObject): Request {
  val command = args.opt("command") as? JSONObject
  return Request(
    operationId = args.requireString("operationId"),
    ownerId = args.requireString("ownerId"),
    deadlineMillis = args.optionalLong("deadlineMillis")?.coerceAtLeast(0),
    kind = command?.optionalString("kind"),
    payload = command?.opt("payload") as? JSONObject,
  )
}

internal fun JSONObject.optionalString(key: String): String? = when (val value = opt(key)) {
  null, JSONObject.NULL -> null
  is String -> value
  else -> throw invalidArgument("$key must be a string")
}

internal fun JSONObject.requireString(key: String): String = optionalString(key) ?: throw invalidArgument("$key is missing")

internal fun JSONObject.optionalLong(key: String): Long? = when (val value = opt(key)) {
  null, JSONObject.NULL -> null
  is Number -> value.toLong()
  else -> throw invalidArgument("$key must be a number")
}

internal fun JSONObject.requireBoolean(key: String): Boolean =
  opt(key) as? Boolean ?: throw invalidArgument("$key must be a boolean")

internal fun JSONObject.requireObject(key: String): JSONObject =
  opt(key) as? JSONObject ?: throw invalidArgument("$key must be an object")

internal fun JSONObject.requireArray(key: String): JSONArray =
  opt(key) as? JSONArray ?: throw invalidArgument("$key must be an array")

internal fun JSONObject.requireUuid(key: String): UUID =
  parseUuid(requireString(key)) ?: throw invalidArgument("$key is not a UUID")

private fun JSONArray.objects(field: String): List<JSONObject> =
  (0 until length()).map { index -> opt(index) as? JSONObject ?: throw invalidArgument("$field[$index] must be an object") }

internal class ScanSpec(val serviceUuids: List<UUID>, val timeoutMs: Long?)

internal fun decodeScan(payload: JSONObject): ScanSpec {
  val uuids = payload.requireArray("serviceUuids")
  val serviceUuids = (0 until uuids.length()).map { index ->
    (uuids.opt(index) as? String)?.let(::parseUuid) ?: throw invalidArgument("serviceUuids[$index] is not a UUID")
  }
  return ScanSpec(serviceUuids.distinct(), payload.optionalLong("timeoutMs")?.coerceAtLeast(0))
}

internal class PermissionAsk(val scan: Boolean, val connect: Boolean, val advertise: Boolean)

internal fun decodePermissionAsk(payload: JSONObject) = PermissionAsk(
  scan = payload.requireBoolean("scan"),
  connect = payload.requireBoolean("connect"),
  advertise = payload.requireBoolean("advertise"),
)

internal fun decodeProperties(properties: JSONObject) = Properties(
  read = properties.requireBoolean("read"),
  write = properties.requireBoolean("write"),
  writeWithoutResponse = properties.requireBoolean("writeWithoutResponse"),
  notify = properties.requireBoolean("notify"),
  indicate = properties.requireBoolean("indicate"),
)

/** A local characteristic. [key] is `<serviceInstanceKey>/<characteristicInstanceKey>`. */
internal class CharacteristicSpec(
  val key: String,
  val uuid: UUID,
  val properties: Properties,
  val initialValue: ByteArray,
  val maxValueLength: Int,
)

internal class ServiceSpec(val uuid: UUID, val primary: Boolean, val characteristics: List<CharacteristicSpec>)

internal fun decodeServer(definition: JSONObject): List<ServiceSpec> {
  val services = definition.requireArray("services").objects("services")
  if (services.isEmpty()) throw invalidArgument("a server needs at least one service")
  val keys = HashSet<String>()
  return services.map { service ->
    val serviceKey = service.requireString("instanceKey")
    val characteristics = service.requireArray("characteristics").objects("characteristics").map { characteristic ->
      val key = "$serviceKey/${characteristic.requireString("instanceKey")}"
      if (!keys.add(key)) throw invalidArgument("the characteristic key $key repeats")
      val maxValueLength = characteristic.optionalLong("maxValueLength")
        ?.takeIf { it in 1L..Int.MAX_VALUE.toLong() }
        ?.toInt()
        ?: throw invalidArgument("maxValueLength of $key must be a positive number")
      val initialValue = characteristic.optionalString("initialValueBase64")
        ?.let { decodeBase64(it, "initialValueBase64 of $key") }
        ?: ByteArray(0)
      if (initialValue.size > maxValueLength) throw invalidArgument("the initial value of $key is longer than maxValueLength")
      CharacteristicSpec(
        key = key,
        uuid = characteristic.requireUuid("uuid"),
        properties = decodeProperties(characteristic.requireObject("properties")),
        initialValue = initialValue,
        maxValueLength = maxValueLength,
      )
    }
    ServiceSpec(service.requireUuid("uuid"), service.requireBoolean("primary"), characteristics)
  }
}

internal class AdvertisingSpec(val serviceUuid: UUID, val localName: String?, val localNameOptional: Boolean)

internal fun decodeAdvertising(options: JSONObject) = AdvertisingSpec(
  serviceUuid = options.requireUuid("serviceUuid"),
  localName = options.optionalString("localName"),
  localNameOptional = options.requireBoolean("localNameOptional"),
)

/** Whether a `writeType` asks for a write response. */
internal fun decodeWriteType(writeType: String): Boolean = when (writeType) {
  "withResponse" -> true
  "withoutResponse" -> false
  else -> throw invalidArgument("unknown writeType $writeType")
}
