// gattify probe: a macOS CoreBluetooth peer that speaks the gattify v1 protocol.
//   probe join [seconds]   dial the first host that advertises the lab service
//   probe host [seconds]   advertise the lab service and answer joiners
import CoreBluetooth
import Foundation

let serviceUUID = CBUUID(string: "80ff87c3-8e84-4914-aedc-0d6a3ba5534d")
let infoUUID = CBUUID(string: "b1e10f10-6a2c-4a62-8e9e-2c938fa30101")
let rxUUID = CBUUID(string: "b1e10f10-6a2c-4a62-8e9e-2c938fa30102")
let txUUID = CBUUID(string: "b1e10f10-6a2c-4a62-8e9e-2c938fa30103")
let started = Date()

func log(_ text: String) {
  let elapsed = String(format: "%7.3f", Date().timeIntervalSince(started))
  print("[\(elapsed)] \(text)")
  fflush(stdout)
}

enum Kind: UInt8 { case hello = 1, helloAck, data, ack, close }

struct Frame {
  var kind: Kind
  var messageId: UInt32
  var index: UInt16
  var count: UInt16
  var total: UInt32
  var payload: Data

  func encode() -> Data {
    var bytes = Data([1, kind.rawValue])
    withUnsafeBytes(of: messageId.littleEndian) { bytes.append(contentsOf: $0) }
    withUnsafeBytes(of: index.littleEndian) { bytes.append(contentsOf: $0) }
    withUnsafeBytes(of: count.littleEndian) { bytes.append(contentsOf: $0) }
    withUnsafeBytes(of: total.littleEndian) { bytes.append(contentsOf: $0) }
    bytes.append(payload)
    return bytes
  }

  static func decode(_ data: Data) -> Frame? {
    let bytes = [UInt8](data)
    guard bytes.count >= 14, bytes[0] == 1, let kind = Kind(rawValue: bytes[1]) else { return nil }
    func u32(_ at: Int) -> UInt32 { (0..<4).reduce(0) { $0 | UInt32(bytes[at + $1]) << (8 * $1) } }
    func u16(_ at: Int) -> UInt16 { UInt16(bytes[at]) | UInt16(bytes[at + 1]) << 8 }
    return Frame(
      kind: kind, messageId: u32(2), index: u16(6), count: u16(8), total: u32(10),
      payload: Data(bytes[14...]))
  }

  static func control(_ kind: Kind) -> Data {
    Frame(kind: kind, messageId: 0, index: 0, count: 1, total: 0, payload: Data()).encode()
  }

  static func ack(_ id: UInt32) -> Data {
    Frame(kind: .ack, messageId: id, index: 0, count: 1, total: 0, payload: Data()).encode()
  }

  static func fragments(id: UInt32, payload: Data, limit: Int) -> [Data] {
    let size = limit - 14
    let count = max(1, (payload.count + size - 1) / size)
    return (0..<count).map { index in
      let start = index * size
      let end = min(start + size, payload.count)
      return Frame(
        kind: .data, messageId: id, index: UInt16(index), count: UInt16(count),
        total: UInt32(payload.count), payload: payload.subdata(in: start..<end)
      ).encode()
    }
  }
}

/// Reassembles DATA messages.
struct Reassembly {
  var parts: [UInt32: [UInt16: Data]] = [:]

  mutating func add(_ frame: Frame) -> Data? {
    var slots = parts[frame.messageId] ?? [:]
    slots[frame.index] = frame.payload
    parts[frame.messageId] = slots
    guard slots.count == Int(frame.count) else { return nil }
    parts[frame.messageId] = nil
    return (0..<frame.count).reduce(into: Data()) { $0.append(slots[$1]!) }
  }
}

func describe(_ payload: Data) -> String {
  if payload.count <= 200, let text = String(data: payload, encoding: .utf8) { return "\"\(text)\"" }
  return "\(payload.count) bytes"
}

// MARK: Joiner

final class Joiner: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {
  var central: CBCentralManager!
  var peripheral: CBPeripheral?
  var rx: CBCharacteristic?
  var tx: CBCharacteristic?
  var limit = 20
  var nextId: UInt32 = 1
  var control: [Data] = []
  var data: [Data] = []
  var writing = false
  var inFlight: (id: UInt32, sentAt: Date, bytes: Int)?
  var reassembly = Reassembly()
  var ready = false

  override init() {
    super.init()
    central = CBCentralManager(delegate: self, queue: .main)
  }

  func centralManagerDidUpdateState(_ central: CBCentralManager) {
    log("central state \(central.state.rawValue)")
    guard central.state == .poweredOn else { return }
    log("scanning for \(serviceUUID)")
    central.scanForPeripherals(withServices: [serviceUUID])
  }

