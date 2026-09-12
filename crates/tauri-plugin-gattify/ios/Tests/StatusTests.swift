import CoreBluetooth
import XCTest

@testable import tauri_plugin_gattify

final class StatusTests: XCTestCase {
  func testMapsManagerStatesToAdapterStates() {
    XCTAssertEqual(adapterStateName(.poweredOn), "poweredOn")
    XCTAssertEqual(adapterStateName(.poweredOff), "poweredOff")
    XCTAssertEqual(adapterStateName(.resetting), "resetting")
    XCTAssertEqual(adapterStateName(.unauthorized), "unauthorized")
    XCTAssertEqual(adapterStateName(.unsupported), "unavailable")
    XCTAssertEqual(adapterStateName(.unknown), "unknown")
  }

  func testReadsTheStateWithoutAManagerFromTheAuthorization() {
    XCTAssertEqual(adapterStateWithoutManager(.denied), "unauthorized")
    XCTAssertEqual(adapterStateWithoutManager(.restricted), "unauthorized")
    XCTAssertEqual(adapterStateWithoutManager(.notDetermined), "unknown")
    XCTAssertEqual(adapterStateWithoutManager(.allowedAlways), "unknown")
    XCTAssertEqual(adapterStateWithoutManager(.unknown), "unknown")
  }

  func testMapsTheAuthorizationToAPermissionOutcome() {
    XCTAssertEqual(permissionOutcome(.allowedAlways), "granted")
    XCTAssertEqual(permissionOutcome(.notDetermined), "promptable")
    XCTAssertEqual(permissionOutcome(.denied), "deniedPermanently")
    XCTAssertEqual(permissionOutcome(.restricted), "restricted")
    XCTAssertEqual(permissionOutcome(.unknown), "unknown")
  }

  func testRejectsCommandsWhenTheAdapterIsNotOn() {
    XCTAssertNil(BridgeError.adapter(.poweredOn))
    XCTAssertEqual(BridgeError.adapter(.poweredOff)?.code, "bluetoothOff")
    XCTAssertEqual(BridgeError.adapter(.unauthorized)?.code, "permissionDenied")
    XCTAssertEqual(BridgeError.adapter(.unsupported)?.code, "unavailable")
    XCTAssertEqual(BridgeError.adapter(.resetting)?.code, "unavailable")
    XCTAssertEqual(BridgeError.adapter(.unknown)?.code, "unavailable")
  }

  func testMapsCoreBluetoothErrorsToContractCodesWhenOneFits() {
    XCTAssertEqual(nativeErrorCode(domain: CBErrorDomain, code: CBError.Code.notConnected.rawValue), "disconnected")
    XCTAssertEqual(nativeErrorCode(domain: CBErrorDomain, code: CBError.Code.peripheralDisconnected.rawValue), "disconnected")
    XCTAssertEqual(nativeErrorCode(domain: CBErrorDomain, code: CBError.Code.connectionTimeout.rawValue), "disconnected")
    XCTAssertEqual(nativeErrorCode(domain: CBErrorDomain, code: CBError.Code.alreadyAdvertising.rawValue), "busy")
    XCTAssertEqual(nativeErrorCode(domain: CBErrorDomain, code: CBError.Code.connectionLimitReached.rawValue), "busy")
    XCTAssertEqual(nativeErrorCode(domain: CBErrorDomain, code: CBError.Code.operationNotSupported.rawValue), "unsupported")
    XCTAssertEqual(
      nativeErrorCode(domain: CBATTErrorDomain, code: CBATTError.Code.invalidAttributeValueLength.rawValue),
      "payloadTooLarge")
  }

  func testKeepsOtherCoreBluetoothErrorsAsPlatformCodes() {
    XCTAssertEqual(nativeErrorCode(domain: CBErrorDomain, code: CBError.Code.connectionFailed.rawValue), "cbError10")
    XCTAssertEqual(nativeErrorCode(domain: CBErrorDomain, code: 0), "cbError0")
    XCTAssertEqual(nativeErrorCode(domain: CBATTErrorDomain, code: CBATTError.Code.readNotPermitted.rawValue), "cbAttError2")
    XCTAssertEqual(nativeErrorCode(domain: NSPOSIXErrorDomain, code: 5), "internal")
    let error = BridgeError(NSError(domain: CBATTErrorDomain, code: 14, userInfo: [NSLocalizedDescriptionKey: "unlikely"]))
    XCTAssertEqual(error, BridgeError(code: "cbAttError14", message: "unlikely"))
  }
}
