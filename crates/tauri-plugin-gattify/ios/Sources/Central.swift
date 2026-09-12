import CoreBluetooth
import Foundation

/// The value length of a link before its MTU exchange: an ATT MTU of 23 minus 3.
let minimumAttributeValueLength = 20
let linkLimitWaitMilliseconds: UInt64 = 1_000
let linkLimitPollMilliseconds: UInt64 = 50

final class DeviceRecord {
  let id: String
  /// iOS invalidates its peripheral objects when the adapter resets, so discovery refreshes this.
  var peripheral: CBPeripheral
  /// The owner families that received this device in a scan result.
  var families: Set<String> = []
  var connection: ConnectionRecord?
  /// Runs when the device has no connection again.
  var whenFree: [() -> Void] = []

  init(id: String, peripheral: CBPeripheral) {
    self.id = id
    self.peripheral = peripheral
  }
}

final class ScanRecord {
  let id: String
  let owner: String
  let filter: Set<String>
  var throttle = ResultThrottle()
  var timer: DispatchWorkItem?

  init(id: String, owner: String, filter: Set<String>) {
    self.id = id
    self.owner = owner
    self.filter = filter
  }
}

final class SubscriptionRecord {
  let id: String
  let owner: String
  let connectionId: String
  let handle: String
  let characteristic: CBCharacteristic

  init(id: String, owner: String, connectionId: String, handle: String, characteristic: CBCharacteristic) {
    self.id = id
    self.owner = owner
    self.connectionId = connectionId
    self.handle = handle
    self.characteristic = characteristic
  }
}

final class GattRequest {
  enum Kind {
    case discover
    case read(CBCharacteristic)
    case write(CBCharacteristic, Data, CBCharacteristicWriteType)
    case subscribe(CBCharacteristic, handle: String)
    case unsubscribe(CBCharacteristic)
  }

  let operation: BridgeOperation
  let kind: Kind
  /// A CoreBluetooth call is out, and its callback has not arrived.
  var awaitingCallback = false
  /// The operation ended early. The request keeps the queue until its late callback arrives.
  var abandoned = false
  var abandonTimer: DispatchWorkItem?
  var remainingDiscoveries = 0
  var discoveryError: BridgeError?

  init(_ operation: BridgeOperation, _ kind: Kind) {
    self.operation = operation
    self.kind = kind
  }

  var isDiscovery: Bool {
    if case .discover = kind { return true }
    return false
  }

  func isRead(of characteristic: CBCharacteristic) -> Bool {
    if case .read(let target) = kind { return target === characteristic }
    return false
  }

  func isWriteWithResponse(to characteristic: CBCharacteristic) -> Bool {
    if case .write(let target, _, .withResponse) = kind { return target === characteristic }
    return false
  }

  func isNotifyChange(of characteristic: CBCharacteristic) -> Bool {
    switch kind {
    case .subscribe(let target, _), .unsubscribe(let target):
      return target === characteristic
    default:
      return false
    }
  }
}

final class ConnectionRecord {
  enum State {
    case connecting
    case connected
    case closing
  }

  let id: String
  let owner: String
  let device: DeviceRecord
  var state = State.connecting
  var connectOperation: BridgeOperation?
  /// iOS reported the connection. The connect still waits for the MTU exchange.
  var linkUp = false
  var closeStarted = false
  var closeTimer: DispatchWorkItem?
  var releaseTimer: DispatchWorkItem?
  var closeWaiters: [() -> Void] = []
  var characteristics: [String: CBCharacteristic] = [:]
  var subscriptions: [String: SubscriptionRecord] = [:]
  var queue: [GattRequest] = []
  var current: GattRequest?
  var pumping = false
  private var handles: [String: String] = [:]
  private var handleIds = IDAllocator()

  init(id: String, owner: String, device: DeviceRecord) {
    self.id = id
    self.owner = owner
    self.device = device
  }

  var peripheral: CBPeripheral { device.peripheral }

  /// The same attribute key gets the same handle on every discovery.
  func handle(_ kind: String, key: String) -> String {
    let slot = "\(kind):\(key)"
    if let handle = handles[slot] { return handle }
    let handle = "\(id)/\(handleIds.next(kind))"
    handles[slot] = handle
    return handle
  }
}