  func centralManager(
    _ central: CBCentralManager, didDiscover peripheral: CBPeripheral,
    advertisementData: [String: Any], rssi: NSNumber
  ) {
    guard self.peripheral == nil else { return }
    let name = advertisementData[CBAdvertisementDataLocalNameKey] as? String ?? peripheral.name ?? "?"
    log("found \(name) rssi \(rssi)")
    central.stopScan()
    self.peripheral = peripheral
    peripheral.delegate = self
    central.connect(peripheral)
  }

  func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
    log("connected, value limit \(peripheral.maximumWriteValueLength(for: .withoutResponse)) at first")
    awaitLimit(peripheral, waited: 0)
  }

  func awaitLimit(_ peripheral: CBPeripheral, waited: Int) {
    let length = peripheral.maximumWriteValueLength(for: .withoutResponse)
    if length <= 20 && waited < 1000 {
      DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(50)) {
        self.awaitLimit(peripheral, waited: waited + 50)
      }
      return
    }
    limit = min(length, 512)
    log("value limit \(limit) after \(waited) ms")
    peripheral.discoverServices([serviceUUID])
  }

  func centralManager(_ central: CBCentralManager, didFailToConnect peripheral: CBPeripheral, error: Error?) {
    log("connect failed: \(String(describing: error))")
    exit(1)
  }

  func centralManager(
    _ central: CBCentralManager, didDisconnectPeripheral peripheral: CBPeripheral, error: Error?
  ) {
    log("disconnected: \(String(describing: error))")
    exit(0)
  }

  func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
    guard let service = peripheral.services?.first(where: { $0.uuid == serviceUUID }) else {
      log("no lab service: \(String(describing: error))")
      exit(1)
    }
    peripheral.discoverCharacteristics([infoUUID, rxUUID, txUUID], for: service)
  }

  func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
    let characteristics = service.characteristics ?? []
    rx = characteristics.first { $0.uuid == rxUUID }
    tx = characteristics.first { $0.uuid == txUUID }
    guard let info = characteristics.first(where: { $0.uuid == infoUUID }), rx != nil, tx != nil else {
      log("missing characteristics")
      exit(1)
    }
    peripheral.readValue(for: info)
  }

  func peripheral(_ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?) {
    let value = characteristic.value ?? Data()
    if characteristic.uuid == infoUUID {
      log("info = \([UInt8](value)) \(error.map { "error \($0)" } ?? "")")
      peripheral.setNotifyValue(true, for: tx!)
      return
    }
    guard let frame = Frame.decode(value) else {
      log("undecodable notification \([UInt8](value))")
      return
    }
    switch frame.kind {
    case .helloAck:
      ready = true
      log("HELLO_ACK: peer ready")
      send("hello from the mac".data(using: .utf8)!)
    case .data:
      if let message = reassembly.add(frame) {
        log("message #\(frame.messageId) from host: \(describe(message))")
        write(Frame.ack(frame.messageId))
      }
    case .ack:
      if let flight = inFlight, flight.id == frame.messageId {
        let ms = Int(Date().timeIntervalSince(flight.sentAt) * 1000)
        log("ACK #\(frame.messageId): \(flight.bytes) bytes in \(ms) ms")
        inFlight = nil
        if flight.bytes < 4096 {
          send(Data((0..<4096).map { UInt8($0 % 251) }))
        } else {
          log("staying connected 20 s for messages from the phone, then closing")
          DispatchQueue.main.asyncAfter(deadline: .now() + 20) { self.close() }
        }
      }
    case .close:
      log("CLOSE from host")
      central.cancelPeripheralConnection(peripheral)
    case .hello:
      break
    }
  }

  func peripheral(_ peripheral: CBPeripheral, didUpdateNotificationStateFor characteristic: CBCharacteristic, error: Error?) {
    log("TX notifying \(characteristic.isNotifying) \(error.map { "error \($0)" } ?? "")")
    write(Frame.control(.hello))
  }

  func peripheral(_ peripheral: CBPeripheral, didWriteValueFor characteristic: CBCharacteristic, error: Error?) {
    if let error { log("write error \(error)") }
    writing = false
    flush()
  }

  func send(_ payload: Data) {
    let id = nextId
    nextId += 1
    inFlight = (id, Date(), payload.count)
    log("sending #\(id): \(describe(payload))")
    data.append(contentsOf: Frame.fragments(id: id, payload: payload, limit: limit))
    flush()
  }

  /// Control frames go ahead of DATA frames. One write with response at a time.
  func write(_ frame: Data) {
    control.append(frame)
    flush()
  }

  func flush() {
    guard !writing, let peripheral, let rx else { return }
    let next = control.isEmpty ? (data.isEmpty ? nil : data.removeFirst()) : control.removeFirst()
    guard let next else { return }
    writing = true
    peripheral.writeValue(next, for: rx, type: .withResponse)
  }

  func close() {
    log("sending CLOSE")
    write(Frame.control(.close))
    DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
      if let peripheral = self.peripheral { self.central.cancelPeripheralConnection(peripheral) }
    }
  }
}

// MARK: Host

