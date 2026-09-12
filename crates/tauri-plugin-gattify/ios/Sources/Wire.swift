import Foundation

/// The longest attribute value in ATT.
let maxAttributeValueLength = 512

/// The foreground advertising bytes iOS gives an app.
let maxAdvertisingDataLength = 28

func clampedValueLength(_ length: Int) -> Int {
  min(length, maxAttributeValueLength)
}

struct LinkLimits: Equatable {
  let writeWithResponse: Int
  let writeWithoutResponse: Int
  let notification: Int
  let attMtu: Int

  /// iOS reports one value length for the link: the ATT MTU minus 3.
  init(maximumWriteValueLength length: Int) {
    let value = clampedValueLength(length)
    writeWithResponse = value
    writeWithoutResponse = value
    notification = value
    attMtu = length + 3
  }

  var json: JSON {
    [
      "writeWithResponse": .int(writeWithResponse),
      "writeWithoutResponse": .int(writeWithoutResponse),
      "notification": .int(notification),
      "attMtu": .int(attMtu),
    ]
  }
}

struct CharacteristicInstance: Equatable {
  let handle: String
  let uuid: String
  let properties: CharacteristicProperties
}

struct ServiceInstance: Equatable {
  let handle: String
  let uuid: String
  let characteristics: [CharacteristicInstance]
}

struct ResourceCounts: Equatable {
  var scans = 0
  var connections = 0
  var subscriptions = 0
  var servers = 0
}

enum Reply: Equatable {
  case empty
  case state(String)
  case capabilities
  case permissions(String)
  case scanStarted(scanId: String)
  case connected(connectionId: String, limits: LinkLimits)
  case services([ServiceInstance])
  case bytes(Data)
  case subscriptionStarted(subscriptionId: String)
  case serverCreated(serverId: String)
  case advertisingStarted(localNameIncluded: Bool, localNameTruncated: Bool)
  case resources(ResourceCounts)

  var json: JSON {
    switch self {
    case .empty:
      return ["kind": "empty"]
    case .state(let state):
      return ["kind": "state", "payload": .string(state)]
    case .capabilities:
      return ["kind": "capabilities", "payload": capabilitiesPayload]
    case .permissions(let outcome):
      let value = JSON.string(outcome)
      return ["kind": "permissions", "payload": ["scan": value, "connect": value, "advertise": value]]
    case .scanStarted(let scanId):
      return ["kind": "scanStarted", "payload": ["scanId": .string(scanId)]]
    case .connected(let connectionId, let limits):
      return [
        "kind": "connected",
        "payload": ["connectionId": .string(connectionId), "limits": limits.json],
      ]
    case .services(let services):
      return ["kind": "services", "payload": .array(services.map(\.json))]
    case .bytes(let value):
      return ["kind": "bytes", "payload": ["valueBase64": .string(value.base64EncodedString())]]
    case .subscriptionStarted(let subscriptionId):
      return ["kind": "subscriptionStarted", "payload": ["subscriptionId": .string(subscriptionId)]]
    case .serverCreated(let serverId):
      return ["kind": "serverCreated", "payload": ["serverId": .string(serverId)]]
    case .advertisingStarted(let included, let truncated):
      return [
        "kind": "advertisingStarted",
        "payload": ["localNameIncluded": .bool(included), "localNameTruncated": .bool(truncated)],
      ]
    case .resources(let counts):
      return [
        "kind": "resources",
        "payload": [
          "scans": .int(counts.scans),
          "connections": .int(counts.connections),
          "subscriptions": .int(counts.subscriptions),
          "servers": .int(counts.servers),
        ],
      ]
    }
  }
}

private let capabilitiesPayload: JSON = {
  let supported: JSON = ["level": "supported", "reason": "available", "description": nil]
  return [
    "central": supported,
    "peripheral": supported,
    "advertising": supported,
    "targetedNotify": supported,
    "simultaneousRoles": supported,
    "background": ["level": "unsupported", "reason": "foregroundOnlyContract", "description": nil],
    "maxConnections": nil,
    "maxAdvertisingDataLength": .int(maxAdvertisingDataLength),
  ]
}()

struct ServiceDataEntry: Equatable {
  let uuid: String
  let bytes: Data
}

struct ManufacturerDataEntry: Equatable {
  let companyId: Int
  let bytes: Data

  /// Splits advertised manufacturer data into its little-endian company ID and the rest.
  init?(_ data: Data) {
    let bytes = [UInt8](data)
    guard bytes.count >= 2 else { return nil }
    companyId = Int(bytes[0]) | Int(bytes[1]) << 8
    self.bytes = Data(bytes[2...])
  }