// MARK: Commands

extension GattifyEngine {
  func startScan(_ serviceUuids: [String], timeoutMs: UInt64?, _ operation: BridgeOperation) {
    var filter = Set<String>()
    for uuid in serviceUuids {
      guard let canonical = canonicalUUID(uuid) else {
        operation.reject(.invalidArgument("\(uuid) is not a UUID"))
        return
      }
      filter.insert(canonical)
    }
    whenCentralReady(operation) { [weak self] _ in
      guard let self else { return }
      let scanId = self.ids.next("scan")
      let scan = ScanRecord(id: scanId, owner: operation.owner, filter: filter)
      self.scans[scanId] = scan
      if let timeoutMs {
        scan.timer = self.schedule(after: timeoutMs) { [weak self] in
          self?.endScan(scanId, notify: true)
        }
      }
      self.updateScan()
      operation.resolve(.scanStarted(scanId: scanId))
    }
  }

  func stopScan(_ scanId: String, _ operation: BridgeOperation) {
    guard let scan = scans[scanId], scan.owner == operation.owner else {
      operation.reject(.invalidHandle(scanId))
      return
    }
    endScan(scan.id, notify: false)
    operation.resolve(.empty)
  }

  func endScan(_ scanId: String, notify: Bool) {
    guard let scan = scans.removeValue(forKey: scanId) else { return }
    scan.timer?.cancel()
    updateScan()
    if notify {
      emit(scan.owner, .scanStopped(scanId: scan.id))
    }
  }

  /// Runs one CoreBluetooth scan with the union of the filters of every scan.
  func updateScan() {
    guard let central, central.state == .poweredOn else { return }
    let plan = ScanPlan(filters: scans.values.map(\.filter))
    guard plan != scanPlan else { return }
    scanPlan = plan
    let options = [CBCentralManagerScanOptionAllowDuplicatesKey: true]
    switch plan {
    case .stopped:
      central.stopScan()
    case .everyDevice:
      central.scanForPeripherals(withServices: nil, options: options)
    case .services(let uuids):
      central.scanForPeripherals(withServices: uuids.compactMap(makeCBUUID), options: options)
    }
  }

  func connect(_ deviceId: String, _ operation: BridgeOperation) {
    guard !operation.isFinished else { return }
    guard let device = devicesById[deviceId], device.families.contains(ownerFamily(operation.owner))
    else {
      operation.reject(.invalidHandle(deviceId))
      return
    }
    if let existing = device.connection {
      guard existing.state == .closing else {
        operation.reject(.busy("\(deviceId) already has a connection"))
        return
      }
      device.whenFree.append { [weak self] in self?.connect(deviceId, operation) }
      return
    }
    whenCentralReady(operation) { [weak self] central in
      guard let self else { return }
      guard device.connection == nil else {
        operation.reject(.busy("\(deviceId) already has a connection"))
        return
      }
      let link = ConnectionRecord(id: self.ids.next("connection"), owner: operation.owner, device: device)
      link.connectOperation = operation
      device.connection = link
      self.connections[link.id] = link
      operation.onAbort = { [weak self, weak link] _ in
        guard let self, let link, link.state == .connecting else { return }
        link.connectOperation = nil
        // The attempt stays on the device until iOS reports its end, so its late callback
        // cannot answer a new attempt.
        self.closeLink(link, rejecting: .cancelled, notify: false)
      }
      device.peripheral.delegate = self
      central.connect(device.peripheral, options: nil)
    }
  }

  func disconnect(_ connectionId: String, _ operation: BridgeOperation) {
    guard let link = ownedLink(connectionId, operation) else { return }
    closeLink(link, rejecting: .disconnected("the connection was closed"), notify: false) {
      operation.resolve(.empty)
    }
  }

  func discoverServices(_ connectionId: String, _ operation: BridgeOperation) {
    guard let link = ownedLink(connectionId, operation) else { return }
    enqueue(GattRequest(operation, .discover), on: link)
  }

  func read(_ connectionId: String, _ handle: String, _ operation: BridgeOperation) {
    guard let link = ownedLink(connectionId, operation),
      let characteristic = characteristic(handle, on: link, operation)
    else { return }
    guard link.subscriptions[handle] == nil else {
      operation.reject(readWhileSubscribed(handle))
      return
    }
    enqueue(GattRequest(operation, .read(characteristic)), on: link)
  }

