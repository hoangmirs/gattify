import Foundation

enum WriteType: String, Decodable, Equatable {
  case withResponse
  case withoutResponse
}

struct CharacteristicProperties: Decodable, Equatable {
  var read = false
  var write = false
  var writeWithoutResponse = false
  var notify = false
  var indicate = false
}

struct PermissionRequest: Decodable, Equatable {
  let scan: Bool
  let connect: Bool
  let advertise: Bool
}

struct LocalCharacteristic: Decodable, Equatable {
  let instanceKey: String
  let uuid: String
  let properties: CharacteristicProperties
  let initialValueBase64: String?
  let maxValueLength: UInt32
}

struct LocalService: Decodable, Equatable {
  let instanceKey: String
  let uuid: String
  let primary: Bool
  let characteristics: [LocalCharacteristic]
}

struct ServerDefinition: Decodable, Equatable {
  let services: [LocalService]
}

struct AdvertisingOptions: Decodable, Equatable {
  let serviceUuid: String
  let localName: String?
  let localNameOptional: Bool
}

enum BridgeCommand: Equatable {
  case getState
  case getCapabilities
  case checkPermissions
  case requestPermissions(PermissionRequest)
  case startScan(serviceUuids: [String], timeoutMs: UInt64?)
  case stopScan(scanId: String)
  case connect(deviceId: String, timeoutMs: UInt64?)
  case disconnect(connectionId: String)
  case discoverServices(connectionId: String)
  case read(connectionId: String, characteristic: String)
  case write(connectionId: String, characteristic: String, valueBase64: String, writeType: WriteType)
  case subscribe(connectionId: String, characteristic: String)
  case unsubscribe(subscriptionId: String)
  case createServer(ServerDefinition)
  case closeServer(serverId: String)
  case startAdvertising(serverId: String, options: AdvertisingOptions)
  case stopAdvertising(serverId: String)
  case setValue(serverId: String, characteristicKey: String, valueBase64: String)
  case notify(serverId: String, peerId: String, characteristicKey: String, valueBase64: String)
  case cancel(operationId: String)
  case closeOwner
  case debugResources

  /// The deadline in milliseconds: the requested one, else the contract default, else none.
  func deadline(requested: UInt64?) -> UInt64? {
    if let requested { return requested }
    switch self {
    case .connect(_, let timeoutMs):
      return timeoutMs ?? 15_000
    case .discoverServices:
      return 10_000
    case .read, .write, .subscribe, .unsubscribe, .notify, .createServer, .startAdvertising:
      return 5_000
    default:
      return nil
    }
  }
}

/// The arguments of the native `execute` command.
struct ExecuteRequest: Equatable {
  let operationId: String
  let ownerId: String
  let deadlineMillis: UInt64?
  let command: BridgeCommand

  static func decode(_ text: String) -> Result<ExecuteRequest, BridgeError> {
    decode(Data(text.utf8))
  }

