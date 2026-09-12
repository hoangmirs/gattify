import CoreBluetooth
import Foundation

/// A rejection: an error code from the bridge contract, or a platform code.
struct BridgeError: Error, Equatable {
  let code: String
  let message: String

  static func invalidArgument(_ message: String) -> BridgeError {
    BridgeError(code: "invalidArgument", message: message)
  }

  static func invalidHandle(_ id: String) -> BridgeError {
    BridgeError(code: "invalidHandle", message: "\(id) is unknown, stale, or owned by another owner")
  }

  static func busy(_ message: String) -> BridgeError {
    BridgeError(code: "busy", message: message)
  }

  static func unsupported(_ message: String) -> BridgeError {
    BridgeError(code: "unsupported", message: message)
  }

  static func payloadTooLarge(_ message: String) -> BridgeError {
    BridgeError(code: "payloadTooLarge", message: message)
  }

  static func disconnected(_ message: String) -> BridgeError {
    BridgeError(code: "disconnected", message: message)
  }

  static let timeout = BridgeError(code: "timeout", message: "the deadline passed")

  static let cancelled = BridgeError(code: "cancelled", message: "the operation was cancelled")

  /// The rejection for a command that needs a powered-on adapter, or nil when the adapter is on.
  static func adapter(_ state: CBManagerState) -> BridgeError? {
    switch state {
    case .poweredOn:
      return nil
    case .poweredOff:
      return BridgeError(code: "bluetoothOff", message: "Bluetooth is off")
    case .unauthorized:
      return BridgeError(code: "permissionDenied", message: "the app may not use Bluetooth")
    case .unsupported:
      return BridgeError(code: "unavailable", message: "this device has no Bluetooth LE adapter")
    case .resetting, .unknown:
      return BridgeError(code: "unavailable", message: "the Bluetooth adapter is not ready")
    @unknown default:
      return BridgeError(code: "unavailable", message: "the Bluetooth adapter is in an unknown state")
    }
  }
}

extension BridgeError {
  init(_ error: Error) {
    let error = error as NSError
    self.init(
      code: nativeErrorCode(domain: error.domain, code: error.code),
      message: error.localizedDescription)
  }
}

/// Maps a CoreBluetooth error to a contract code when one fits, else to `cbError<n>` or `cbAttError<n>`.
func nativeErrorCode(domain: String, code: Int) -> String {
  switch domain {
  case CBErrorDomain:
    switch CBError.Code(rawValue: code) {
    case .notConnected?, .peripheralDisconnected?, .connectionTimeout?:
      return "disconnected"
    case .alreadyAdvertising?, .connectionLimitReached?:
      return "busy"
    case .operationNotSupported?:
      return "unsupported"
    default:
      return "cbError\(code)"
    }
  case CBATTErrorDomain:
    if CBATTError.Code(rawValue: code) == .invalidAttributeValueLength {
      return "payloadTooLarge"
    }
    return "cbAttError\(code)"
  default:
    return "internal"
  }
}
