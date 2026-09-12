import Foundation

/// A JSON value with explicit nulls. It always encodes to valid JSON.
enum JSON: Equatable {
  case null
  case bool(Bool)
  case int(Int)
  case string(String)
  case array([JSON])
  case object([String: JSON])

  init(_ value: String?) {
    self = value.map(JSON.string) ?? .null
  }

  init(_ value: Int?) {
    self = value.map(JSON.int) ?? .null
  }

  init(_ value: Bool?) {
    self = value.map(JSON.bool) ?? .null
  }

  /// The compact encoding with sorted keys.
  var text: String {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    guard let data = try? encoder.encode(self) else { return "null" }
    return String(decoding: data, as: UTF8.self)
  }
}

extension JSON: Encodable {
  func encode(to encoder: Encoder) throws {
    var container = encoder.singleValueContainer()
    switch self {
    case .null:
      try container.encodeNil()
    case .bool(let value):
      try container.encode(value)
    case .int(let value):
      try container.encode(value)
    case .string(let value):
      try container.encode(value)
    case .array(let values):
      try container.encode(values)
    case .object(let fields):
      try container.encode(fields)
    }
  }
}

extension JSON: ExpressibleByNilLiteral {
  init(nilLiteral: ()) {
    self = .null
  }
}

extension JSON: ExpressibleByBooleanLiteral {
  init(booleanLiteral value: Bool) {
    self = .bool(value)
  }
}

extension JSON: ExpressibleByIntegerLiteral {
  init(integerLiteral value: Int) {
    self = .int(value)
  }
}

extension JSON: ExpressibleByStringLiteral {
  init(stringLiteral value: String) {
    self = .string(value)
  }
}

extension JSON: ExpressibleByArrayLiteral {
  init(arrayLiteral elements: JSON...) {
    self = .array(elements)
  }
}

extension JSON: ExpressibleByDictionaryLiteral {
  init(dictionaryLiteral elements: (String, JSON)...) {
    self = .object(Dictionary(elements, uniquingKeysWith: { _, last in last }))
  }
}