  func write(
    _ connectionId: String, _ handle: String, _ valueBase64: String, _ writeType: WriteType,
    _ operation: BridgeOperation
  ) {
    guard let link = ownedLink(connectionId, operation),
      let characteristic = characteristic(handle, on: link, operation)
    else { return }
    guard let value = Data(base64Encoded: valueBase64) else {
      operation.reject(.invalidArgument("valueBase64 is not valid base64"))
      return
    }
    switch writeType {
    case .withResponse:
      guard value.count <= maxAttributeValueLength else {
        operation.reject(.payloadTooLarge("a write is longer than \(maxAttributeValueLength) bytes"))
        return
      }
      enqueue(GattRequest(operation, .write(characteristic, value, .withResponse)), on: link)
    case .withoutResponse:
      let limit = clampedValueLength(link.peripheral.maximumWriteValueLength(for: .withoutResponse))
      guard value.count <= limit else {
        operation.reject(.payloadTooLarge("a write without response is longer than \(limit) bytes"))
        return
      }
      enqueue(GattRequest(operation, .write(characteristic, value, .withoutResponse)), on: link)
    }
  }

  func subscribe(_ connectionId: String, _ handle: String, _ operation: BridgeOperation) {
    guard let link = ownedLink(connectionId, operation),
      let characteristic = characteristic(handle, on: link, operation)
    else { return }
    guard CharacteristicProperties(characteristic.properties).notifiable else {
      operation.reject(.unsupported("\(handle) has neither notify nor indicate"))
      return
    }
    guard link.subscriptions[handle] == nil else {
      operation.reject(.busy("\(handle) already has a subscription"))
      return
    }
    enqueue(GattRequest(operation, .subscribe(characteristic, handle: handle)), on: link)
  }

  func unsubscribe(_ subscriptionId: String, _ operation: BridgeOperation) {
    guard let subscription = subscriptions[subscriptionId], subscription.owner == operation.owner,
      let link = connections[subscription.connectionId], link.state == .connected
    else {
      operation.reject(.invalidHandle(subscriptionId))
      return
    }
    subscriptions[subscriptionId] = nil
    link.subscriptions[subscription.handle] = nil
    enqueue(GattRequest(operation, .unsubscribe(subscription.characteristic)), on: link)
  }

  private func ownedLink(_ connectionId: String, _ operation: BridgeOperation) -> ConnectionRecord? {
    guard let link = connections[connectionId], link.owner == operation.owner, link.state == .connected
    else {
      operation.reject(.invalidHandle(connectionId))
      return nil
    }
    return link
  }

  private func characteristic(
    _ handle: String, on link: ConnectionRecord, _ operation: BridgeOperation
  ) -> CBCharacteristic? {
    guard let characteristic = link.characteristics[handle] else {
      operation.reject(.invalidHandle(handle))
      return nil
    }
    return characteristic
  }

  private func readWhileSubscribed(_ handle: String) -> BridgeError {
    .busy("\(handle) has a subscription, and iOS cannot tell a read from a notification")
  }
}

// MARK: Links

extension GattifyEngine {
  /// Ends a connection: rejects its procedures and ends its subscriptions without events, then
  /// closes the link. `linkDown` means the link is already gone.
  func closeLink(
    _ link: ConnectionRecord, rejecting error: BridgeError, notify: Bool, linkDown: Bool = false,
    whenClosed: (() -> Void)? = nil
  ) {
    if let whenClosed {
      link.closeWaiters.append(whenClosed)
    }
    if !link.closeStarted {
      link.closeStarted = true
      link.state = .closing
      let requests = (link.current.map { [$0] } ?? []) + link.queue
      link.current = nil
      link.queue = []
      for request in requests {
        request.abandonTimer?.cancel()
        request.operation.reject(error)
      }
      for subscription in link.subscriptions.values {
        subscriptions[subscription.id] = nil
      }
      link.subscriptions = [:]
      if notify {
        emit(link.owner, .connectionClosed(connectionId: link.id))
      }
      if !linkDown {
        central?.cancelPeripheralConnection(link.peripheral)
        // The caller hears back within 2 s. The device stays reserved until iOS confirms the
        // close, so a late callback of this link never reaches the next one.
        link.closeTimer = schedule(after: 2_000) { [weak self, weak link] in
          if let link { self?.answerClose(link) }
        }
        link.releaseTimer = schedule(after: 10_000) { [weak self, weak link] in
          if let link { self?.finishClose(link) }
        }
      }
    }
    if linkDown {
      finishClose(link)
    }
  }

