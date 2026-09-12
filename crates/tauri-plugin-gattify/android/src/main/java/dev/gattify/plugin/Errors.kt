package dev.gattify.plugin

/** A rejection: a contract error code, or a platform code such as `gattStatus133`. */
internal class BleException(val code: String, override val message: String) : Exception(message)

internal object ErrorCode {
  const val PERMISSION_DENIED = "permissionDenied"
  const val BLUETOOTH_OFF = "bluetoothOff"
  const val UNAVAILABLE = "unavailable"
  const val UNSUPPORTED = "unsupported"
  const val INVALID_ARGUMENT = "invalidArgument"
  const val INVALID_HANDLE = "invalidHandle"
  const val BUSY = "busy"
  const val TIMEOUT = "timeout"
  const val DISCONNECTED = "disconnected"
  const val PAYLOAD_TOO_LARGE = "payloadTooLarge"
  const val CANCELLED = "cancelled"
  const val INTERNAL = "internal"
}

internal fun invalidArgument(message: String) = BleException(ErrorCode.INVALID_ARGUMENT, message)

internal fun invalidHandle(id: String) =
  BleException(ErrorCode.INVALID_HANDLE, "$id is unknown, stale, or owned by another owner")

internal fun busy(message: String) = BleException(ErrorCode.BUSY, message)

internal fun payloadTooLarge(message: String) = BleException(ErrorCode.PAYLOAD_TOO_LARGE, message)

internal fun unsupported(message: String) = BleException(ErrorCode.UNSUPPORTED, message)

internal fun internalError(message: String) = BleException(ErrorCode.INTERNAL, message)

internal fun disconnected(message: String = "the link closed during the procedure") =
  BleException(ErrorCode.DISCONNECTED, message)

internal fun cancelled(message: String) = BleException(ErrorCode.CANCELLED, message)

internal fun permissionDenied(message: String) = BleException(ErrorCode.PERMISSION_DENIED, message)

/** A GATT status other than success, as the platform code `gattStatus<n>`. */
internal fun gattStatusError(status: Int, procedure: String) =
  BleException("gattStatus$status", "$procedure failed with GATT status $status")

/** Maps an `AdvertiseCallback.onStartFailure` code. */
internal fun advertiseFailure(errorCode: Int): BleException = when (errorCode) {
  ADVERTISE_FAILED_DATA_TOO_LARGE ->
    payloadTooLarge("the advertisement is larger than the advertising data budget")
  ADVERTISE_FAILED_TOO_MANY_ADVERTISERS -> busy("the device has no free advertising instance")
  ADVERTISE_FAILED_ALREADY_STARTED -> busy("the advertisement already started")
  ADVERTISE_FAILED_FEATURE_UNSUPPORTED -> unsupported("this device cannot advertise")
  else -> internalError("advertising failed with error $errorCode")
}

/** Maps a `BluetoothStatusCodes` value that an API 33 GATT call returned instead of `SUCCESS`. */
internal fun statusCodeError(code: Int, procedure: String): BleException = when (code) {
  STATUS_BLUETOOTH_NOT_ENABLED -> BleException(ErrorCode.BLUETOOTH_OFF, "Bluetooth is off")
  STATUS_MISSING_CONNECT_PERMISSION -> permissionDenied("the connect permission is missing")
  STATUS_DEVICE_NOT_CONNECTED -> disconnected("the device is not connected")
  STATUS_GATT_WRITE_REQUEST_BUSY -> busy("the Bluetooth stack is busy with another request")
  else -> internalError("$procedure failed with Bluetooth status $code")
}

// AdvertiseCallback.ADVERTISE_FAILED_* values.
private const val ADVERTISE_FAILED_DATA_TOO_LARGE = 1
private const val ADVERTISE_FAILED_TOO_MANY_ADVERTISERS = 2
private const val ADVERTISE_FAILED_ALREADY_STARTED = 3
private const val ADVERTISE_FAILED_FEATURE_UNSUPPORTED = 5

// BluetoothStatusCodes values. ERROR_DEVICE_NOT_CONNECTED is documented but not in the public SDK.
internal const val STATUS_SUCCESS = 0
private const val STATUS_BLUETOOTH_NOT_ENABLED = 1
private const val STATUS_DEVICE_NOT_CONNECTED = 4
private const val STATUS_MISSING_CONNECT_PERMISSION = 6
private const val STATUS_GATT_WRITE_REQUEST_BUSY = 201
