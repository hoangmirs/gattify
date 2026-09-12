import XCTest

@testable import tauri_plugin_gattify

final class LocalNameTests: XCTestCase {
  func testNoNameAdvertisesNothing() {
    XCTAssertEqual(cutLocalName(nil), LocalNameCut(name: nil, truncated: false))
    XCTAssertEqual(cutLocalName(""), LocalNameCut(name: nil, truncated: false))
    XCTAssertFalse(cutLocalName(nil).included)
  }

  func testAShortNameIsKept() {
    let cut = cutLocalName("Host")
    XCTAssertEqual(cut, LocalNameCut(name: "Host", truncated: false))
    XCTAssertTrue(cut.included)
    XCTAssertEqual(cutLocalName("12345678"), LocalNameCut(name: "12345678", truncated: false))
  }

  func testTheIOSBudgetIsEightBytes() {
    XCTAssertEqual(localNameBudget, 8)
    XCTAssertEqual(cutLocalName("Host A of the lab"), LocalNameCut(name: "Host A o", truncated: true))
  }

  func testCutsAtACharacterBoundary() {
    XCTAssertEqual(cutLocalName("\u{E9}\u{E9}\u{E9}\u{E9}"), LocalNameCut(name: "\u{E9}\u{E9}\u{E9}\u{E9}", truncated: false))
    XCTAssertEqual(cutLocalName("\u{E9}\u{E9}\u{E9}\u{E9}\u{E9}"), LocalNameCut(name: "\u{E9}\u{E9}\u{E9}\u{E9}", truncated: true))
    XCTAssertEqual(cutLocalName("\u{65E5}\u{672C}\u{8A9E}"), LocalNameCut(name: "\u{65E5}\u{672C}", truncated: true))
    XCTAssertEqual(cutLocalName("a\u{1F600}\u{1F600}"), LocalNameCut(name: "a\u{1F600}", truncated: true))
  }

  func testTheCutNeverExceedsTheBudget() {
    for name in ["Host A of the lab", "\u{1F600}\u{1F600}\u{1F600}", "ab\u{65E5}\u{672C}\u{8A9E}", "\u{E9}xyz\u{E9}\u{E9}"] {
      for budget in 0...12 {
        let cut = cutLocalName(name, budget: budget)
        XCTAssertLessThanOrEqual(cut.name?.utf8.count ?? 0, budget, "\(name) at \(budget)")
        XCTAssertTrue(name.hasPrefix(cut.name ?? ""), "\(name) at \(budget)")
      }
    }
  }

  func testANameWhoseFirstCharacterDoesNotFitIsNotIncluded() {
    let cut = cutLocalName("\u{1F600}", budget: 3)
    XCTAssertEqual(cut, LocalNameCut(name: nil, truncated: true))
    XCTAssertFalse(cut.included)
  }
}