  /// Answers the callers waiting for the close, without releasing the device.
  func answerClose(_ link: ConnectionRecord) {
    let waiters = link.closeWaiters
    link.closeWaiters = []
    waiters.forEach { $0() }
  }

  func finishClose(_ link: ConnectionRecord) {
    link.closeTimer?.cancel()
    link.releaseTimer?.cancel()
    guard connections[link.id] === link || link.device.connection === link else { return }
    answerClose(link)
    detach(link)
  }

  /// Forgets a link, then lets a waiting connect to the same device run.
  func detach(_ link: ConnectionRecord) {
    if connections[link.id] === link {
      connections[link.id] = nil
    }
    guard link.device.connection === link else { return }
    link.device.connection = nil
    let resumes = link.device.whenFree
    link.device.whenFree = []
    resumes.forEach { $0() }
  }

  func deviceRecord(for peripheral: CBPeripheral) -> DeviceRecord {
    if let device = devices[peripheral.identifier] {
      if device.connection == nil {
        device.peripheral = peripheral
      }
      return device
    }
    let device = DeviceRecord(id: ids.next("device"), peripheral: peripheral)
    devices[peripheral.identifier] = device
    devicesById[device.id] = device
    return device
  }

  /// The link to `peripheral` while it is connected.
  func liveLink(_ peripheral: CBPeripheral) -> ConnectionRecord? {
    guard let link = devices[peripheral.identifier]?.connection, link.state == .connected else {
      return nil
    }
    return link
  }
}

// MARK: GATT procedures

extension GattifyEngine {
  func enqueue(_ request: GattRequest, on link: ConnectionRecord) {
    request.operation.onAbort = { [weak self, weak link, weak request] error in
      guard let self, let link, let request else { return }
      self.abandon(request, on: link, error)
    }
    link.queue.append(request)
    pump(link)
  }

  /// Starts queued procedures, one at a time.
  func pump(_ link: ConnectionRecord) {
    guard !link.pumping else { return }
    link.pumping = true
    defer { link.pumping = false }
    while link.current == nil, link.state == .connected, !link.queue.isEmpty {
      let request = link.queue.removeFirst()
      guard !request.operation.isFinished else { continue }
      link.current = request
      start(request, on: link)
    }
  }

  func complete(_ request: GattRequest, on link: ConnectionRecord, _ result: Result<Reply, BridgeError>) {
    guard link.current === request else { return }
    link.current = nil
    request.abandonTimer?.cancel()
    request.operation.answer(result)
    pump(link)
  }

  /// A queued request just leaves the queue. A request whose callback is still out keeps the
  /// queue until the callback arrives, so the callback cannot answer a later request. At the
  /// deadline, the link closes instead.
  func abandon(_ request: GattRequest, on link: ConnectionRecord, _ error: BridgeError) {
    guard link.current === request else {
      link.queue.removeAll { $0 === request }
      return
    }
    guard request.awaitingCallback else {
      link.current = nil
      pump(link)
      return
    }
    guard error != .timeout else {
      closeAfterTimeout(link)
      return
    }
    request.abandoned = true
    let deadline = request.operation.deadline ?? .now() + .milliseconds(5_000)
    request.abandonTimer = schedule(at: deadline) { [weak self, weak link, weak request] in
      guard let self, let link, let request, link.current === request else { return }
      self.closeAfterTimeout(link)
    }
  }

  func closeAfterTimeout(_ link: ConnectionRecord) {
    closeLink(link, rejecting: .disconnected("the link closed after a procedure timed out"), notify: true)
  }

