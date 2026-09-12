package dev.gattify.plugin

import java.nio.ByteBuffer
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

/** The scan response budget for the name: 31 bytes, minus 2 for the header and 16 for the UUID. */
internal const val ADVERTISED_NAME_BUDGET = 13

/** A local name cut to a byte budget at a UTF-8 character boundary. */
internal class CutName(val bytes: ByteArray, val truncated: Boolean)

internal fun cutUtf8(name: String, maxBytes: Int): CutName {
  val bytes = name.toByteArray(Charsets.UTF_8)
  if (bytes.size <= maxBytes) return CutName(bytes, false)
  var end = maxBytes
  // A continuation byte is 10xxxxxx: step back to the first byte of its character.
  while (end > 0 && (bytes[end].toInt() and 0xC0) == 0x80) end--
  return CutName(bytes.copyOf(end), true)
}

/** Decodes [bytes] as UTF-8, or returns null when they are not valid UTF-8. */
internal fun decodeUtf8(bytes: ByteArray): String? = try {
  Charsets.UTF_8.newDecoder()
    .onMalformedInput(CodingErrorAction.REPORT)
    .onUnmappableCharacter(CodingErrorAction.REPORT)
    .decode(ByteBuffer.wrap(bytes))
    .toString()
} catch (_: CharacterCodingException) {
  null
}

/**
 * The display name of a scan result: the advertised local name, else the service
 * data of a filter UUID when it is valid UTF-8 (how an Android host advertises its
 * name), else the cached device name.
 */
internal fun scanName(
  localName: String?,
  serviceData: List<Pair<String, ByteArray>>,
  filter: Set<String>,
  cachedName: String?,
): String? {
  if (!localName.isNullOrEmpty()) return localName
  for ((uuid, bytes) in serviceData) {
    if (uuid !in filter || bytes.isEmpty()) continue
    decodeUtf8(bytes)?.let { return it }
  }
  return cachedName?.takeIf { it.isNotEmpty() }
}
