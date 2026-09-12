import Foundation
import Tauri

struct SetEventChannelArgs: Decodable {
  let channel: Channel
}

final class GattifyPlugin: Plugin {
  /// The only queue that touches plugin state. It is also the CoreBluetooth delegate queue.
  private let queue = DispatchQueue(label: "dev.gattify.plugin")
  private var events: Channel?
  private lazy var engine: GattifyEngine = GattifyEngine(queue: queue) { [weak self] ownerId, event in
    self?.send(event, to: ownerId)
  }

  @objc public func setEventChannel(_ invoke: Invoke) throws {
    let channel = try invoke.parseArgs(SetEventChannelArgs.self).channel
    queue.async {
      self.events = channel
      invoke.resolve()
    }
  }

  @objc public func execute(_ invoke: Invoke) {
    let request = ExecuteRequest.decode(invoke.getRawArgs())
    queue.async {
      switch request {
      case .failure(let error):
        invoke.reject(error.message, code: error.code)
      case .success(let request):
        self.engine.execute(request) { result in
          switch result {
          case .success(let reply):
            invoke.resolve(reply.json)
          case .failure(let error):
            invoke.reject(error.message, code: error.code)
          }
        }
      }
    }
  }

  /// Runs on the plugin queue. Events raised before Rust sends the channel are dropped.
  private func send(_ event: BridgeEvent, to ownerId: String) {
    try? events?.send(event.envelope(ownerId: ownerId))
  }
}

@_cdecl("init_plugin_gattify")
public func initPlugin() -> Plugin {
  GattifyPlugin()
}