  func start(_ request: GattRequest, on link: ConnectionRecord) {
    let peripheral = link.peripheral
    switch request.kind {
    case .discover:
      request.awaitingCallback = true
      peripheral.discoverServices(nil)
    case .read(let characteristic):
      let subscribed = link.subscriptions.values.contains { $0.characteristic === characteristic }
      guard !subscribed, !characteristic.isNotifying else {
        let handle = link.characteristics.first { $0.value === characteristic }?.key ?? "the characteristic"
        complete(request, on: link, .failure(readWhileSubscribed(handle)))
        return
      }
      request.awaitingCallback = true
      peripheral.readValue(for: characteristic)
    case .write(let characteristic, let value, let type):
      if type == .withResponse {
        request.awaitingCallback = true
        peripheral.writeValue(value, for: characteristic, type: .withResponse)
      } else if peripheral.canSendWriteWithoutResponse {
        peripheral.writeValue(value, for: characteristic, type: .withoutResponse)
        complete(request, on: link, .success(.empty))
      }
    case .subscribe(let characteristic, let handle):
      if link.subscriptions[handle] != nil {
        complete(request, on: link, .failure(.busy("\(handle) already has a subscription")))
      } else if characteristic.isNotifying {
        let subscriptionId = addSubscription(on: link, handle: handle, characteristic: characteristic)
        complete(request, on: link, .success(.subscriptionStarted(subscriptionId: subscriptionId)))
      } else {
        request.awaitingCallback = true
        peripheral.setNotifyValue(true, for: characteristic)
      }
    case .unsubscribe(let characteristic):
      if characteristic.isNotifying {
        request.awaitingCallback = true
        peripheral.setNotifyValue(false, for: characteristic)
      } else {
        complete(request, on: link, .success(.empty))
      }
    }
  }

  func addSubscription(on link: ConnectionRecord, handle: String, characteristic: CBCharacteristic) -> String {
    let subscription = SubscriptionRecord(
      id: ids.next("subscription"), owner: link.owner, connectionId: link.id, handle: handle,
      characteristic: characteristic)
    subscriptions[subscription.id] = subscription
    link.subscriptions[handle] = subscription
    return subscription.id
  }

  /// The discovered services with stable handles. Refreshes the handle table of the link.
  func catalog(_ link: ConnectionRecord) -> [ServiceInstance] {
    let services = link.peripheral.services ?? []
    var table: [String: CBCharacteristic] = [:]
    let serviceKeys = occurrenceKeys(services.map { canonicalUUID($0.uuid) })
    let instances = zip(services, serviceKeys).map { service, serviceKey -> ServiceInstance in
      let members = service.characteristics ?? []
      let memberKeys = occurrenceKeys(members.map { canonicalUUID($0.uuid) })
      let characteristics = zip(members, memberKeys).map { characteristic, key -> CharacteristicInstance in
        let handle = link.handle("characteristic", key: "\(serviceKey)/\(key)")
        table[handle] = characteristic
        return CharacteristicInstance(
          handle: handle, uuid: canonicalUUID(characteristic.uuid),
          properties: CharacteristicProperties(characteristic.properties))
      }
      return ServiceInstance(
        handle: link.handle("service", key: serviceKey), uuid: canonicalUUID(service.uuid),
        characteristics: characteristics)
    }
    link.characteristics = table
    return instances
  }

  /// The in-flight request on a live link to `peripheral`, when `matches` accepts it.
  func awaitingRequest(
    _ peripheral: CBPeripheral, _ matches: (GattRequest) -> Bool
  ) -> (link: ConnectionRecord, request: GattRequest)? {
    guard let link = liveLink(peripheral), let request = link.current, request.awaitingCallback,
      matches(request)
    else { return nil }
    return (link, request)
  }
}

// MARK: CBCentralManagerDelegate

extension GattifyEngine: CBCentralManagerDelegate {
  func centralManagerDidUpdateState(_ central: CBCentralManager) {
    let state = central.state
    centralHasState = true
    noteAdapterState(state)
    if let error = BridgeError.adapter(state) {
      scanPlan = .stopped
      for scan in Array(scans.values) {
        endScan(scan.id, notify: true)
      }
      for link in Array(connections.values) {
        switch link.state {
        case .connecting:
          let operation = link.connectOperation
          detach(link)
          operation?.reject(error)
        case .connected:
          closeLink(link, rejecting: .disconnected("Bluetooth stopped"), notify: true, linkDown: true)
        case .closing:
          finishClose(link)
        }
      }
    } else {
      updateScan()
    }
    let waiters = centralWaiters
    centralWaiters = []
    centralWaiters = waiters.filter { !$0(state) } + centralWaiters
  }

