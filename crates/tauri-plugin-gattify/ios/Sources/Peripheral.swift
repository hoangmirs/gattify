import CoreBluetooth
import Foundation

final class RemoteCentral {
  let id: String
  var central: CBCentral

  init(id: String, central: CBCentral) {
    self.id = id
    self.central = central
  }
}

struct Subscriber {
  let central: RemoteCentral
  let maxValueLength: Int
}

final class ServerAttribute {
  let key: String
  let characteristic: CBMutableCharacteristic
  let properties: CharacteristicProperties
  let maxValueLength: Int
  var value: Data
  var subscribers: [String: Subscriber] = [:]

  init(
    key: String, characteristic: CBMutableCharacteristic, properties: CharacteristicProperties,
    maxValueLength: Int, value: Data
  ) {
    self.key = key
    self.characteristic = characteristic
    self.properties = properties
    self.maxValueLength = maxValueLength
    self.value = value
  }
}

/// The CoreBluetooth objects for a `ServerDefinition`, before any manager sees them.
struct ServerBlueprint {
  let services: [CBMutableService]
  let serviceUuids: Set<String>
  let attributes: [String: ServerAttribute]

  static func build(_ definition: ServerDefinition) -> Result<ServerBlueprint, BridgeError> {
    guard !definition.services.isEmpty else {
      return .failure(.invalidArgument("a server needs at least one service"))
    }
    var services: [CBMutableService] = []
    var serviceUuids = Set<String>()
    var attributes: [String: ServerAttribute] = [:]
    for local in definition.services {
      guard let serviceUuid = canonicalUUID(local.uuid), let type = makeCBUUID(serviceUuid) else {
        return .failure(.invalidArgument("\(local.uuid) is not a UUID"))
      }
      let service = CBMutableService(type: type, primary: local.primary)
      var members: [CBMutableCharacteristic] = []
      for spec in local.characteristics {
        let key = "\(local.instanceKey)/\(spec.instanceKey)"
        guard attributes[key] == nil else {
          return .failure(.invalidArgument("the characteristic key \(key) appears twice"))
        }
        guard let type = makeCBUUID(spec.uuid) else {
          return .failure(.invalidArgument("\(spec.uuid) is not a UUID"))
        }
        var value = Data()
        if let initial = spec.initialValueBase64 {
          guard let decoded = Data(base64Encoded: initial) else {
            return .failure(.invalidArgument("the initial value of \(key) is not valid base64"))
          }
          value = decoded
        }
        let maxValueLength = Int(spec.maxValueLength)
        guard value.count <= maxValueLength else {
          return .failure(.invalidArgument("the initial value of \(key) is longer than its maxValueLength"))
        }
        // A nil value makes CoreBluetooth ask the plugin for every read.
        let characteristic = CBMutableCharacteristic(
          type: type, properties: spec.properties.cbProperties, value: nil,
          permissions: spec.properties.cbPermissions)
        members.append(characteristic)
        attributes[key] = ServerAttribute(
          key: key, characteristic: characteristic, properties: spec.properties,
          maxValueLength: maxValueLength, value: value)
      }
      service.characteristics = members
      services.append(service)
      serviceUuids.insert(serviceUuid)
    }
    return .success(ServerBlueprint(services: services, serviceUuids: serviceUuids, attributes: attributes))
  }
}

final class ServerRecord {
  enum State {
    case registering
    case ready
    /// The adapter dropped the registration. Only `closeServer` and `stopAdvertising` still work.
    case lost
  }

  let id: String
  let owner: String
  let services: [CBMutableService]
  let serviceUuids: Set<String>
  let attributes: [String: ServerAttribute]
  var state = State.registering
  var addedServices: [CBMutableService] = []
  var remainingAdds = 0
  var addError: BridgeError?
  var createOperation: BridgeOperation?

  init(id: String, owner: String, blueprint: ServerBlueprint) {
    self.id = id
    self.owner = owner
    services = blueprint.services
    serviceUuids = blueprint.serviceUuids
    attributes = blueprint.attributes
  }
}

