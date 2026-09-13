import Foundation

/// Rust's reply callback: the ticket of an `execute` call, whether the command resolved, and
/// UTF-8 JSON that is valid only during the call. A resolved command carries a Reply, a
/// rejected one `{ "code", "message" }`.
public typealias ReplyCallback = @convention(c) (UInt64, Bool, UnsafePointer<UInt8>?, Int) -> Void

/// Rust's event callback: one UTF-8 JSON event envelope, valid only during the call.
public typealias EventCallback = @convention(c) (UnsafePointer<UInt8>?, Int) -> Void

/// The macOS side of the bridge. It runs the iOS engine unchanged: CoreBluetooth is the same
/// API on both systems. Only this file differs from the Tauri iOS plugin.
private final class MacBridge {
  static let shared = MacBridge()

  /// The only queue that touches bridge state. It is also the CoreBluetooth delegate queue.
  let queue = DispatchQueue(label: "dev.gattify.plugin")
  var onReply: ReplyCallback?
  var onEvent: EventCallback?
  lazy var engine = GattifyEngine(queue: queue) { [unowned self] ownerId, event in
    self.send(event, to: ownerId)
  }

  /// Runs on the queue.
  func answer(_ ticket: UInt64, _ result: Result<Reply, BridgeError>) {
    guard let onReply else { return }
    var text: String
    let resolved: Bool
    switch result {
    case .success(let reply):
      text = reply.json.text
      resolved = true
    case .failure(let error):
      text = JSON.object(["code": .string(error.code), "message": .string(error.message)]).text
      resolved = false
    }
    text.withUTF8 { bytes in onReply(ticket, resolved, bytes.baseAddress, bytes.count) }
  }

  /// Runs on the queue. Events raised before Rust starts the bridge are dropped.
  func send(_ event: BridgeEvent, to ownerId: String) {
    guard let onEvent else { return }
    var text = event.envelope(ownerId: ownerId).text
    text.withUTF8 { bytes in onEvent(bytes.baseAddress, bytes.count) }
  }
}

/// Keeps the latest callbacks. Rust calls it before the first `gattify_macos_execute`.
@_cdecl("gattify_macos_start")
public func gattifyMacosStart(_ onReply: ReplyCallback, _ onEvent: EventCallback) {
  let bridge = MacBridge.shared
  bridge.queue.sync {
    bridge.onReply = onReply
    bridge.onEvent = onEvent
  }
}

/// Runs one command. The JSON is `{ operationId, ownerId, deadlineMillis, command }`, as on iOS.
/// The bytes are copied before this returns; the answer arrives later through `onReply`.
@_cdecl("gattify_macos_execute")
public func gattifyMacosExecute(_ ticket: UInt64, _ bytes: UnsafePointer<UInt8>?, _ length: Int) {
  let data = bytes.map { Data(bytes: $0, count: length) } ?? Data()
  let request = ExecuteRequest.decode(data)
  let bridge = MacBridge.shared
  bridge.queue.async {
    switch request {
    case .failure(let error):
      bridge.answer(ticket, .failure(error))
    case .success(let request):
      bridge.engine.execute(request) { result in bridge.answer(ticket, result) }
    }
  }
}