  init(companyId: Int, bytes: Data) {
    self.companyId = companyId
    self.bytes = bytes
  }
}

struct AdvertisementData: Equatable {
  var localName: String?
  var serviceData: [ServiceDataEntry] = []
  var manufacturerData: [ManufacturerDataEntry] = []
  var connectable: Bool?

  var json: JSON {
    [
      "localName": JSON(localName),
      "serviceData": .array(
        serviceData.map { entry in
          ["serviceUuid": .string(entry.uuid), "bytesBase64": .string(entry.bytes.base64EncodedString())]
        }),
      "manufacturerData": .array(
        manufacturerData.map { entry in
          ["companyId": .int(entry.companyId), "bytesBase64": .string(entry.bytes.base64EncodedString())]
        }),
      "connectable": JSON(connectable),
    ]
  }
}

struct DiscoveredDevice: Equatable {
  let id: String
  let name: String?
  let rssi: Int?
  let serviceUuids: [String]
  let advertisement: AdvertisementData
  let observedAtMillis: Int
  let scanId: String

  var json: JSON {
    [
      "id": .string(id),
      "name": JSON(name),
      "rssi": JSON(rssi),
      "serviceUuids": .array(serviceUuids.map(JSON.string)),
      "advertisement": advertisement.json,
      "observedAtMillis": .int(observedAtMillis),
      "scanId": .string(scanId),
    ]
  }
}

enum BridgeEvent: Equatable {
  case adapterStateChanged(state: String)
  case scanResult(DiscoveredDevice)
  case scanStopped(scanId: String)
  case connectionClosed(connectionId: String)
  case characteristicValue(subscriptionId: String, value: Data)
  case serverWrite(serverId: String, peerId: String, characteristicKey: String, value: Data)
  case subscriptionChanged(
    serverId: String, peerId: String, characteristicKey: String, subscribed: Bool,
    maxValueLength: Int?)
  case criticalStateLoss(resourceId: String, reason: String)

  var kind: String {
    switch self {
    case .adapterStateChanged: return "adapterStateChanged"
    case .scanResult: return "scanResult"
    case .scanStopped: return "scanStopped"
    case .connectionClosed: return "connectionClosed"
    case .characteristicValue: return "characteristicValue"
    case .serverWrite: return "serverWrite"
    case .subscriptionChanged: return "subscriptionChanged"
    case .criticalStateLoss: return "criticalStateLoss"
    }
  }

  var payload: JSON {
    switch self {
    case .adapterStateChanged(let state):
      return ["state": .string(state)]
    case .scanResult(let device):
      return ["device": device.json]
    case .scanStopped(let scanId):
      return ["scanId": .string(scanId)]
    case .connectionClosed(let connectionId):
      return ["connectionId": .string(connectionId)]
    case .characteristicValue(let subscriptionId, let value):
      return ["subscriptionId": .string(subscriptionId), "valueBase64": .string(value.base64EncodedString())]
    case .serverWrite(let serverId, let peerId, let key, let value):
      return [
        "serverId": .string(serverId),
        "peerId": .string(peerId),
        "characteristicKey": .string(key),
        "valueBase64": .string(value.base64EncodedString()),
      ]
    case .subscriptionChanged(let serverId, let peerId, let key, let subscribed, let maxValueLength):
      return [
        "serverId": .string(serverId),
        "peerId": .string(peerId),
        "characteristicKey": .string(key),
        "subscribed": .bool(subscribed),
        "maxValueLength": JSON(maxValueLength),
      ]
    case .criticalStateLoss(let resourceId, let reason):
      return ["resourceId": .string(resourceId), "reason": .string(reason)]
    }
  }

  func envelope(ownerId: String) -> JSON {
    ["ownerId": .string(ownerId), "event": ["kind": .string(kind), "payload": payload]]
  }
}

extension CharacteristicProperties {
  var json: JSON {
    [
      "read": .bool(read),
      "write": .bool(write),
      "writeWithoutResponse": .bool(writeWithoutResponse),
      "notify": .bool(notify),
      "indicate": .bool(indicate),
    ]
  }
}

extension CharacteristicInstance {
  var json: JSON {
    ["handle": .string(handle), "uuid": .string(uuid), "properties": properties.json]
  }
}

extension ServiceInstance {
  var json: JSON {
    ["handle": .string(handle), "uuid": .string(uuid), "characteristics": .array(characteristics.map(\.json))]
  }
}
