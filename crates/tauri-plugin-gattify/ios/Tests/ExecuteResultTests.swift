import Foundation
import Tauri
import XCTest

@testable import tauri_plugin_gattify

final class ExecuteResultTests: XCTestCase {
  private func resolvedPayload(_ result: ExecuteResult) throws -> [String: Any] {
    guard case .resolve(let reply) = result else {
      XCTFail("expected a resolved reply")
      return [:]
    }
    let text = try JsonValue.dictionary(reply).jsonRepresentation()!
    let json = try JSONSerialization.jsonObject(with: Data(text.utf8))
    return json as! [String: Any]
  }

  private func rejected(_ result: ExecuteResult) -> (message: String, code: String)? {
    guard case .reject(let message, let code) = result else { return nil }
    return (message, code)
  }

  func testGetStateResolvesWithAdapterState() throws {
    let result = executeResult(kind: "getState", adapterState: { "sentinel-state" })
    let json = try resolvedPayload(result)
    XCTAssertEqual(json["kind"] as? String, "state")
    XCTAssertEqual(json["payload"] as? String, "sentinel-state")
  }

  func testGetCapabilitiesResolvesLevels() throws {
    let result = executeResult(kind: "getCapabilities", adapterState: { "unknown" })
    let json = try resolvedPayload(result)
    XCTAssertEqual(json["kind"] as? String, "capabilities")
    let payload = try XCTUnwrap(json["payload"] as? [String: Any])
    for key in ["central", "peripheral", "advertising", "targetedNotify", "simultaneousRoles"] {
      let entry = try XCTUnwrap(payload[key] as? [String: Any], key)
      XCTAssertEqual(entry["level"] as? String, "unknown", key)
      XCTAssertEqual(entry["reason"] as? String, "backendNotImplemented", key)
    }
    let background = try XCTUnwrap(payload["background"] as? [String: Any])
    XCTAssertEqual(background["level"] as? String, "unsupported")
    XCTAssertEqual(background["reason"] as? String, "foregroundOnlyContract")
  }

  func testCheckPermissionsResolvesUnknown() throws {
    let result = executeResult(kind: "checkPermissions", adapterState: { "unknown" })
    let json = try resolvedPayload(result)
    XCTAssertEqual(json["kind"] as? String, "permissions")
    let payload = try XCTUnwrap(json["payload"] as? [String: Any])
    XCTAssertEqual(payload["scan"] as? String, "unknown")
    XCTAssertEqual(payload["connect"] as? String, "unknown")
    XCTAssertEqual(payload["advertise"] as? String, "unknown")
  }

  func testCancelAndCloseOwnerResolveEmptyWithNoPayload() throws {
    for kind in ["cancel", "closeOwner"] {
      let result = executeResult(kind: kind, adapterState: { "unknown" })
      let json = try resolvedPayload(result)
      XCTAssertEqual(json["kind"] as? String, "empty", kind)
      XCTAssertNil(json["payload"], kind)
    }
  }

  func testRadioCommandsRejectUnsupported() {
    for kind in ["startScan", "connect", "createServer", "startAdvertising", "notify"] {
      let result = executeResult(kind: kind, adapterState: { "unknown" })
      XCTAssertEqual(rejected(result)?.code, "unsupported", kind)
      XCTAssertEqual(
        rejected(result)?.message, "the iOS backend does not implement \(kind) yet", kind)
    }
  }

  func testMissingOrEmptyKindRejectsUnsupported() {
    let kinds: [String?] = [nil, ""]
    for kind in kinds {
      let result = executeResult(kind: kind, adapterState: { "unknown" })
      XCTAssertEqual(rejected(result)?.code, "unsupported")
      XCTAssertEqual(
        rejected(result)?.message, "the iOS backend does not implement this command yet")
    }
  }
}
