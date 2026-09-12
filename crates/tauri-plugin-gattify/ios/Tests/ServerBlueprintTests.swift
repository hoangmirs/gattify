import CoreBluetooth
import XCTest

@testable import tauri_plugin_gattify

final class ServerBlueprintTests: XCTestCase {
  func testBuildsDynamicCharacteristicsFromAServerDefinition() throws {
    let definition = ServerDefinition(services: [
      LocalService(
        instanceKey: "peer", uuid: "80ff87c3-8e84-4914-aedc-0d6a3ba5534d", primary: true,
        characteristics: [
          LocalCharacteristic(
            instanceKey: "info", uuid: "b1e10f10-6a2c-4a62-8e9e-2c938fa30101",
            properties: CharacteristicProperties(read: true), initialValueBase64: "AQ==", maxValueLength: 1),
          LocalCharacteristic(
            instanceKey: "tx", uuid: "b1e10f10-6a2c-4a62-8e9e-2c938fa30103",
            properties: CharacteristicProperties(notify: true), initialValueBase64: nil, maxValueLength: 512),
        ])
    ])
    let blueprint = try ServerBlueprint.build(definition).get()
    XCTAssertEqual(blueprint.serviceUuids, ["80ff87c3-8e84-4914-aedc-0d6a3ba5534d"])
    XCTAssertEqual(blueprint.services.count, 1)
    XCTAssertEqual(blueprint.services[0].isPrimary, true)
    XCTAssertEqual(blueprint.services[0].characteristics?.count, 2)
    XCTAssertEqual(Set(blueprint.attributes.keys), ["peer/info", "peer/tx"])
    let info = try XCTUnwrap(blueprint.attributes["peer/info"])
    XCTAssertEqual(info.value, Data([1]))
    XCTAssertEqual(info.maxValueLength, 1)
    XCTAssertNil(info.characteristic.value, "a nil value makes iOS ask the plugin for every read")
    XCTAssertEqual(canonicalUUID(info.characteristic.uuid), "b1e10f10-6a2c-4a62-8e9e-2c938fa30101")
    XCTAssertEqual(info.characteristic.properties, [.read])
    XCTAssertEqual(blueprint.attributes["peer/tx"]?.value, Data())
  }

  func testRejectsAMalformedServerDefinition() {
    func build(initial: String? = nil, max: UInt32 = 4, uuid: String = "b1e10f10-6a2c-4a62-8e9e-2c938fa30101") -> String? {
      let definition = ServerDefinition(services: [
        LocalService(
          instanceKey: "s", uuid: "80ff87c3-8e84-4914-aedc-0d6a3ba5534d", primary: true,
          characteristics: [
            LocalCharacteristic(
              instanceKey: "c", uuid: uuid, properties: CharacteristicProperties(read: true),
              initialValueBase64: initial, maxValueLength: max)
          ])
      ])
      guard case .failure(let error) = ServerBlueprint.build(definition) else { return nil }
      return error.code
    }
    XCTAssertNil(build())
    XCTAssertEqual(build(initial: "not base64!"), "invalidArgument")
    XCTAssertEqual(build(initial: "AQIDBAU=", max: 4), "invalidArgument")
    XCTAssertEqual(build(uuid: "nope"), "invalidArgument")
    guard case .failure(let error) = ServerBlueprint.build(ServerDefinition(services: [])) else {
      return XCTFail("an empty definition must fail")
    }
    XCTAssertEqual(error.code, "invalidArgument")
  }
}
