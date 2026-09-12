/// The local name bytes iOS fits next to a 128-bit service UUID: 28 foreground bytes, minus 18
/// for the UUID field, minus 2 for the name field header.
let localNameBudget = 8

struct LocalNameCut: Equatable {
  /// The name to advertise, or nil when no byte of it fits.
  let name: String?
  let truncated: Bool

  var included: Bool { name != nil }
}

/// Cuts a name to `budget` UTF-8 bytes without splitting a character's byte sequence.
func cutLocalName(_ name: String?, budget: Int = localNameBudget) -> LocalNameCut {
  guard let name, !name.isEmpty else { return LocalNameCut(name: nil, truncated: false) }
  var kept = String.UnicodeScalarView()
  var used = 0
  for scalar in name.unicodeScalars {
    let size = utf8Length(scalar)
    guard used + size <= budget else { break }
    used += size
    kept.append(scalar)
  }
  let truncated = kept.count < name.unicodeScalars.count
  return LocalNameCut(name: kept.isEmpty ? nil : String(kept), truncated: truncated)
}

private func utf8Length(_ scalar: Unicode.Scalar) -> Int {
  switch scalar.value {
  case 0..<0x80: return 1
  case 0x80..<0x800: return 2
  case 0x800..<0x10000: return 3
  default: return 4
  }
}
