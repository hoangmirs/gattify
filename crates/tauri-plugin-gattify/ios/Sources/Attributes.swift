import CoreBluetooth

extension CharacteristicProperties {
  init(_ properties: CBCharacteristicProperties) {
    read = properties.contains(.read)
    write = properties.contains(.write)
    writeWithoutResponse = properties.contains(.writeWithoutResponse)
    notify = properties.contains(.notify) || properties.contains(.notifyEncryptionRequired)
    indicate = properties.contains(.indicate) || properties.contains(.indicateEncryptionRequired)
  }

  var writable: Bool { write || writeWithoutResponse }

  var notifiable: Bool { notify || indicate }

  var cbProperties: CBCharacteristicProperties {
    var properties: CBCharacteristicProperties = []
    if read { properties.insert(.read) }
    if write { properties.insert(.write) }
    if writeWithoutResponse { properties.insert(.writeWithoutResponse) }
    if notify { properties.insert(.notify) }
    if indicate { properties.insert(.indicate) }
    return properties
  }

  var cbPermissions: CBAttributePermissions {
    var permissions: CBAttributePermissions = []
    if read { permissions.insert(.readable) }
    if writable { permissions.insert(.writeable) }
    return permissions
  }
}

/// The part of a stored value that a read at `offset` returns, or nil when the offset is past the end.
func readSlice(_ value: Data, offset: Int) -> Data? {
  guard offset >= 0, offset <= value.count else { return nil }
  return Data([UInt8](value).dropFirst(offset))
}

/// One request of a `didReceiveWrite` batch, reduced to what validation needs.
struct WritePart {
  /// Identifies the characteristic and the central, or nil when no server owns the characteristic.
  let target: Int?
  let writable: Bool
  let maxValueLength: Int
  let offset: Int
  let value: Data
}

struct AssembledWrite: Equatable {
  let target: Int
  var value: Data
}

enum WriteBatchOutcome: Equatable {
  case accepted([AssembledWrite])
  case rejected(CBATTError.Code)
}

/// Validates a whole batch before anything is emitted. The parts of each target start at offset 0
/// and are contiguous. The assembled values keep the order of each target's first part.
func assembleWrites(_ parts: [WritePart]) -> WriteBatchOutcome {
  var writes: [AssembledWrite] = []
  var slots: [Int: Int] = [:]
  for part in parts {
    guard let target = part.target else { return .rejected(.attributeNotFound) }
    guard part.writable else { return .rejected(.writeNotPermitted) }
    let slot: Int
    if let existing = slots[target] {
      slot = existing
    } else {
      slot = writes.count
      slots[target] = slot
      writes.append(AssembledWrite(target: target, value: Data()))
    }
    guard part.offset == writes[slot].value.count else { return .rejected(.invalidOffset) }
    guard writes[slot].value.count + part.value.count <= part.maxValueLength else {
      return .rejected(.invalidAttributeValueLength)
    }
    writes[slot].value.append(part.value)
  }
  return .accepted(writes)
}