final class AdvertisementRecord {
  let serverId: String
  let cut: LocalNameCut
  var starting: BridgeOperation?

  init(serverId: String, cut: LocalNameCut, starting: BridgeOperation) {
    self.serverId = serverId
    self.cut = cut
    self.starting = starting
  }
}

final class PendingNotification {
  let operation: BridgeOperation
  let server: ServerRecord
  let attribute: ServerAttribute
  let central: RemoteCentral
  let value: Data

  init(
    operation: BridgeOperation, server: ServerRecord, attribute: ServerAttribute,
    central: RemoteCentral, value: Data
  ) {
    self.operation = operation
    self.server = server
    self.attribute = attribute
    self.central = central
    self.value = value
  }
}

// MARK: Commands

extension GattifyEngine {
  func createServer(_ definition: ServerDefinition, _ operation: BridgeOperation) {
    let blueprint: ServerBlueprint
    switch ServerBlueprint.build(definition) {
    case .failure(let error):
      operation.reject(error)
      return
    case .success(let built):
      blueprint = built
    }
    whenPeripheralReady(operation) { [weak self] manager in
      guard let self else { return }
      let taken = self.servers.values.filter { $0.state != .lost }
        .reduce(into: Set<String>()) { $0.formUnion($1.serviceUuids) }
      if let clash = blueprint.serviceUuids.intersection(taken).sorted().first {
        operation.reject(.busy("another server registered the service \(clash)"))
        return
      }
      let server = ServerRecord(id: self.ids.next("server"), owner: operation.owner, blueprint: blueprint)
      self.servers[server.id] = server
      for attribute in server.attributes.values {
        self.serverAttributes[ObjectIdentifier(attribute.characteristic)] = (server, attribute)
      }
      server.createOperation = operation
      server.remainingAdds = server.services.count
      operation.onAbort = { [weak self, weak server] _ in
        guard let self, let server else { return }
        self.discard(server, rejecting: .cancelled)
      }
      for service in server.services {
        self.pendingServiceAdds[ObjectIdentifier(service)] = server
        manager.add(service)
      }
    }
  }

  func closeServer(_ serverId: String, _ operation: BridgeOperation) {
    guard let server = ownedServer(serverId, operation, allowLost: true) else { return }
    discard(server, rejecting: .cancelled)
    operation.resolve(.empty)
  }

  func startAdvertising(_ serverId: String, _ options: AdvertisingOptions, _ operation: BridgeOperation) {
    guard let server = ownedServer(serverId, operation, allowLost: false) else { return }
    guard let serviceUuid = canonicalUUID(options.serviceUuid), server.serviceUuids.contains(serviceUuid),
      let type = makeCBUUID(serviceUuid)
    else {
      operation.reject(.invalidArgument("\(serverId) has no service \(options.serviceUuid)"))
      return
    }
    let cut = cutLocalName(options.localName)
    if cut.truncated && !options.localNameOptional {
      operation.reject(.payloadTooLarge("the local name is longer than \(localNameBudget) bytes"))
      return
    }
    whenPeripheralReady(operation) { [weak self] manager in
      guard let self else { return }
      if let current = self.advertisement {
        guard current.serverId == serverId, current.starting == nil else {
          operation.reject(.busy("another advertisement is running"))
          return
        }
        manager.stopAdvertising()
      }
      let record = AdvertisementRecord(serverId: serverId, cut: cut, starting: operation)
      self.advertisement = record
      operation.onAbort = { [weak self, weak record] _ in
        guard let self, let record, self.advertisement === record else { return }
        self.advertisement = nil
        manager.stopAdvertising()
      }
      var data: [String: Any] = [CBAdvertisementDataServiceUUIDsKey: [type]]
      if let name = cut.name {
        data[CBAdvertisementDataLocalNameKey] = name
      }
      manager.startAdvertising(data)
    }
  }

