import Foundation

/// Allocates `<prefix>-<n>` IDs. Each prefix counts from 1 and never reuses a number.
struct IDAllocator {
  private var counters: [String: Int] = [:]

  mutating func next(_ prefix: String) -> String {
    let value = counters[prefix, default: 0] + 1
    counters[prefix] = value
    return "\(prefix)-\(value)"
  }
}

/// The text after the first `:` of an owner ID. An owner ID without `:` is its own family.
func ownerFamily(_ ownerId: String) -> String {
  guard let colon = ownerId.firstIndex(of: ":") else { return ownerId }
  return String(ownerId[ownerId.index(after: colon)...])
}

/// Keys that identify each attribute by its UUID and its position among attributes with the
/// same UUID. CoreBluetooth hides ATT handles, and these keys stay the same across discoveries.
func occurrenceKeys(_ uuids: [String]) -> [String] {
  var seen: [String: Int] = [:]
  return uuids.map { uuid in
    let index = seen[uuid, default: 0]
    seen[uuid] = index + 1
    return "\(uuid)#\(index)"
  }
}

/// Clamps a relative deadline for `DispatchTime` arithmetic.
func dispatchInterval(milliseconds: UInt64) -> DispatchTimeInterval {
  .milliseconds(Int(min(milliseconds, UInt64(Int32.max))))
}