  static func decode(_ data: Data) -> Result<ExecuteRequest, BridgeError> {
    let decoder = JSONDecoder()
    let header: Header
    do {
      header = try decoder.decode(Header.self, from: data)
    } catch {
      return .failure(.invalidArgument("malformed execute arguments: \(describe(error))"))
    }
    let kind = header.command.kind
    func payload<Payload: Decodable>(_ type: Payload.Type) throws -> Payload {
      try decoder.decode(Envelope<Payload>.self, from: data).command.payload
    }
    let command: BridgeCommand
    do {
      switch kind {
      case "getState":
        command = .getState
      case "getCapabilities":
        command = .getCapabilities
      case "checkPermissions":
        command = .checkPermissions
      case "requestPermissions":
        command = .requestPermissions(try payload(PermissionRequest.self))
      case "startScan":
        let options = try payload(ScanOptions.self)
        command = .startScan(serviceUuids: options.serviceUuids, timeoutMs: options.timeoutMs)
      case "stopScan":
        command = .stopScan(scanId: try payload(ScanTarget.self).scanId)
      case "connect":
        let target = try payload(ConnectTarget.self)
        command = .connect(deviceId: target.deviceId, timeoutMs: target.options.timeoutMs)
      case "disconnect":
        command = .disconnect(connectionId: try payload(ConnectionTarget.self).connectionId)
      case "discoverServices":
        command = .discoverServices(connectionId: try payload(ConnectionTarget.self).connectionId)
      case "read":
        let target = try payload(CharacteristicTarget.self)
        command = .read(connectionId: target.connectionId, characteristic: target.characteristic)
      case "write":
        let target = try payload(WriteTarget.self)
        command = .write(
          connectionId: target.connectionId, characteristic: target.characteristic,
          valueBase64: target.valueBase64, writeType: target.writeType)
      case "subscribe":
        let target = try payload(CharacteristicTarget.self)
        command = .subscribe(connectionId: target.connectionId, characteristic: target.characteristic)
      case "unsubscribe":
        command = .unsubscribe(subscriptionId: try payload(SubscriptionTarget.self).subscriptionId)
      case "createServer":
        command = .createServer(try payload(ServerDefinition.self))
      case "closeServer":
        command = .closeServer(serverId: try payload(ServerTarget.self).serverId)
      case "startAdvertising":
        let target = try payload(AdvertisingTarget.self)
        command = .startAdvertising(serverId: target.serverId, options: target.options)
      case "stopAdvertising":
        command = .stopAdvertising(serverId: try payload(ServerTarget.self).serverId)
      case "setValue":
        let target = try payload(SetValueTarget.self)
        command = .setValue(
          serverId: target.serverId, characteristicKey: target.characteristicKey,
          valueBase64: target.valueBase64)
      case "notify":
        let target = try payload(NotifyTarget.self)
        command = .notify(
          serverId: target.serverId, peerId: target.peerId,
          characteristicKey: target.characteristicKey, valueBase64: target.valueBase64)
      case "cancel":
        command = .cancel(operationId: try payload(CancelTarget.self).operationId)
      case "closeOwner":
        command = .closeOwner
      case "debugResources":
        command = .debugResources
      default:
        let name = kind.isEmpty ? "an empty command kind" : kind
        return .failure(.unsupported("the CoreBluetooth backend does not implement \(name)"))
      }
    } catch {
      return .failure(.invalidArgument("malformed \(kind) payload: \(describe(error))"))
    }
    return .success(
      ExecuteRequest(
        operationId: header.operationId, ownerId: header.ownerId,
        deadlineMillis: header.deadlineMillis, command: command))
  }
}

private struct Header: Decodable {
  struct Command: Decodable {
    let kind: String
  }

  let operationId: String
  let ownerId: String
  let deadlineMillis: UInt64?
  let command: Command
}

private struct Envelope<Payload: Decodable>: Decodable {
  struct Command: Decodable {
    let payload: Payload
  }

  let command: Command
}

private struct ScanOptions: Decodable {
  let serviceUuids: [String]
  let timeoutMs: UInt64?
}

private struct ScanTarget: Decodable {
  let scanId: String
}

private struct ConnectTarget: Decodable {
  struct Options: Decodable {
    let timeoutMs: UInt64?
  }

  let deviceId: String
  let options: Options
}

private struct ConnectionTarget: Decodable {
  let connectionId: String
}

private struct CharacteristicTarget: Decodable {
  let connectionId: String
  let characteristic: String
}

private struct WriteTarget: Decodable {
  let connectionId: String
  let characteristic: String
  let valueBase64: String
  let writeType: WriteType
}

private struct SubscriptionTarget: Decodable {
  let subscriptionId: String
}

private struct ServerTarget: Decodable {
  let serverId: String
}

private struct AdvertisingTarget: Decodable {
  let serverId: String
  let options: AdvertisingOptions
}

private struct SetValueTarget: Decodable {
  let serverId: String
  let characteristicKey: String
  let valueBase64: String
}

private struct NotifyTarget: Decodable {
  let serverId: String
  let peerId: String
  let characteristicKey: String
  let valueBase64: String
}

private struct CancelTarget: Decodable {
  let operationId: String
}

private func describe(_ error: Error) -> String {
  guard let error = error as? DecodingError else { return "\(error)" }
  func path(_ keys: [CodingKey]) -> String {
    let parts = keys.map { key in key.intValue.map { "[\($0)]" } ?? key.stringValue }
    return parts.isEmpty ? "the top level" : parts.joined(separator: ".")
  }
  switch error {
  case .keyNotFound(let key, let context):
    return "missing \(path(context.codingPath + [key]))"
  case .valueNotFound(_, let context):
    return "null at \(path(context.codingPath))"
  case .typeMismatch(_, let context):
    return "wrong type at \(path(context.codingPath))"
  case .dataCorrupted(let context):
    return "invalid value at \(path(context.codingPath))"
  @unknown default:
    return "invalid JSON"
  }
}