  func stopAdvertising(_ serverId: String, _ operation: BridgeOperation) {
    guard ownedServer(serverId, operation, allowLost: true) != nil else { return }
    if advertisement?.serverId == serverId {
      stopAdvertisement(rejecting: .cancelled)
    }
    operation.resolve(.empty)
  }

  func setValue(_ serverId: String, _ key: String, _ valueBase64: String, _ operation: BridgeOperation) {
    guard let server = ownedServer(serverId, operation, allowLost: false),
      let attribute = attribute(key, of: server, operation),
      let value = decode(valueBase64, operation)
    else { return }
    guard value.count <= attribute.maxValueLength else {
      operation.reject(.payloadTooLarge("the value is longer than the maxValueLength of \(key)"))
      return
    }
    attribute.value = value
    operation.resolve(.empty)
  }

  func notify(
    _ serverId: String, _ peerId: String, _ key: String, _ valueBase64: String,
    _ operation: BridgeOperation
  ) {
    guard let server = ownedServer(serverId, operation, allowLost: false),
      let attribute = attribute(key, of: server, operation)
    else { return }
    guard let subscriber = attribute.subscribers[peerId] else {
      operation.reject(.invalidHandle(peerId))
      return
    }
    guard let value = decode(valueBase64, operation) else { return }
    guard value.count <= subscriber.maxValueLength else {
      operation.reject(
        .payloadTooLarge("the value is longer than the notification size \(subscriber.maxValueLength) of \(peerId)"))
      return
    }
    let notification = PendingNotification(
      operation: operation, server: server, attribute: attribute, central: subscriber.central, value: value)
    operation.onAbort = { [weak self, weak notification] _ in
      self?.notifications.removeAll { $0 === notification }
    }
    notifications.append(notification)
    drainNotifications()
  }

  private func ownedServer(_ serverId: String, _ operation: BridgeOperation, allowLost: Bool) -> ServerRecord? {
    guard let server = servers[serverId], server.owner == operation.owner,
      server.state == .ready || (allowLost && server.state == .lost)
    else {
      operation.reject(.invalidHandle(serverId))
      return nil
    }
    return server
  }

  private func attribute(_ key: String, of server: ServerRecord, _ operation: BridgeOperation) -> ServerAttribute? {
    guard let attribute = server.attributes[key] else {
      operation.reject(.invalidArgument("\(server.id) has no characteristic \(key)"))
      return nil
    }
    return attribute
  }

  private func decode(_ valueBase64: String, _ operation: BridgeOperation) -> Data? {
    guard let value = Data(base64Encoded: valueBase64) else {
      operation.reject(.invalidArgument("valueBase64 is not valid base64"))
      return nil
    }
    return value
  }
}

// MARK: Server state

extension GattifyEngine {
  /// Releases a server without events: its advertisement, services, subscribers and queued notifications.
  func discard(_ server: ServerRecord, rejecting error: BridgeError) {
    guard servers[server.id] === server else { return }
    servers[server.id] = nil
    if advertisement?.serverId == server.id {
      stopAdvertisement(rejecting: error)
    }
    for attribute in server.attributes.values {
      attribute.subscribers = [:]
      serverAttributes[ObjectIdentifier(attribute.characteristic)] = nil
    }
    for service in server.addedServices {
      peripheralManager?.remove(service)
    }
    server.addedServices = []
    rejectNotifications(error) { $0.server === server }
  }

  func stopAdvertisement(rejecting error: BridgeError) {
    guard let current = advertisement else { return }
    advertisement = nil
    peripheralManager?.stopAdvertising()
    current.starting?.reject(error)
  }

  func rejectNotifications(_ error: BridgeError, where matches: (PendingNotification) -> Bool) {
    let dropped = notifications.filter(matches)
    guard !dropped.isEmpty else { return }
    notifications.removeAll(where: matches)
    dropped.forEach { $0.operation.reject(error) }
  }

