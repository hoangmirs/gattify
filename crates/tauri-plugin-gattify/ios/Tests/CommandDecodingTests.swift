import XCTest

@testable import tauri_plugin_gattify

final class CommandDecodingTests: XCTestCase {
  private func decode(_ command: String, deadline: String = "null") -> Result<ExecuteRequest, BridgeError> {
    ExecuteRequest.decode(
      #"{"operationId":"op-1","ownerId":"webview:main","deadlineMillis":\#(deadline),"command":\#(command)}"#)
  }

  private func command(_ json: String) throws -> BridgeCommand {
    try decode(json).get().command
  }

  private func rejection(_ result: Result<ExecuteRequest, BridgeError>) -> BridgeError? {
    guard case .failure(let error) = result else { return nil }
    return error
  }

  func testDecodesTheExecuteArguments() throws {
    let request = try decode(#"{"kind":"getState"}"#, deadline: "5000").get()
    XCTAssertEqual(request.operationId, "op-1")
    XCTAssertEqual(request.ownerId, "webview:main")
    XCTAssertEqual(request.deadlineMillis, 5000)
    XCTAssertEqual(request.command, .getState)
    XCTAssertNil(try decode(#"{"kind":"getState"}"#).get().deadlineMillis)
  }

  func testDecodesCommandsWithoutPayload() throws {
    XCTAssertEqual(try command(#"{"kind":"getState"}"#), .getState)
    XCTAssertEqual(try command(#"{"kind":"getCapabilities"}"#), .getCapabilities)
    XCTAssertEqual(try command(#"{"kind":"checkPermissions"}"#), .checkPermissions)
    XCTAssertEqual(try command(#"{"kind":"closeOwner"}"#), .closeOwner)
    XCTAssertEqual(try command(#"{"kind":"debugResources"}"#), .debugResources)
  }

  func testDecodesCentralCommands() throws {
    XCTAssertEqual(
      try command(#"{"kind":"requestPermissions","payload":{"scan":true,"connect":false,"advertise":true}}"#),
      .requestPermissions(PermissionRequest(scan: true, connect: false, advertise: true)))
    XCTAssertEqual(
      try command(#"{"kind":"startScan","payload":{"serviceUuids":["80ff87c3-8e84-4914-aedc-0d6a3ba5534d"],"timeoutMs":null}}"#),
      .startScan(serviceUuids: ["80ff87c3-8e84-4914-aedc-0d6a3ba5534d"], timeoutMs: nil))
    XCTAssertEqual(
      try command(#"{"kind":"startScan","payload":{"serviceUuids":[],"timeoutMs":10000}}"#),
      .startScan(serviceUuids: [], timeoutMs: 10_000))
    XCTAssertEqual(try command(#"{"kind":"stopScan","payload":{"scanId":"scan-1"}}"#), .stopScan(scanId: "scan-1"))
    XCTAssertEqual(
      try command(#"{"kind":"connect","payload":{"deviceId":"device-3","options":{"timeoutMs":8000}}}"#),
      .connect(deviceId: "device-3", timeoutMs: 8_000))
    XCTAssertEqual(
      try command(#"{"kind":"disconnect","payload":{"connectionId":"connection-2"}}"#),
      .disconnect(connectionId: "connection-2"))
    XCTAssertEqual(
      try command(#"{"kind":"discoverServices","payload":{"connectionId":"connection-2"}}"#),
      .discoverServices(connectionId: "connection-2"))
    XCTAssertEqual(
      try command(#"{"kind":"read","payload":{"connectionId":"connection-2","characteristic":"connection-2/characteristic-1"}}"#),
      .read(connectionId: "connection-2", characteristic: "connection-2/characteristic-1"))
    XCTAssertEqual(
      try command(
        #"{"kind":"write","payload":{"connectionId":"connection-2","characteristic":"connection-2/characteristic-1","valueBase64":"AQI=","writeType":"withoutResponse"}}"#),
      .write(
        connectionId: "connection-2", characteristic: "connection-2/characteristic-1", valueBase64: "AQI=",
        writeType: .withoutResponse))
    XCTAssertEqual(
      try command(#"{"kind":"subscribe","payload":{"connectionId":"connection-2","characteristic":"connection-2/characteristic-3"}}"#),
      .subscribe(connectionId: "connection-2", characteristic: "connection-2/characteristic-3"))
    XCTAssertEqual(
      try command(#"{"kind":"unsubscribe","payload":{"subscriptionId":"subscription-4"}}"#),
      .unsubscribe(subscriptionId: "subscription-4"))
  }

  func testDecodesServerCommands() throws {
    let definition = try command(
      #"""
      {"kind":"createServer","payload":{"services":[{"instanceKey":"peer","uuid":"80ff87c3-8e84-4914-aedc-0d6a3ba5534d","primary":true,"characteristics":[{"instanceKey":"info","uuid":"b1e10f10-6a2c-4a62-8e9e-2c938fa30101","properties":{"read":true,"write":false,"writeWithoutResponse":false,"notify":false,"indicate":false},"initialValueBase64":"AQ==","maxValueLength":1}]}]}}
      """#)
    XCTAssertEqual(
      definition,
      .createServer(
        ServerDefinition(services: [
          LocalService(
            instanceKey: "peer", uuid: "80ff87c3-8e84-4914-aedc-0d6a3ba5534d", primary: true,
            characteristics: [
              LocalCharacteristic(
                instanceKey: "info", uuid: "b1e10f10-6a2c-4a62-8e9e-2c938fa30101",
                properties: CharacteristicProperties(read: true), initialValueBase64: "AQ==", maxValueLength: 1)
            ])
        ])))
    XCTAssertEqual(try command(#"{"kind":"closeServer","payload":{"serverId":"server-1"}}"#), .closeServer(serverId: "server-1"))
    XCTAssertEqual(
      try command(
        #"{"kind":"startAdvertising","payload":{"serverId":"server-1","options":{"serviceUuid":"80ff87c3-8e84-4914-aedc-0d6a3ba5534d","localName":null,"localNameOptional":true}}}"#),
      .startAdvertising(
        serverId: "server-1",
        options: AdvertisingOptions(
          serviceUuid: "80ff87c3-8e84-4914-aedc-0d6a3ba5534d", localName: nil, localNameOptional: true)))
    XCTAssertEqual(
      try command(#"{"kind":"stopAdvertising","payload":{"serverId":"server-1"}}"#), .stopAdvertising(serverId: "server-1"))
    XCTAssertEqual(
      try command(#"{"kind":"setValue","payload":{"serverId":"server-1","characteristicKey":"peer/info","valueBase64":"AQ=="}}"#),
      .setValue(serverId: "server-1", characteristicKey: "peer/info", valueBase64: "AQ=="))
    XCTAssertEqual(
      try command(
        #"{"kind":"notify","payload":{"serverId":"server-1","peerId":"central-2","characteristicKey":"peer/tx","valueBase64":"AQ=="}}"#),
      .notify(serverId: "server-1", peerId: "central-2", characteristicKey: "peer/tx", valueBase64: "AQ=="))
    XCTAssertEqual(try command(#"{"kind":"cancel","payload":{"operationId":"op-9"}}"#), .cancel(operationId: "op-9"))
  }

  func testAnUnknownKindRejectsUnsupported() {
    XCTAssertEqual(rejection(decode(#"{"kind":"teleport"}"#))?.code, "unsupported")
    XCTAssertEqual(rejection(decode(#"{"kind":""}"#))?.code, "unsupported")
  }

  func testAMalformedPayloadRejectsInvalidArgument() {
    let payloads = [
      #"{"kind":"stopScan"}"#,
      #"{"kind":"startScan","payload":{"timeoutMs":null}}"#,
      #"{"kind":"write","payload":{"connectionId":"c","characteristic":"h","valueBase64":"AA==","writeType":"eventually"}}"#,
      #"{"kind":"connect","payload":{"deviceId":7,"options":{"timeoutMs":null}}}"#,
    ]
    for payload in payloads {
      let error = rejection(decode(payload))
      XCTAssertEqual(error?.code, "invalidArgument", payload)
    }
    XCTAssertEqual(
      rejection(decode(#"{"kind":"stopScan"}"#))?.message, "malformed stopScan payload: missing command.payload")
  }

  func testMalformedArgumentsRejectInvalidArgument() {
    XCTAssertEqual(rejection(ExecuteRequest.decode("not json"))?.code, "invalidArgument")
    XCTAssertEqual(
      rejection(ExecuteRequest.decode(#"{"operationId":"op-1","command":{"kind":"getState"}}"#))?.code,
      "invalidArgument")
  }

  func testAppliesTheContractDefaultDeadlines() {
    XCTAssertEqual(BridgeCommand.connect(deviceId: "d", timeoutMs: nil).deadline(requested: nil), 15_000)
    XCTAssertEqual(BridgeCommand.connect(deviceId: "d", timeoutMs: 3_000).deadline(requested: nil), 3_000)
    XCTAssertEqual(BridgeCommand.discoverServices(connectionId: "c").deadline(requested: nil), 10_000)
    let procedures: [BridgeCommand] = [
      .read(connectionId: "c", characteristic: "h"),
      .write(connectionId: "c", characteristic: "h", valueBase64: "", writeType: .withResponse),
      .subscribe(connectionId: "c", characteristic: "h"),
      .unsubscribe(subscriptionId: "s"),
      .notify(serverId: "s", peerId: "p", characteristicKey: "k", valueBase64: ""),
      .createServer(ServerDefinition(services: [])),
      .startAdvertising(serverId: "s", options: AdvertisingOptions(serviceUuid: "u", localName: nil, localNameOptional: true)),
    ]
    for command in procedures {
      XCTAssertEqual(command.deadline(requested: nil), 5_000, "\(command)")
    }
    XCTAssertNil(BridgeCommand.requestPermissions(PermissionRequest(scan: true, connect: true, advertise: true)).deadline(requested: nil))
    XCTAssertNil(BridgeCommand.startScan(serviceUuids: [], timeoutMs: 10_000).deadline(requested: nil))
    XCTAssertEqual(BridgeCommand.connect(deviceId: "d", timeoutMs: 3_000).deadline(requested: 1_000), 1_000)
    XCTAssertEqual(BridgeCommand.getState.deadline(requested: 250), 250)
  }
}
