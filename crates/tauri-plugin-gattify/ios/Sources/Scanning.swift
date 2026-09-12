import CoreBluetooth

/// What CoreBluetooth reports for one advertisement, in contract form.
struct Advertisement: Equatable {
  let data: AdvertisementData
  /// The advertised service UUIDs, including the overflow area.
  let serviceUuids: [String]
  /// Every UUID a scan filter can match: services, solicited services and service data.
  let matchUuids: Set<String>

  init(_ advertisementData: [String: Any]) {
    func uuids(_ key: String) -> [String] {
      (advertisementData[key] as? [CBUUID] ?? []).map(canonicalUUID)
    }
    var services: [String] = []
    for uuid in uuids(CBAdvertisementDataServiceUUIDsKey) + uuids(CBAdvertisementDataOverflowServiceUUIDsKey)
    where !services.contains(uuid) {
      services.append(uuid)
    }
    let serviceData = (advertisementData[CBAdvertisementDataServiceDataKey] as? [CBUUID: Data] ?? [:])
      .map { ServiceDataEntry(uuid: canonicalUUID($0.key), bytes: $0.value) }
      .sorted { $0.uuid < $1.uuid }
    let manufacturer = (advertisementData[CBAdvertisementDataManufacturerDataKey] as? Data)
      .flatMap(ManufacturerDataEntry.init)
    data = AdvertisementData(
      localName: advertisementData[CBAdvertisementDataLocalNameKey] as? String,
      serviceData: serviceData,
      manufacturerData: manufacturer.map { [$0] } ?? [],
      connectable: (advertisementData[CBAdvertisementDataIsConnectable] as? NSNumber)?.boolValue)
    serviceUuids = services
    matchUuids = Set(services + uuids(CBAdvertisementDataSolicitedServiceUUIDsKey) + serviceData.map(\.uuid))
  }
}

/// The advertised local name; else the UTF-8 service data of a filter UUID, which is how an
/// Android host advertises its name; else the cached device name.
func deviceName(_ advertisement: AdvertisementData, filter: Set<String>, cachedName: String?) -> String? {
  if let name = advertisement.localName, !name.isEmpty { return name }
  for entry in advertisement.serviceData where filter.contains(entry.uuid) && !entry.bytes.isEmpty {
    if let name = String(data: entry.bytes, encoding: .utf8) { return name }
  }
  return cachedName
}

/// CoreBluetooth reports 127 when it has no RSSI.
func reportedRSSI(_ value: NSNumber) -> Int? {
  let rssi = value.intValue
  guard rssi != 127, rssi >= Int(Int16.min), rssi <= Int(Int16.max) else { return nil }
  return rssi
}

/// An empty filter matches every advertisement.
func scanFilterMatches(_ filter: Set<String>, _ advertised: Set<String>) -> Bool {
  filter.isEmpty || !filter.isDisjoint(with: advertised)
}

/// iOS runs one scan: the union of the filters of every active scan.
enum ScanPlan: Equatable {
  case stopped
  case everyDevice
  case services([String])

  init(filters: [Set<String>]) {
    if filters.isEmpty {
      self = .stopped
    } else if filters.contains(where: \.isEmpty) {
      self = .everyDevice
    } else {
      self = .services(filters.reduce(into: Set<String>()) { $0.formUnion($1) }.sorted())
    }
  }
}

/// Lets through at most one result per device every `interval` seconds.
struct ResultThrottle {
  var interval: TimeInterval = 1
  private var lastEmitted: [String: TimeInterval] = [:]

  mutating func allow(_ deviceId: String, at now: TimeInterval) -> Bool {
    if let previous = lastEmitted[deviceId], now - previous < interval { return false }
    lastEmitted[deviceId] = now
    return true
  }
}