  func centralManager(
    _ central: CBCentralManager, didDiscover peripheral: CBPeripheral,
    advertisementData: [String: Any], rssi RSSI: NSNumber
  ) {
    let advertisement = Advertisement(advertisementData)
    let matching = scans.values.filter { scanFilterMatches($0.filter, advertisement.matchUuids) }
    guard !matching.isEmpty else { return }
    let device = deviceRecord(for: peripheral)
    let now = ProcessInfo.processInfo.systemUptime
    let observedAt = Int((Date().timeIntervalSince1970 * 1000).rounded())
    let rssi = reportedRSSI(RSSI)
    for scan in matching {
      guard scan.throttle.allow(device.id, at: now) else { continue }
      device.families.insert(ownerFamily(scan.owner))
      let result = DiscoveredDevice(
        id: device.id,
        name: deviceName(advertisement.data, filter: scan.filter, cachedName: peripheral.name),
        rssi: rssi, serviceUuids: advertisement.serviceUuids, advertisement: advertisement.data,
        observedAtMillis: observedAt, scanId: scan.id)
      emit(scan.owner, .scanResult(result))
    }
  }

  func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
    guard let device = devices[peripheral.identifier], let link = device.connection else {
      central.cancelPeripheralConnection(peripheral)
      return
    }
    guard link.state == .connecting, let operation = link.connectOperation else {
      if link.state == .closing {
        // A cancelled attempt connected anyway.
        central.cancelPeripheralConnection(peripheral)
      }
      return
    }
    link.linkUp = true
    awaitLinkLimits(link, operation, waited: 0)
  }

  /// iOS negotiates the MTU after `didConnect`, and until then reports 20-byte values. Frames
  /// that small make a 4 KiB message take seconds, so the connect waits up to 1 s for the
  /// negotiated length, as the Android connect waits for its MTU reply.
  func awaitLinkLimits(_ link: ConnectionRecord, _ operation: BridgeOperation, waited: UInt64) {
    guard !operation.isFinished, link.state == .connecting, connections[link.id] === link else {
      return
    }
    let length = link.peripheral.maximumWriteValueLength(for: .withoutResponse)
    if length <= minimumAttributeValueLength && waited < linkLimitWaitMilliseconds {
      schedule(after: linkLimitPollMilliseconds) { [weak self, weak link] in
        guard let self, let link else { return }
        self.awaitLinkLimits(link, operation, waited: waited + linkLimitPollMilliseconds)
      }
      return
    }
    link.state = .connected
    link.connectOperation = nil
    operation.resolve(
      .connected(connectionId: link.id, limits: LinkLimits(maximumWriteValueLength: length)))
  }

  func centralManager(
    _ central: CBCentralManager, didFailToConnect peripheral: CBPeripheral, error: Error?
  ) {
    guard let link = devices[peripheral.identifier]?.connection else { return }
    if link.state == .closing {
      finishClose(link)
      return
    }
    guard link.state == .connecting else { return }
    let operation = link.connectOperation
    detach(link)
    operation?.reject(
      error.map { BridgeError($0) }
        ?? BridgeError(code: "internal", message: "the connection attempt failed"))
  }

  func centralManager(
    _ central: CBCentralManager, didDisconnectPeripheral peripheral: CBPeripheral, error: Error?
  ) {
    guard let link = devices[peripheral.identifier]?.connection else { return }
    switch link.state {
    case .connecting:
      guard link.linkUp else {
        // The late callback of an attempt that was cancelled before this one started.
        return
      }
      // The link dropped while the connect waited for the MTU exchange.
      let operation = link.connectOperation
      detach(link)
      operation?.reject(.disconnected("the link closed before the connection was ready"))
    case .connected:
      closeLink(link, rejecting: .disconnected("the link closed"), notify: true, linkDown: true)
    case .closing:
      finishClose(link)
    }
  }
}

// MARK: CBPeripheralDelegate