  /// Sends queued notifications in order until the transmit queue is full. This is the flow
  /// control for the host: a notify resolves only when CoreBluetooth takes the value.
  func drainNotifications() {
    guard let manager = peripheralManager else { return }
    while let next = notifications.first {
      guard !next.operation.isFinished else {
        notifications.removeFirst()
        continue
      }
      guard servers[next.server.id] === next.server, next.server.state == .ready,
        next.attribute.subscribers[next.central.id] != nil
      else {
        notifications.removeFirst()
        next.operation.reject(.disconnected("the central unsubscribed"))
        continue
      }
      guard manager.updateValue(next.value, for: next.attribute.characteristic, onSubscribedCentrals: [next.central.central])
      else { return }
      notifications.removeFirst()
      next.operation.resolve(.empty)
    }
  }

  func remoteCentral(_ central: CBCentral) -> RemoteCentral {
    if let known = centrals[central.identifier] {
      known.central = central
      return known
    }
    let remote = RemoteCentral(id: ids.next("central"), central: central)
    centrals[central.identifier] = remote
    return remote
  }

  func liveAttribute(_ characteristic: CBCharacteristic) -> (server: ServerRecord, attribute: ServerAttribute)? {
    guard let entry = serverAttributes[ObjectIdentifier(characteristic)], entry.server.state == .ready else {
      return nil
    }
    return entry
  }
}

// MARK: CBPeripheralManagerDelegate

extension GattifyEngine: CBPeripheralManagerDelegate {
  func peripheralManagerDidUpdateState(_ manager: CBPeripheralManager) {
    let state = manager.state
    peripheralHasState = true
    noteAdapterState(state)
    if let error = BridgeError.adapter(state) {
      if let current = advertisement {
        advertisement = nil
        current.starting?.reject(error)
      }
      rejectNotifications(error) { _ in true }
      for server in Array(servers.values) {
        loseRegistration(server, error)
      }
    }
    let waiters = peripheralWaiters
    peripheralWaiters = []
    peripheralWaiters = waiters.filter { !$0(state) } + peripheralWaiters
  }

  /// The adapter removed every service. A registering server fails. A ready server reports
  /// each subscriber as gone, then `criticalStateLoss`.
  private func loseRegistration(_ server: ServerRecord, _ error: BridgeError) {
    server.addedServices = []
    pendingServiceAdds = pendingServiceAdds.filter { $0.value !== server }
    switch server.state {
    case .registering:
      let operation = server.createOperation
      server.createOperation = nil
      discard(server, rejecting: error)
      operation?.reject(error)
    case .ready:
      server.state = .lost
      for attribute in server.attributes.values.sorted(by: { $0.key < $1.key }) {
        for peerId in attribute.subscribers.keys.sorted() {
          emit(
            server.owner,
            .subscriptionChanged(
              serverId: server.id, peerId: peerId, characteristicKey: attribute.key, subscribed: false,
              maxValueLength: nil))
        }
        attribute.subscribers = [:]
        serverAttributes[ObjectIdentifier(attribute.characteristic)] = nil
      }
      emit(server.owner, .criticalStateLoss(resourceId: server.id, reason: error.code))
    case .lost:
      break
    }
  }

  func peripheralManager(_ manager: CBPeripheralManager, didAdd service: CBService, error: Error?) {
    guard let server = pendingServiceAdds.removeValue(forKey: ObjectIdentifier(service)),
      let added = server.services.first(where: { $0 === service })
    else { return }
    guard servers[server.id] === server, server.state == .registering else {
      if error == nil {
        manager.remove(added)
      }
      return
    }
    if let error {
      server.addError = server.addError ?? BridgeError(error)
    } else {
      server.addedServices.append(added)
    }
    server.remainingAdds -= 1
    guard server.remainingAdds == 0 else { return }
    let operation = server.createOperation
    server.createOperation = nil
    if let failure = server.addError {
      discard(server, rejecting: .cancelled)
      operation?.reject(failure)
    } else {
      server.state = .ready
      operation?.resolve(.serverCreated(serverId: server.id))
    }
  }

