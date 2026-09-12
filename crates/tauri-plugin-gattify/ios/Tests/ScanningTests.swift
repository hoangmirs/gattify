import CoreBluetooth
import XCTest

@testable import tauri_plugin_gattify

final class ScanningTests: XCTestCase {
  private let lab = "80ff87c3-8e84-4914-aedc-0d6a3ba5534d"
  private let heartRate = "0000180d-0000-1000-8000-00805f9b34fb"
  private let battery = "0000180f-0000-1000-8000-00805f9b34fb"
  private let solicited = "00001812-0000-1000-8000-00805f9b34fb"

  func testParsesAnAdvertisement() {
    let advertisement = Advertisement([
      CBAdvertisementDataLocalNameKey: "Host A",
      CBAdvertisementDataServiceUUIDsKey: [CBUUID(string: lab.uppercased()), CBUUID(string: "180D")],
      CBAdvertisementDataOverflowServiceUUIDsKey: [CBUUID(string: "180D"), CBUUID(string: "180F")],
      CBAdvertisementDataSolicitedServiceUUIDsKey: [CBUUID(string: "1812")],
      CBAdvertisementDataServiceDataKey: [CBUUID(string: "180F"): Data([0x64])],
      CBAdvertisementDataManufacturerDataKey: Data([0x4C, 0x00, 0x01, 0x02]),
      CBAdvertisementDataIsConnectable: NSNumber(value: true),
    ])
    XCTAssertEqual(advertisement.serviceUuids, [lab, heartRate, battery])
    XCTAssertEqual(advertisement.matchUuids, [lab, heartRate, battery, solicited])
    XCTAssertEqual(
      advertisement.data,
      AdvertisementData(
        localName: "Host A",
        serviceData: [ServiceDataEntry(uuid: battery, bytes: Data([0x64]))],
        manufacturerData: [ManufacturerDataEntry(companyId: 76, bytes: Data([1, 2]))],
        connectable: true))
  }

  func testAnEmptyAdvertisementHasNoFields() {
    let advertisement = Advertisement([:])
    XCTAssertEqual(advertisement.serviceUuids, [])
    XCTAssertEqual(advertisement.matchUuids, [])
    XCTAssertEqual(advertisement.data, AdvertisementData())
  }

  func testManufacturerDataNeedsACompanyId() {
    XCTAssertNil(ManufacturerDataEntry(Data([0x4C])))
    XCTAssertEqual(ManufacturerDataEntry(Data([0x4C, 0x00])), ManufacturerDataEntry(companyId: 76, bytes: Data()))
    XCTAssertEqual(
      ManufacturerDataEntry(Data([0x34, 0x12, 0xAA]))?.companyId, 0x1234, "the company ID is little-endian")
  }

  func testTheAdvertisedNameComesFirst() {
    let data = AdvertisementData(
      localName: "Host A", serviceData: [ServiceDataEntry(uuid: lab, bytes: Data("Other".utf8))])
    XCTAssertEqual(deviceName(data, filter: [lab], cachedName: "Cached"), "Host A")
  }

  func testAnAndroidHostNameComesFromTheServiceDataOfAFilterUUID() {
    let data = AdvertisementData(serviceData: [ServiceDataEntry(uuid: lab, bytes: Data("Pixel".utf8))])
    XCTAssertEqual(deviceName(data, filter: [lab], cachedName: "Cached"), "Pixel")
    XCTAssertEqual(deviceName(data, filter: [heartRate], cachedName: "Cached"), "Cached")
    XCTAssertEqual(deviceName(data, filter: [], cachedName: nil), nil)
  }

  func testServiceDataThatIsNotUTF8IsNotAName() {
    let data = AdvertisementData(
      localName: "", serviceData: [ServiceDataEntry(uuid: lab, bytes: Data([0xFF, 0xFE]))])
    XCTAssertEqual(deviceName(data, filter: [lab], cachedName: "Cached"), "Cached")
    let empty = AdvertisementData(serviceData: [ServiceDataEntry(uuid: lab, bytes: Data())])
    XCTAssertNil(deviceName(empty, filter: [lab], cachedName: nil))
  }

  func testScanFiltersMatchAnyAdvertisedUUID() {
    XCTAssertTrue(scanFilterMatches([], []))
    XCTAssertTrue(scanFilterMatches([], [lab]))
    XCTAssertTrue(scanFilterMatches([lab, heartRate], [heartRate]))
    XCTAssertFalse(scanFilterMatches([lab], [heartRate]))
    XCTAssertFalse(scanFilterMatches([lab], []))
  }

  func testOneScanRunsWithTheUnionOfTheFilters() {
    XCTAssertEqual(ScanPlan(filters: []), .stopped)
    XCTAssertEqual(ScanPlan(filters: [[lab], []]), .everyDevice)
    XCTAssertEqual(ScanPlan(filters: [[lab], [heartRate, lab]]), .services([heartRate, lab]))
  }

  func testThrottlesResultsPerDevice() {
    var throttle = ResultThrottle()
    XCTAssertTrue(throttle.allow("device-1", at: 10))
    XCTAssertFalse(throttle.allow("device-1", at: 10.5))
    XCTAssertTrue(throttle.allow("device-2", at: 10.5))
    XCTAssertTrue(throttle.allow("device-1", at: 11))
    XCTAssertFalse(throttle.allow("device-1", at: 11.999))
  }

  func testReportsNoRSSIForTheUnavailableMarker() {
    XCTAssertNil(reportedRSSI(127))
    XCTAssertEqual(reportedRSSI(-58), -58)
    XCTAssertEqual(reportedRSSI(0), 0)
  }
}