final class Host: NSObject, CBPeripheralManagerDelegate {
  var manager: CBPeripheralManager!
  let info = CBMutableCharacteristic(type: infoUUID, properties: [.read], value: Data([1]), permissions: [.readable])
  let rx = CBMutableCharacteristic(type: rxUUID, properties: [.write], value: nil, permissions: [.writeable])
  let tx = CBMutableCharacteristic(type: txUUID, properties: [.notify], value: nil, permissions: [])
  var centrals: [UUID: CBCentral] = [:]
  var reassembly = Reassembly()
  var controlQueue: [(Data, CBCentral)] = []
  var queue: [(Data, CBCentral)] = []
  var nextId: UInt32 = 1
  var sentAt: [UInt32: (Date, Int)] = [:]

  override init() {
    super.init()
    manager = CBPeripheralManager(delegate: self, queue: .main)
  }

  func peripheralManagerDidUpdateState(_ manager: CBPeripheralManager) {
    log("peripheral state \(manager.state.rawValue)")
    guard manager.state == .poweredOn else { return }
    let service = CBMutableService(type: serviceUUID, primary: true)
    service.characteristics = [info, rx, tx]
    manager.add(service)
  }

  func peripheralManager(_ manager: CBPeripheralManager, didAdd service: CBService, error: Error?) {
    log("service added \(error.map { "error \($0)" } ?? "")")
    manager.startAdvertising([
      CBAdvertisementDataServiceUUIDsKey: [serviceUUID], CBAdvertisementDataLocalNameKey: "mac host",
    ])
  }

  func peripheralManagerDidStartAdvertising(_ manager: CBPeripheralManager, error: Error?) {
    log("advertising \(error.map { "error \($0)" } ?? "")")
  }

  func peripheralManager(_ manager: CBPeripheralManager, central: CBCentral, didSubscribeTo characteristic: CBCharacteristic) {
    log("central subscribed, notification size \(central.maximumUpdateValueLength)")
    centrals[central.identifier] = central
  }

  func peripheralManager(_ manager: CBPeripheralManager, central: CBCentral, didUnsubscribeFrom characteristic: CBCharacteristic) {
    log("central unsubscribed")
  }

  func peripheralManager(_ manager: CBPeripheralManager, didReceiveWrite requests: [CBATTRequest]) {
    manager.respond(to: requests[0], withResult: .success)
    for request in requests {
      guard let value = request.value, let frame = Frame.decode(value) else { continue }
      let central = request.central
      switch frame.kind {
      case .hello:
        log("HELLO from central")
        notify(Frame.control(.helloAck), to: central)
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
          self.send("hello from the mac host".data(using: .utf8)!, to: central)
        }
      case .data:
        if let message = reassembly.add(frame) {
          log("message #\(frame.messageId) from joiner: \(describe(message))")
          notify(Frame.ack(frame.messageId), to: central)
        }
      case .ack:
        if let (at, bytes) = sentAt.removeValue(forKey: frame.messageId) {
          log("ACK #\(frame.messageId): \(bytes) bytes in \(Int(Date().timeIntervalSince(at) * 1000)) ms")
        }
      case .close:
        log("CLOSE from joiner")
      case .helloAck:
        break
      }
    }
  }

  func peripheralManager(_ manager: CBPeripheralManager, didReceiveRead request: CBATTRequest) {
    request.value = info.value
    manager.respond(to: request, withResult: .success)
  }

  func send(_ payload: Data, to central: CBCentral) {
    let id = nextId
    nextId += 1
    sentAt[id] = (Date(), payload.count)
    log("sending #\(id): \(describe(payload))")
    let limit = min(central.maximumUpdateValueLength, 512)
    for fragment in Frame.fragments(id: id, payload: payload, limit: limit) {
      queue.append((fragment, central))
    }
    drain()
  }

  /// Control frames go ahead of DATA frames.
  func notify(_ frame: Data, to central: CBCentral) {
    controlQueue.append((frame, central))
    drain()
  }

  func drain() {
    while true {
      let fromControl = !controlQueue.isEmpty
      guard let (frame, central) = fromControl ? controlQueue.first : queue.first else { return }
      guard manager.updateValue(frame, for: tx, onSubscribedCentrals: [central]) else { return }
      if fromControl { controlQueue.removeFirst() } else { queue.removeFirst() }
    }
  }

  func peripheralManagerIsReady(toUpdateSubscribers manager: CBPeripheralManager) {
    drain()
  }
}

let arguments = CommandLine.arguments
let mode = arguments.count > 1 ? arguments[1] : "join"
let seconds = arguments.count > 2 ? Double(arguments[2]) ?? 60 : 60
var keep: AnyObject?
switch mode {
case "host":
  keep = Host()
default:
  let joiner = Joiner()
  keep = joiner
  DispatchQueue.main.asyncAfter(deadline: .now() + seconds - 3) { joiner.close() }
}
DispatchQueue.main.asyncAfter(deadline: .now() + seconds) {
  log("done")
  exit(0)
}
RunLoop.main.run()