  func peripheralManagerDidStartAdvertising(_ manager: CBPeripheralManager, error: Error?) {
    guard let record = advertisement, let operation = record.starting else { return }
    record.starting = nil
    if let error {
      advertisement = nil
      operation.reject(BridgeError(error))
    } else {
      operation.resolve(
        .advertisingStarted(localNameIncluded: record.cut.included, localNameTruncated: record.cut.truncated))
    }
  }

  func peripheralManager(
    _ manager: CBPeripheralManager, central: CBCentral, didSubscribeTo characteristic: CBCharacteristic
  ) {
    guard let (server, attribute) = liveAttribute(characteristic) else { return }
    let remote = remoteCentral(central)
    let size = clampedValueLength(central.maximumUpdateValueLength)
    attribute.subscribers[remote.id] = Subscriber(central: remote, maxValueLength: size)
    emit(
      server.owner,
      .subscriptionChanged(
        serverId: server.id, peerId: remote.id, characteristicKey: attribute.key, subscribed: true,
        maxValueLength: size))
  }

  func peripheralManager(
    _ manager: CBPeripheralManager, central: CBCentral, didUnsubscribeFrom characteristic: CBCharacteristic
  ) {
    guard let (server, attribute) = liveAttribute(characteristic), let remote = centrals[central.identifier],
      attribute.subscribers.removeValue(forKey: remote.id) != nil
    else { return }
    rejectNotifications(.disconnected("the central unsubscribed")) {
      $0.attribute === attribute && $0.central === remote
    }
    emit(
      server.owner,
      .subscriptionChanged(
        serverId: server.id, peerId: remote.id, characteristicKey: attribute.key, subscribed: false,
        maxValueLength: nil))
  }

  func peripheralManager(_ manager: CBPeripheralManager, didReceiveRead request: CBATTRequest) {
    guard let (_, attribute) = liveAttribute(request.characteristic) else {
      manager.respond(to: request, withResult: .attributeNotFound)
      return
    }
    _ = remoteCentral(request.central)
    guard attribute.properties.read else {
      manager.respond(to: request, withResult: .readNotPermitted)
      return
    }
    guard let value = readSlice(attribute.value, offset: request.offset) else {
      manager.respond(to: request, withResult: .invalidOffset)
      return
    }
    request.value = value
    manager.respond(to: request, withResult: .success)
  }

  /// The batch succeeds or fails together, and CoreBluetooth takes one answer, for the first request.
  func peripheralManager(_ manager: CBPeripheralManager, didReceiveWrite requests: [CBATTRequest]) {
    guard let first = requests.first else { return }
    var targets: [(server: ServerRecord, attribute: ServerAttribute, central: RemoteCentral)] = []
    var slots: [String: Int] = [:]
    var parts: [WritePart] = []
    for request in requests {
      let remote = remoteCentral(request.central)
      let value = request.value ?? Data()
      guard let (server, attribute) = liveAttribute(request.characteristic) else {
        parts.append(WritePart(target: nil, writable: false, maxValueLength: 0, offset: request.offset, value: value))
        continue
      }
      let slotKey = "\(server.id)|\(attribute.key)|\(remote.id)"
      let target: Int
      if let existing = slots[slotKey] {
        target = existing
      } else {
        target = targets.count
        slots[slotKey] = target
        targets.append((server, attribute, remote))
      }
      parts.append(
        WritePart(
          target: target, writable: attribute.properties.writable,
          maxValueLength: attribute.maxValueLength, offset: request.offset, value: value))
    }
    switch assembleWrites(parts) {
    case .rejected(let code):
      manager.respond(to: first, withResult: code)
    case .accepted(let writes):
      manager.respond(to: first, withResult: .success)
      for write in writes {
        let (server, attribute, remote) = targets[write.target]
        emit(
          server.owner,
          .serverWrite(serverId: server.id, peerId: remote.id, characteristicKey: attribute.key, value: write.value))
      }
    }
  }

  func peripheralManagerIsReady(toUpdateSubscribers manager: CBPeripheralManager) {
    drainNotifications()
  }
}