extension GattifyEngine: CBPeripheralDelegate {
  func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
    guard let (link, request) = awaitingRequest(peripheral, \.isDiscovery) else { return }
    if let error {
      complete(request, on: link, .failure(BridgeError(error)))
      return
    }
    let services = peripheral.services ?? []
    guard !services.isEmpty else {
      complete(request, on: link, .success(.services(catalog(link))))
      return
    }
    request.remainingDiscoveries = services.count
    for service in services {
      peripheral.discoverCharacteristics(nil, for: service)
    }
  }

  func peripheral(
    _ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?
  ) {
    guard let (link, request) = awaitingRequest(peripheral, \.isDiscovery),
      request.remainingDiscoveries > 0
    else { return }
    if let error, request.discoveryError == nil {
      request.discoveryError = BridgeError(error)
    }
    request.remainingDiscoveries -= 1
    guard request.remainingDiscoveries == 0 else { return }
    if let failure = request.discoveryError {
      complete(request, on: link, .failure(failure))
    } else {
      complete(request, on: link, .success(.services(catalog(link))))
    }
  }

  func peripheral(
    _ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?
  ) {
    if let (link, request) = awaitingRequest(peripheral, { $0.isRead(of: characteristic) }) {
      let value = characteristic.value ?? Data()
      complete(request, on: link, error.map { .failure(BridgeError($0)) } ?? .success(.bytes(value)))
      return
    }
    guard error == nil, let link = liveLink(peripheral),
      let subscription = link.subscriptions.values.first(where: { $0.characteristic === characteristic })
    else { return }
    emit(
      subscription.owner,
      .characteristicValue(subscriptionId: subscription.id, value: characteristic.value ?? Data()))
  }

  func peripheral(
    _ peripheral: CBPeripheral, didWriteValueFor characteristic: CBCharacteristic, error: Error?
  ) {
    guard let (link, request) = awaitingRequest(peripheral, { $0.isWriteWithResponse(to: characteristic) })
    else { return }
    complete(request, on: link, error.map { .failure(BridgeError($0)) } ?? .success(.empty))
  }

  func peripheral(
    _ peripheral: CBPeripheral, didUpdateNotificationStateFor characteristic: CBCharacteristic,
    error: Error?
  ) {
    guard let (link, request) = awaitingRequest(peripheral, { $0.isNotifyChange(of: characteristic) })
    else { return }
    if let error {
      complete(request, on: link, .failure(BridgeError(error)))
      return
    }
    guard case .subscribe(_, let handle) = request.kind, !request.abandoned else {
      complete(request, on: link, .success(.empty))
      return
    }
    let subscriptionId = addSubscription(on: link, handle: handle, characteristic: characteristic)
    complete(request, on: link, .success(.subscriptionStarted(subscriptionId: subscriptionId)))
  }

  func peripheralIsReady(toSendWriteWithoutResponse peripheral: CBPeripheral) {
    guard let link = liveLink(peripheral), let request = link.current, !request.awaitingCallback,
      case .write(let characteristic, let value, .withoutResponse) = request.kind
    else { return }
    peripheral.writeValue(value, for: characteristic, type: .withoutResponse)
    complete(request, on: link, .success(.empty))
  }

  /// A remote that re-registers a service kills its subscriptions without a disconnect. When a
  /// service in use goes away, the link closes, so the caller can dial again.
  func peripheral(_ peripheral: CBPeripheral, didModifyServices invalidatedServices: [CBService]) {
    guard let link = liveLink(peripheral) else { return }
    let invalidated = { (characteristic: CBCharacteristic) in
      invalidatedServices.contains { $0 === characteristic.service }
    }
    let inUse = link.subscriptions.values.contains { invalidated($0.characteristic) }
      || link.current.map { request in requestTouches(request, invalidated) } == true
    if inUse {
      closeLink(link, rejecting: .disconnected("the remote changed its services"), notify: true)
      return
    }
    link.characteristics = link.characteristics.filter { !invalidated($0.value) }
  }

  private func requestTouches(_ request: GattRequest, _ invalidated: (CBCharacteristic) -> Bool) -> Bool {
    switch request.kind {
    case .discover:
      return false
    case .read(let characteristic), .write(let characteristic, _, _), .subscribe(let characteristic, _),
      .unsubscribe(let characteristic):
      return invalidated(characteristic)
    }
  }
}
