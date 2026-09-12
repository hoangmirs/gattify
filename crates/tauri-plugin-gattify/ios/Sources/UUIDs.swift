import CoreBluetooth

private let bluetoothBaseSuffix = "-0000-1000-8000-00805f9b34fb"

/// The contract form of a UUID: lowercase, 128-bit, with hyphens. A 16-bit or 32-bit UUID
/// is expanded with the Bluetooth base UUID. Returns nil for anything else.
func canonicalUUID(_ value: String) -> String? {
  let hex = value.replacingOccurrences(of: "-", with: "").lowercased()
  guard hex.allSatisfy({ $0.isASCII && $0.isHexDigit }) else { return nil }
  switch hex.count {
  case 4:
    return "0000\(hex)\(bluetoothBaseSuffix)"
  case 8:
    return "\(hex)\(bluetoothBaseSuffix)"
  case 32:
    let characters = Array(hex)
    let groups = [0..<8, 8..<12, 12..<16, 16..<20, 20..<32].map { String(characters[$0]) }
    return groups.joined(separator: "-")
  default:
    return nil
  }
}

/// CoreBluetooth shortens a Bluetooth base UUID to `180D` or `0000180D` in `uuidString`.
func canonicalUUID(_ uuid: CBUUID) -> String {
  canonicalUUID(uuid.uuidString) ?? uuid.uuidString.lowercased()
}

/// `CBUUID(string:)` raises an Objective-C exception for a malformed string, so validate first.
func makeCBUUID(_ value: String) -> CBUUID? {
  guard let canonical = canonicalUUID(value) else { return nil }
  return CBUUID(string: canonical.uppercased())
}
