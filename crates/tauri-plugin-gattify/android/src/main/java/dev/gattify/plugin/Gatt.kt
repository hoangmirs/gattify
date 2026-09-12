package dev.gattify.plugin

import app.tauri.plugin.JSObject

/** The ATT statuses a local server answers with. */
internal object Att {
  const val SUCCESS = 0
  const val READ_NOT_PERMITTED = 0x02
  const val WRITE_NOT_PERMITTED = 0x03
  const val REQUEST_NOT_SUPPORTED = 0x06
  const val INVALID_OFFSET = 0x07
  const val INVALID_ATTRIBUTE_LENGTH = 0x0D

  /** The common profile error for a CCCD value that the characteristic cannot take. */
  const val CCCD_IMPROPERLY_CONFIGURED = 0xFD
}

internal const val DEFAULT_ATT_MTU = 23
internal const val REQUESTED_ATT_MTU = 517

/** The longest attribute value that ATT allows. */
internal const val MAX_ATTRIBUTE_LENGTH = 512

/** The largest value that one ATT write or notification carries at [mtu]: the MTU minus 3, at most 512. */
internal fun valueLength(mtu: Int): Int = (mtu - 3).coerceIn(DEFAULT_ATT_MTU - 3, MAX_ATTRIBUTE_LENGTH)

// BluetoothGattCharacteristic.PROPERTY_* bits.
private const val PROPERTY_READ = 0x02
private const val PROPERTY_WRITE_NO_RESPONSE = 0x04
private const val PROPERTY_WRITE = 0x08
private const val PROPERTY_NOTIFY = 0x10
private const val PROPERTY_INDICATE = 0x20

// BluetoothGattCharacteristic.PERMISSION_* and BluetoothGattDescriptor.PERMISSION_* bits.
internal const val PERMISSION_READ = 0x01
internal const val PERMISSION_WRITE = 0x10

/** The characteristic properties of the wire contract. */
internal data class Properties(
  val read: Boolean,
  val write: Boolean,
  val writeWithoutResponse: Boolean,
  val notify: Boolean,
  val indicate: Boolean,
) {
  val writable: Boolean
    get() = write || writeWithoutResponse

  val notifies: Boolean
    get() = notify || indicate

  val bits: Int
    get() = (if (read) PROPERTY_READ else 0) or
      (if (write) PROPERTY_WRITE else 0) or
      (if (writeWithoutResponse) PROPERTY_WRITE_NO_RESPONSE else 0) or
      (if (notify) PROPERTY_NOTIFY else 0) or
      (if (indicate) PROPERTY_INDICATE else 0)

  /** The attribute permissions of a local characteristic with these properties. */
  val permissions: Int
    get() = (if (read) PERMISSION_READ else 0) or (if (writable) PERMISSION_WRITE else 0)

  fun toJson(): JSObject = JSObject()
    .put("read", read)
    .put("write", write)
    .put("writeWithoutResponse", writeWithoutResponse)
    .put("notify", notify)
    .put("indicate", indicate)

  companion object {
    fun fromBits(bits: Int) = Properties(
      read = bits and PROPERTY_READ != 0,
      write = bits and PROPERTY_WRITE != 0,
      writeWithoutResponse = bits and PROPERTY_WRITE_NO_RESPONSE != 0,
      notify = bits and PROPERTY_NOTIFY != 0,
      indicate = bits and PROPERTY_INDICATE != 0,
    )
  }
}

// Client characteristic configuration bits.
internal const val CCCD_NOTIFY = 0x01
internal const val CCCD_INDICATE = 0x02

/** The unsigned 16-bit little-endian value of a CCCD write, or null when it is not two bytes long. */
internal fun cccdValueOf(value: ByteArray): Int? =
  if (value.size == 2) (value[0].toInt() and 0xFF) or ((value[1].toInt() and 0xFF) shl 8) else null

/**
 * Checks a CCCD write: two bytes, no reserved bit, and only modes that the
 * characteristic has. A failed write keeps the previous configuration.
 */
internal fun cccdWriteStatus(properties: Properties, value: ByteArray): Int {
  val bits = cccdValueOf(value) ?: return Att.INVALID_ATTRIBUTE_LENGTH
  return when {
    bits and (CCCD_NOTIFY or CCCD_INDICATE).inv() != 0 -> Att.CCCD_IMPROPERLY_CONFIGURED
    bits and CCCD_NOTIFY != 0 && !properties.notify -> Att.CCCD_IMPROPERLY_CONFIGURED
    bits and CCCD_INDICATE != 0 && !properties.indicate -> Att.CCCD_IMPROPERLY_CONFIGURED
    else -> Att.SUCCESS
  }
}

internal fun cccdValue(bits: Int): ByteArray = byteArrayOf(bits.toByte(), (bits shr 8).toByte())

/** A notify is an indication when the central enabled indications and not notifications. */
internal fun confirms(bits: Int): Boolean = bits and CCCD_NOTIFY == 0 && bits and CCCD_INDICATE != 0

/** Answers a read of [value] at [offset]: the ATT status, and the bytes from the offset on. */
internal fun readAnswer(readable: Boolean, value: ByteArray, offset: Int): Pair<Int, ByteArray?> = when {
  !readable -> Att.READ_NOT_PERMITTED to null
  offset < 0 || offset > value.size -> Att.INVALID_OFFSET to null
  else -> Att.SUCCESS to value.copyOfRange(offset, value.size)
}

/**
 * Checks one part of a write. The characteristic must be writable, the part must
 * start where the assembled value ends, and the value must fit [maxValueLength].
 */
internal fun writeStatus(properties: Properties, maxValueLength: Int, assembled: Int, offset: Int, size: Int): Int =
  when {
    !properties.writable -> Att.WRITE_NOT_PERMITTED
    offset != assembled -> Att.INVALID_OFFSET
    assembled + size > maxValueLength -> Att.INVALID_ATTRIBUTE_LENGTH
    else -> Att.SUCCESS
  }
