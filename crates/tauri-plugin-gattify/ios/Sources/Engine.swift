import CoreBluetooth
import Foundation

typealias EventHandler = (_ ownerId: String, _ event: BridgeEvent) -> Void

/// The CoreBluetooth backend of the native bridge, on iOS and on macOS. Every method runs on
/// `queue`, which is also the delegate queue of both CoreBluetooth managers.
final class GattifyEngine: NSObject {
  let queue: DispatchQueue
  let authorization: () -> Authorization
  let emit: EventHandler

  var ids = IDAllocator()
  var operations: [String: BridgeOperation] = [:]
  var lastAdapterState: String?

  var central: CBCentralManager?
  var centralHasState = false
  /// Each waiter returns true when it is done.
  var centralWaiters: [(CBManagerState) -> Bool] = []
  var devices: [UUID: DeviceRecord] = [:]
  var devicesById: [String: DeviceRecord] = [:]
  var scans: [String: ScanRecord] = [:]
  var scanPlan = ScanPlan.stopped
  var connections: [String: ConnectionRecord] = [:]
  var subscriptions: [String: SubscriptionRecord] = [:]

  var peripheralManager: CBPeripheralManager?
  var peripheralHasState = false
  var peripheralWaiters: [(CBManagerState) -> Bool] = []
  var servers: [String: ServerRecord] = [:]
  var serverAttributes: [ObjectIdentifier: (server: ServerRecord, attribute: ServerAttribute)] = [:]
  var pendingServiceAdds: [ObjectIdentifier: ServerRecord] = [:]
  var centrals: [UUID: RemoteCentral] = [:]
  var advertisement: AdvertisementRecord?
  var notifications: [PendingNotification] = []

  init(
    queue: DispatchQueue,
    authorization: @escaping () -> Authorization = currentAuthorization,
    emit: @escaping EventHandler
  ) {
    self.queue = queue
    self.authorization = authorization
    self.emit = emit
    super.init()
  }

  var hasCentralManager: Bool { central != nil }

  var hasPeripheralManager: Bool { peripheralManager != nil }

  func execute(_ request: ExecuteRequest, reply: @escaping ReplyHandler) {
    dispatchPrecondition(condition: .onQueue(queue))
    let operation = BridgeOperation(id: request.operationId, owner: request.ownerId, reply: reply)
    operation.onFinish = { [weak self] finished in
      if self?.operations[finished.id] === finished {
        self?.operations[finished.id] = nil
      }
    }
    operations[operation.id] = operation
    if let deadline = request.command.deadline(requested: request.deadlineMillis) {
      operation.arm(milliseconds: deadline, on: queue)
    }
    switch request.command {
    case .getState:
      operation.resolve(.state(adapterState()))
    case .getCapabilities:
      operation.resolve(.capabilities)
    case .checkPermissions:
      operation.resolve(.permissions(permissionOutcome(authorization())))
    case .requestPermissions(let roles):
      requestPermissions(roles, operation)
    case .startScan(let serviceUuids, let timeoutMs):
      startScan(serviceUuids, timeoutMs: timeoutMs, operation)
    case .stopScan(let scanId):
      stopScan(scanId, operation)
    case .connect(let deviceId, _):
      connect(deviceId, operation)
    case .disconnect(let connectionId):
      disconnect(connectionId, operation)
    case .discoverServices(let connectionId):
      discoverServices(connectionId, operation)
    case .read(let connectionId, let handle):
      read(connectionId, handle, operation)
    case .write(let connectionId, let handle, let valueBase64, let writeType):
      write(connectionId, handle, valueBase64, writeType, operation)
    case .subscribe(let connectionId, let handle):
      subscribe(connectionId, handle, operation)
    case .unsubscribe(let subscriptionId):
      unsubscribe(subscriptionId, operation)
    case .createServer(let definition):
      createServer(definition, operation)
    case .closeServer(let serverId):
      closeServer(serverId, operation)
    case .startAdvertising(let serverId, let options):
      startAdvertising(serverId, options, operation)
    case .stopAdvertising(let serverId):
      stopAdvertising(serverId, operation)
    case .setValue(let serverId, let key, let valueBase64):
      setValue(serverId, key, valueBase64, operation)
    case .notify(let serverId, let peerId, let key, let valueBase64):
      notify(serverId, peerId, key, valueBase64, operation)
    case .cancel(let target):
      cancel(target, operation)
    case .closeOwner:
      closeOwner(operation)
    case .debugResources:
      operation.resolve(.resources(resources(of: operation.owner)))
    }
  }

  @discardableResult
  func schedule(after milliseconds: UInt64, _ body: @escaping () -> Void) -> DispatchWorkItem {
    schedule(at: .now() + dispatchInterval(milliseconds: milliseconds), body)
  }

  @discardableResult
  func schedule(at deadline: DispatchTime, _ body: @escaping () -> Void) -> DispatchWorkItem {
    let item = DispatchWorkItem(block: body)
    queue.asyncAfter(deadline: deadline, execute: item)
    return item
  }

