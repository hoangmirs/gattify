import Foundation
import XCTest

@testable import tauri_plugin_gattify

final class WireTests: XCTestCase {
  func testAnEmptyReplyHasNoPayloadKey() {
    XCTAssertEqual(Reply.empty.json, ["kind": "empty"])
    XCTAssertEqual(Reply.empty.json.text, #"{"kind":"empty"}"#)
  }

  func testStateAndPermissionsReplies() {
    XCTAssertEqual(Reply.state("poweredOn").json, ["kind": "state", "payload": "poweredOn"])
    XCTAssertEqual(
      Reply.permissions("promptable").json,
      ["kind": "permissions", "payload": ["scan": "promptable", "connect": "promptable", "advertise": "promptable"]])
  }

  func testCapabilitiesHaveEveryField() {
    let supported: JSON = ["level": "supported", "reason": "available", "description": nil]
    XCTAssertEqual(
      Reply.capabilities.json,
      [
        "kind": "capabilities",
        "payload": [
          "central": supported,
          "peripheral": supported,
          "advertising": supported,
          "targetedNotify": supported,
          "simultaneousRoles": supported,
          "background": ["level": "unsupported", "reason": "foregroundOnlyContract", "description": nil],
          "maxConnections": nil,
          "maxAdvertisingDataLength": 28,
        ],
      ])
  }

  func testLinkLimitsClampValueLengthsButNotTheMTU() {
    XCTAssertEqual(
      LinkLimits(maximumWriteValueLength: 182).json,
      ["writeWithResponse": 182, "writeWithoutResponse": 182, "notification": 182, "attMtu": 185])
    XCTAssertEqual(
      LinkLimits(maximumWriteValueLength: 524).json,
      ["writeWithResponse": 512, "writeWithoutResponse": 512, "notification": 512, "attMtu": 527])
    XCTAssertEqual(
      Reply.connected(connectionId: "connection-1", limits: LinkLimits(maximumWriteValueLength: 20)).json,
      [
        "kind": "connected",
        "payload": [
          "connectionId": "connection-1",
          "limits": ["writeWithResponse": 20, "writeWithoutResponse": 20, "notification": 20, "attMtu": 23],
        ],
      ])
  }

  func testServicesReply() {
    let services = [
      ServiceInstance(
        handle: "connection-2/service-1", uuid: "80ff87c3-8e84-4914-aedc-0d6a3ba5534d",
        characteristics: [
          CharacteristicInstance(
            handle: "connection-2/characteristic-1", uuid: "b1e10f10-6a2c-4a62-8e9e-2c938fa30101",
            properties: CharacteristicProperties(read: true, indicate: true))
        ])
    ]
    XCTAssertEqual(
      Reply.services(services).json,
      [
        "kind": "services",
        "payload": [
          [
            "handle": "connection-2/service-1",
            "uuid": "80ff87c3-8e84-4914-aedc-0d6a3ba5534d",
            "characteristics": [
              [
                "handle": "connection-2/characteristic-1",
                "uuid": "b1e10f10-6a2c-4a62-8e9e-2c938fa30101",
                "properties": [
                  "read": true, "write": false, "writeWithoutResponse": false, "notify": false, "indicate": true,
                ],
              ]
            ],
          ]
        ],
      ])
    XCTAssertEqual(Reply.services([]).json, ["kind": "services", "payload": []])
  }

  func testRepliesWithIdentifiersAndBytes() {
    XCTAssertEqual(Reply.bytes(Data([1, 2])).json, ["kind": "bytes", "payload": ["valueBase64": "AQI="]])
    XCTAssertEqual(Reply.bytes(Data()).json, ["kind": "bytes", "payload": ["valueBase64": ""]])
    XCTAssertEqual(Reply.scanStarted(scanId: "scan-1").json, ["kind": "scanStarted", "payload": ["scanId": "scan-1"]])
    XCTAssertEqual(
      Reply.subscriptionStarted(subscriptionId: "subscription-1").json,
      ["kind": "subscriptionStarted", "payload": ["subscriptionId": "subscription-1"]])
    XCTAssertEqual(
      Reply.serverCreated(serverId: "server-1").json, ["kind": "serverCreated", "payload": ["serverId": "server-1"]])
    XCTAssertEqual(
      Reply.advertisingStarted(localNameIncluded: true, localNameTruncated: false).json,
      ["kind": "advertisingStarted", "payload": ["localNameIncluded": true, "localNameTruncated": false]])
    XCTAssertEqual(
      Reply.resources(ResourceCounts(scans: 1, connections: 2, subscriptions: 3, servers: 4)).json,
      ["kind": "resources", "payload": ["scans": 1, "connections": 2, "subscriptions": 3, "servers": 4]])
  }

  func testEventsTravelInAnOwnerEnvelope() throws {
    let envelope = BridgeEvent.scanStopped(scanId: "scan-1").envelope(ownerId: "webview:main")
    XCTAssertEqual(
      envelope, ["ownerId": "webview:main", "event": ["kind": "scanStopped", "payload": ["scanId": "scan-1"]]])
    let parsed = try XCTUnwrap(
      JSONSerialization.jsonObject(with: Data(envelope.text.utf8)) as? [String: Any])
    XCTAssertEqual(parsed["ownerId"] as? String, "webview:main")
  }

  func testEventPayloads() {
    XCTAssertEqual(BridgeEvent.adapterStateChanged(state: "poweredOff").payload, ["state": "poweredOff"])
    XCTAssertEqual(BridgeEvent.connectionClosed(connectionId: "connection-1").payload, ["connectionId": "connection-1"])
    XCTAssertEqual(
      BridgeEvent.characteristicValue(subscriptionId: "subscription-1", value: Data([0xFF])).payload,
      ["subscriptionId": "subscription-1", "valueBase64": "/w=="])
    XCTAssertEqual(
      BridgeEvent.serverWrite(serverId: "server-1", peerId: "central-1", characteristicKey: "peer/rx", value: Data([1]))
        .payload,
      ["serverId": "server-1", "peerId": "central-1", "characteristicKey": "peer/rx", "valueBase64": "AQ=="])
    XCTAssertEqual(
      BridgeEvent.criticalStateLoss(resourceId: "server-1", reason: "bluetoothOff").payload,
      ["resourceId": "server-1", "reason": "bluetoothOff"])
  }

  func testSubscriptionChangesCarryTheNotificationSizeOnlyWhileSubscribed() {
    let subscribed = BridgeEvent.subscriptionChanged(
      serverId: "server-1", peerId: "central-1", characteristicKey: "peer/tx", subscribed: true, maxValueLength: 182)
    XCTAssertEqual(subscribed.kind, "subscriptionChanged")
    XCTAssertEqual(
      subscribed.payload,
      ["serverId": "server-1", "peerId": "central-1", "characteristicKey": "peer/tx", "subscribed": true, "maxValueLength": 182])
    let unsubscribed = BridgeEvent.subscriptionChanged(
      serverId: "server-1", peerId: "central-1", characteristicKey: "peer/tx", subscribed: false, maxValueLength: nil)
    XCTAssertEqual(
      unsubscribed.payload,
      ["serverId": "server-1", "peerId": "central-1", "characteristicKey": "peer/tx", "subscribed": false, "maxValueLength": nil])
  }

  func testScanResultCarriesTheWholeDevice() {
    let device = DiscoveredDevice(
      id: "device-3", name: "Host A", rssi: -58, serviceUuids: ["80ff87c3-8e84-4914-aedc-0d6a3ba5534d"],
      advertisement: AdvertisementData(
        localName: "Host A",
        serviceData: [ServiceDataEntry(uuid: "80ff87c3-8e84-4914-aedc-0d6a3ba5534d", bytes: Data("Host A".utf8))],
        manufacturerData: [ManufacturerDataEntry(companyId: 76, bytes: Data([1, 2]))],
        connectable: true),
      observedAtMillis: 1_757_664_000_000, scanId: "scan-1")
    XCTAssertEqual(
      BridgeEvent.scanResult(device).payload,
      [
        "device": [
          "id": "device-3",
          "name": "Host A",
          "rssi": -58,
          "serviceUuids": ["80ff87c3-8e84-4914-aedc-0d6a3ba5534d"],
          "advertisement": [
            "localName": "Host A",
            "serviceData": [["serviceUuid": "80ff87c3-8e84-4914-aedc-0d6a3ba5534d", "bytesBase64": "SG9zdCBB"]],
            "manufacturerData": [["companyId": 76, "bytesBase64": "AQI="]],
            "connectable": true,
          ],
          "observedAtMillis": .int(1_757_664_000_000),
          "scanId": "scan-1",
        ]
      ])
  }

  func testAnAnonymousDeviceReportsNulls() {
    let device = DiscoveredDevice(
      id: "device-1", name: nil, rssi: nil, serviceUuids: [], advertisement: AdvertisementData(),
      observedAtMillis: 0, scanId: "scan-2")
    XCTAssertEqual(
      device.json,
      [
        "id": "device-1", "name": nil, "rssi": nil, "serviceUuids": [],
        "advertisement": ["localName": nil, "serviceData": [], "manufacturerData": [], "connectable": nil],
        "observedAtMillis": 0, "scanId": "scan-2",
      ])
  }
}
