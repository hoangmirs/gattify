package dev.gattify.plugin

import java.util.UUID

/** The client characteristic configuration descriptor. */
internal val CCCD_UUID: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")

private const val BASE_UUID_SUFFIX = "00001000800000805f9b34fb"

/**
 * Parses a 16-bit, 32-bit or 128-bit UUID, with or without hyphens. A short UUID
 * expands with the Bluetooth base UUID.
 */
internal fun parseUuid(value: String): UUID? {
  val hex = value.replace("-", "")
  if (!hex.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }) return null
  val full = when (hex.length) {
    4 -> "0000$hex$BASE_UUID_SUFFIX"
    8 -> "$hex$BASE_UUID_SUFFIX"
    32 -> hex
    else -> return null
  }
  return UUID(
    java.lang.Long.parseUnsignedLong(full.substring(0, 16), 16),
    java.lang.Long.parseUnsignedLong(full.substring(16), 16),
  )
}

/** The wire form of a UUID: lowercase, 128-bit, with hyphens. */
internal fun uuidString(uuid: UUID): String = uuid.toString().lowercase()

/** Whether an advertisement matches a scan filter. An empty filter matches every device. */
internal fun matchesFilter(filter: Set<String>, serviceUuids: List<String>, serviceDataUuids: List<String>): Boolean =
  filter.isEmpty() || serviceUuids.any { it in filter } || serviceDataUuids.any { it in filter }
