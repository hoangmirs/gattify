import CoreBluetooth
import XCTest

@testable import tauri_plugin_gattify

final class UUIDTests: XCTestCase {
  private let heartRate = "0000180d-0000-1000-8000-00805f9b34fb"
  private let lab = "80ff87c3-8e84-4914-aedc-0d6a3ba5534d"

  func testExpandsShortUUIDsWithTheBluetoothBase() {
    XCTAssertEqual(canonicalUUID("180D"), heartRate)
    XCTAssertEqual(canonicalUUID("180d"), heartRate)
    XCTAssertEqual(canonicalUUID("0000180D"), heartRate)
    XCTAssertEqual(canonicalUUID("12345678"), "12345678-0000-1000-8000-00805f9b34fb")
  }

  func testLowercasesAndHyphenates128BitUUIDs() {
    XCTAssertEqual(canonicalUUID("80FF87C3-8E84-4914-AEDC-0D6A3BA5534D"), lab)
    XCTAssertEqual(canonicalUUID("80FF87C38E844914AEDC0D6A3BA5534D"), lab)
    XCTAssertEqual(canonicalUUID(lab), lab)
  }

  func testRejectsMalformedUUIDs() {
    for value in ["", "xyz", "180", "18 0D", "gggg", "80ff87c3-8e84-4914-aedc-0d6a3ba5534", "\u{FF11}\u{FF18}\u{FF10}\u{FF24}"] {
      XCTAssertNil(canonicalUUID(value), value)
    }
  }

  func testCanonicalizesCoreBluetoothUUIDs() {
    XCTAssertEqual(canonicalUUID(CBUUID(string: "180D")), heartRate)
    XCTAssertEqual(canonicalUUID(CBUUID(string: "0000180D-0000-1000-8000-00805F9B34FB")), heartRate)
    XCTAssertEqual(canonicalUUID(CBUUID(string: "80FF87C3-8E84-4914-AEDC-0D6A3BA5534D")), lab)
  }

  func testMakesCoreBluetoothUUIDsOnlyFromValidStrings() throws {
    XCTAssertNil(makeCBUUID("not a uuid"))
    XCTAssertEqual(canonicalUUID(try XCTUnwrap(makeCBUUID(lab))), lab)
    XCTAssertEqual(canonicalUUID(try XCTUnwrap(makeCBUUID(heartRate))), heartRate)
    XCTAssertEqual(canonicalUUID(try XCTUnwrap(makeCBUUID("180d"))), heartRate)
  }
}