  // MARK: Status

  /// Never creates a manager: a manager shows the Bluetooth prompt.
  func adapterState() -> String {
    if let central { return adapterStateName(central.state) }
    if let peripheralManager { return adapterStateName(peripheralManager.state) }
    return adapterStateWithoutManager(authorization())
  }

  func requestPermissions(_ roles: PermissionRequest, _ operation: BridgeOperation) {
    let answer = { [weak self] in
      guard let self else { return }
      operation.resolve(.permissions(permissionOutcome(self.authorization())))
    }
    guard roles.scan || roles.connect || roles.advertise, authorization() == .notDetermined else {
      answer()
      return
    }
    let manager = centralManager()
    if centralHasState, manager.state != .unknown {
      answer()
      return
    }
    centralWaiters.append { state in
      guard !operation.isFinished else { return true }
      guard state != .unknown else { return false }
      answer()
      return true
    }
  }

  func resources(of owner: String) -> ResourceCounts {
    ResourceCounts(
      scans: scans.values.filter { $0.owner == owner }.count,
      connections: connections.values.filter { $0.owner == owner && $0.state == .connected }.count,
      subscriptions: subscriptions.values.filter { $0.owner == owner }.count,
      servers: servers.values.filter { $0.owner == owner && $0.state != .registering }.count)
  }

  // MARK: Owners

  func cancel(_ target: String, _ operation: BridgeOperation) {
    if let pending = operations[target], pending !== operation, pending.owner == operation.owner {
      pending.abort(.cancelled)
    }
    operation.resolve(.empty)
  }

  /// Releases everything the owner holds, without events.
  func closeOwner(_ operation: BridgeOperation) {
    let owner = operation.owner
    let links = connections.values.filter { $0.owner == owner && $0.state == .connected }
    // A closing link starts no procedure, so aborting one cannot start the next.
    for link in links {
      link.state = .closing
    }
    for pending in Array(operations.values) where pending.owner == owner && pending !== operation {
      pending.abort(.cancelled)
    }
    for scan in Array(scans.values) where scan.owner == owner {
      endScan(scan.id, notify: false)
    }
    for link in links {
      closeLink(link, rejecting: .cancelled, notify: false)
    }
    for server in Array(servers.values) where server.owner == owner {
      discard(server, rejecting: .cancelled)
    }
    operation.resolve(.empty)
  }

  // MARK: Adapter state

  func centralManager() -> CBCentralManager {
    if let central { return central }
    let manager = CBCentralManager(
      delegate: self, queue: queue, options: [CBCentralManagerOptionShowPowerAlertKey: false])
    central = manager
    return manager
  }

  func peripheralManagerInstance() -> CBPeripheralManager {
    if let peripheralManager { return peripheralManager }
    let manager = CBPeripheralManager(
      delegate: self, queue: queue, options: [CBPeripheralManagerOptionShowPowerAlertKey: false])
    peripheralManager = manager
    return manager
  }

  /// Creates the central manager if needed, waits for its first state, then runs `body` when
  /// the adapter is on. Otherwise rejects with the code of the state.
  func whenCentralReady(_ operation: BridgeOperation, _ body: @escaping (CBCentralManager) -> Void) {
    let manager = centralManager()
    let proceed: (CBManagerState) -> Void = { state in
      guard !operation.isFinished else { return }
      if let error = BridgeError.adapter(state) {
        operation.reject(error)
      } else {
        body(manager)
      }
    }
    if centralHasState {
      proceed(manager.state)
    } else {
      centralWaiters.append { state in
        proceed(state)
        return true
      }
    }
  }

  func whenPeripheralReady(
    _ operation: BridgeOperation, _ body: @escaping (CBPeripheralManager) -> Void
  ) {
    let manager = peripheralManagerInstance()
    let proceed: (CBManagerState) -> Void = { state in
      guard !operation.isFinished else { return }
      if let error = BridgeError.adapter(state) {
        operation.reject(error)
      } else {
        body(manager)
      }
    }
    if peripheralHasState {
      proceed(manager.state)
    } else {
      peripheralWaiters.append { state in
        proceed(state)
        return true
      }
    }
  }

  /// Both managers report the same adapter, so send each change once.
  func noteAdapterState(_ state: CBManagerState) {
    let name = adapterStateName(state)
    guard name != lastAdapterState else { return }
    lastAdapterState = name
    for owner in resourceOwners().sorted() {
      emit(owner, .adapterStateChanged(state: name))
    }
  }

  func resourceOwners() -> Set<String> {
    var owners = Set(scans.values.map(\.owner))
    owners.formUnion(connections.values.filter { $0.state != .closing }.map(\.owner))
    owners.formUnion(subscriptions.values.map(\.owner))
    owners.formUnion(servers.values.map(\.owner))
    return owners
  }
}
