import CoreBluetooth
import XCTest

@testable import tauri_plugin_gattify

final class AttributesTests: XCTestCase {
  private func part(_ target: Int?, offset: Int, _ bytes: [UInt8], max: Int = 512, writable: Bool = true) -> WritePart {
    WritePart(target: target, writable: writable, maxValueLength: max, offset: offset, value: Data(bytes))
  }

  func testASingleWriteIsAccepted() {
    XCTAssertEqual(assembleWrites([part(0, offset: 0, [1, 2])]), .accepted([AssembledWrite(target: 0, value: Data([1, 2]))]))
  }

  func testAssemblesTheLongWriteOfEachCharacteristicInTheOrderOfItsFirstPart() {
    let outcome = assembleWrites([
      part(0, offset: 0, [1, 2]),
      part(1, offset: 0, [9]),
      part(0, offset: 2, [3]),
      part(1, offset: 1, [8, 7]),
    ])
    XCTAssertEqual(
      outcome,
      .accepted([AssembledWrite(target: 0, value: Data([1, 2, 3])), AssembledWrite(target: 1, value: Data([9, 8, 7]))]))
  }

  func testPartsMustStartAtZeroAndBeContiguous() {
    XCTAssertEqual(assembleWrites([part(0, offset: 1, [1])]), .rejected(.invalidOffset))
    XCTAssertEqual(assembleWrites([part(0, offset: 0, [1, 2]), part(0, offset: 3, [3])]), .rejected(.invalidOffset))
    XCTAssertEqual(assembleWrites([part(0, offset: 0, [1, 2]), part(0, offset: 1, [3])]), .rejected(.invalidOffset))
  }

  func testTheAssembledValueMustFitTheCharacteristic() {
    XCTAssertEqual(assembleWrites([part(0, offset: 0, [1, 2], max: 2)]), .accepted([AssembledWrite(target: 0, value: Data([1, 2]))]))
    XCTAssertEqual(assembleWrites([part(0, offset: 0, [1, 2, 3], max: 2)]), .rejected(.invalidAttributeValueLength))
    XCTAssertEqual(
      assembleWrites([part(0, offset: 0, [1, 2], max: 3), part(0, offset: 2, [3, 4], max: 3)]),
      .rejected(.invalidAttributeValueLength))
  }

  func testTheCharacteristicMustBeWritableAndKnown() {
    XCTAssertEqual(assembleWrites([part(0, offset: 0, [1], writable: false)]), .rejected(.writeNotPermitted))
    XCTAssertEqual(assembleWrites([part(nil, offset: 0, [1])]), .rejected(.attributeNotFound))
  }

  func testTheWholeBatchFailsWithItsFirstError() {
    let outcome = assembleWrites([
      part(0, offset: 0, [1]),
      part(1, offset: 0, [1], writable: false),
      part(0, offset: 5, [1]),
    ])
    XCTAssertEqual(outcome, .rejected(.writeNotPermitted))
  }

  func testReadsServeTheStoredValueFromTheRequestOffset() {
    let value = Data([1, 2, 3, 4]).dropFirst(1)
    XCTAssertEqual(readSlice(value, offset: 0), Data([2, 3, 4]))
    XCTAssertEqual(readSlice(value, offset: 2), Data([4]))
    XCTAssertEqual(readSlice(value, offset: 3), Data())
    XCTAssertNil(readSlice(value, offset: 4))
    XCTAssertNil(readSlice(value, offset: -1))
    XCTAssertEqual(readSlice(value, offset: 1)?.startIndex, 0)
  }

  func testMapsCharacteristicProperties() {
    let remote = CharacteristicProperties([.read, .writeWithoutResponse, .indicate])
    XCTAssertEqual(remote, CharacteristicProperties(read: true, writeWithoutResponse: true, indicate: true))
    XCTAssertTrue(remote.writable)
    XCTAssertTrue(remote.notifiable)
    XCTAssertEqual(CharacteristicProperties([.notifyEncryptionRequired]), CharacteristicProperties(notify: true))

    let local = CharacteristicProperties(read: true, write: true, notify: true)
    XCTAssertEqual(local.cbProperties, [.read, .write, .notify])
    XCTAssertEqual(local.cbPermissions, [.readable, .writeable])
    XCTAssertEqual(CharacteristicProperties(notify: true).cbPermissions, [])
    XCTAssertEqual(CharacteristicProperties(writeWithoutResponse: true).cbPermissions, [.writeable])
  }

  func testAttributeKeysCountRepeatedUUIDs() {
    XCTAssertEqual(occurrenceKeys(["a", "b", "a", "a"]), ["a#0", "b#0", "a#1", "a#2"])
    XCTAssertEqual(occurrenceKeys([]), [])
  }

  func testAllocatesIdentifiersPerPrefixWithoutReuse() {
    var ids = IDAllocator()
    XCTAssertEqual(ids.next("device"), "device-1")
    XCTAssertEqual(ids.next("device"), "device-2")
    XCTAssertEqual(ids.next("scan"), "scan-1")
    XCTAssertEqual(ids.next("device"), "device-3")
  }

  func testTheOwnerFamilyIsTheTextAfterTheFirstColon() {
    XCTAssertEqual(ownerFamily("webview:main"), "main")
    XCTAssertEqual(ownerFamily("gattify-peer:main"), "main")
    XCTAssertEqual(ownerFamily("webview:a:b"), "a:b")
    XCTAssertEqual(ownerFamily("plain"), "plain")
  }
}
