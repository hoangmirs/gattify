import Foundation
import XCTest

@testable import tauri_plugin_gattify

/// Drives the engine through `execute`. A CoreBluetooth manager needs a Bluetooth usage
/// description and a radio, so every test also checks that no manager was created.
final class EngineTests: XCTestCase {
  private let queue = DispatchQueue(label: "engine-tests")
  private var authorization = Authorization.notDetermined
  private var events: [(String, BridgeEvent)] = []
  private lazy var engine = GattifyEngine(queue: queue, authorization: { [unowned self] in self.authorization }) {
    [unowned self] owner, event in self.events.append((owner, event))
  }

  override func tearDown() {
    XCTAssertFalse(engine.hasCentralManager, "a test created a central manager")
    XCTAssertFalse(engine.hasPeripheralManager, "a test created a peripheral manager")
    XCTAssertTrue(events.isEmpty, "a test raised events")
    super.tearDown()
  }

  private func run(_ command: String, owner: String = "webview:main", operation: String = "op-1") -> Result<Reply, BridgeError>? {
    let request = ExecuteRequest.decode(
      #"{"operationId":"\#(operation)","ownerId":"\#(owner)","deadlineMillis":null,"command":\#(command)}"#)
    guard case .success(let request) = request else {
      XCTFail("the test command did not decode: \(command)")
      return nil
    }
    var result: Result<Reply, BridgeError>?
    queue.sync {
      engine.execute(request) { result = $0 }
    }
    return result
  }

  private func reply(_ command: String, owner: String = "webview:main") -> Reply? {
    guard case .success(let reply) = run(command, owner: owner) else { return nil }
    return reply
  }

  private func code(_ command: String) -> String? {
    guard case .failure(let error) = run(command) else { return nil }
    return error.code
  }

  func testGetStateReadsTheAuthorizationWithoutAManager() {
    authorization = .denied
    XCTAssertEqual(reply(#"{"kind":"getState"}"#), .state("unauthorized"))
    authorization = .restricted
    XCTAssertEqual(reply(#"{"kind":"getState"}"#), .state("unauthorized"))
    authorization = .allowedAlways
    XCTAssertEqual(reply(#"{"kind":"getState"}"#), .state("unknown"))
  }

  func testStatusCommands() {
    XCTAssertEqual(reply(#"{"kind":"getCapabilities"}"#), .capabilities)
    authorization = .allowedAlways
    XCTAssertEqual(reply(#"{"kind":"checkPermissions"}"#), .permissions("granted"))
    authorization = .notDetermined
    XCTAssertEqual(reply(#"{"kind":"checkPermissions"}"#), .permissions("promptable"))
  }

  func testRequestPermissionsAnswersAtOnceWhenTheUserAlreadyAnswered() {
    authorization = .denied
    XCTAssertEqual(
      reply(#"{"kind":"requestPermissions","payload":{"scan":true,"connect":true,"advertise":true}}"#),
      .permissions("deniedPermanently"))
  }

  func testRequestPermissionsForNoRoleShowsNoPrompt() {
    XCTAssertEqual(
      reply(#"{"kind":"requestPermissions","payload":{"scan":false,"connect":false,"advertise":false}}"#),
      .permissions("promptable"))
  }

  func testUnknownHandlesRejectInvalidHandle() {
    let commands = [
      #"{"kind":"stopScan","payload":{"scanId":"scan-1"}}"#,
      #"{"kind":"connect","payload":{"deviceId":"device-1","options":{"timeoutMs":null}}}"#,
      #"{"kind":"disconnect","payload":{"connectionId":"connection-1"}}"#,
      #"{"kind":"discoverServices","payload":{"connectionId":"connection-1"}}"#,
      #"{"kind":"read","payload":{"connectionId":"connection-1","characteristic":"connection-1/characteristic-1"}}"#,
      #"{"kind":"write","payload":{"connectionId":"connection-1","characteristic":"connection-1/characteristic-1","valueBase64":"AA==","writeType":"withResponse"}}"#,
      #"{"kind":"subscribe","payload":{"connectionId":"connection-1","characteristic":"connection-1/characteristic-1"}}"#,
      #"{"kind":"unsubscribe","payload":{"subscriptionId":"subscription-1"}}"#,
      #"{"kind":"closeServer","payload":{"serverId":"server-1"}}"#,
      #"{"kind":"startAdvertising","payload":{"serverId":"server-1","options":{"serviceUuid":"80ff87c3-8e84-4914-aedc-0d6a3ba5534d","localName":null,"localNameOptional":true}}}"#,
      #"{"kind":"stopAdvertising","payload":{"serverId":"server-1"}}"#,
      #"{"kind":"setValue","payload":{"serverId":"server-1","characteristicKey":"peer/info","valueBase64":"AQ=="}}"#,
      #"{"kind":"notify","payload":{"serverId":"server-1","peerId":"central-1","characteristicKey":"peer/tx","valueBase64":"AQ=="}}"#,
    ]
    for command in commands {
      XCTAssertEqual(code(command), "invalidHandle", command)
    }
  }

  func testMalformedRadioCommandsRejectBeforeCreatingAManager() {
    XCTAssertEqual(code(#"{"kind":"startScan","payload":{"serviceUuids":["not-a-uuid"],"timeoutMs":null}}"#), "invalidArgument")
    XCTAssertEqual(code(#"{"kind":"createServer","payload":{"services":[]}}"#), "invalidArgument")
  }

  func testCancelOfAnUnknownOperationIsNotAnError() {
    XCTAssertEqual(reply(#"{"kind":"cancel","payload":{"operationId":"op-404"}}"#), .empty)
  }

  func testCloseOwnerIsIdempotent() {
    XCTAssertEqual(reply(#"{"kind":"closeOwner"}"#), .empty)
    XCTAssertEqual(reply(#"{"kind":"closeOwner"}"#), .empty)
    XCTAssertEqual(reply(#"{"kind":"debugResources"}"#), .resources(ResourceCounts()))
  }

  func testFinishedOperationsAreForgotten() {
    _ = reply(#"{"kind":"getState"}"#)
    _ = code(#"{"kind":"stopScan","payload":{"scanId":"scan-1"}}"#)
    queue.sync {
      XCTAssertTrue(engine.operations.isEmpty)
    }
  }
}

final class BridgeOperationTests: XCTestCase {
  func testAnswersExactlyOnce() {
    var results: [Result<Reply, BridgeError>] = []
    let operation = BridgeOperation(id: "op-1", owner: "webview:main") { results.append($0) }
    operation.resolve(.empty)
    operation.reject(.timeout)
    operation.abort(.cancelled)
    XCTAssertEqual(results.count, 1)
    XCTAssertTrue(operation.isFinished)
  }

  func testAnAbortRejectsThenCleansUp() {
    var log: [String] = []
    let operation = BridgeOperation(id: "op-1", owner: "webview:main") { result in
      if case .failure(let error) = result { log.append("rejected \(error.code)") }
    }
    operation.onAbort = { error in log.append("cleanup after \(error.code)") }
    operation.onFinish = { _ in log.append("finished") }
    operation.abort(.cancelled)
    XCTAssertEqual(log, ["finished", "rejected cancelled", "cleanup after cancelled"])
  }

  func testAResolvedOperationSkipsItsCleanup() {
    var cleaned = false
    let operation = BridgeOperation(id: "op-1", owner: "webview:main") { _ in }
    operation.onAbort = { _ in cleaned = true }
    operation.resolve(.empty)
    operation.abort(.timeout)
    XCTAssertFalse(cleaned)
  }

  func testTheDeadlineRejectsWithTimeout() {
    let queue = DispatchQueue(label: "deadline-tests")
    let rejected = expectation(description: "timeout")
    let operation = BridgeOperation(id: "op-1", owner: "webview:main") { result in
      if case .failure(let error) = result, error == .timeout { rejected.fulfill() }
    }
    queue.sync { operation.arm(milliseconds: 20, on: queue) }
    XCTAssertNotNil(operation.deadline)
    wait(for: [rejected], timeout: 2)
  }
}
