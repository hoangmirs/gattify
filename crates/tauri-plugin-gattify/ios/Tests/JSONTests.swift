import Foundation
import XCTest

@testable import tauri_plugin_gattify

final class JSONTests: XCTestCase {
  func testEncodesExplicitNullsWithSortedKeys() {
    let value: JSON = ["name": nil, "ok": true, "count": 7, "list": ["a", 1, nil]]
    XCTAssertEqual(value.text, #"{"count":7,"list":["a",1,null],"name":null,"ok":true}"#)
  }

  func testOptionalInitializersMapNilToNull() {
    XCTAssertEqual(JSON(String?.none), .null)
    XCTAssertEqual(JSON(Int?.none), .null)
    XCTAssertEqual(JSON(Bool?.none), .null)
    XCTAssertEqual(JSON(String?.some("x")), .string("x"))
    XCTAssertEqual(JSON(Int?.some(-58)), .int(-58))
    XCTAssertEqual(JSON(Bool?.some(false)), .bool(false))
  }

  func testTextParsesBackToTheSameValues() throws {
    let value: JSON = ["key": "peer/tx \"quoted\"\n", "nested": ["deep": [true, false]]]
    let parsed = try XCTUnwrap(
      JSONSerialization.jsonObject(with: Data(value.text.utf8)) as? [String: Any])
    XCTAssertEqual(parsed["key"] as? String, "peer/tx \"quoted\"\n")
    let nested = try XCTUnwrap(parsed["nested"] as? [String: Any])
    XCTAssertEqual(nested["deep"] as? [Bool], [true, false])
  }
}
