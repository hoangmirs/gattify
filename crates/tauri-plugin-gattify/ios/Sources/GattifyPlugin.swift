import CoreBluetooth
import Tauri

struct SetEventChannelArgs: Decodable {
  let channel: Channel
}

final class GattifyPlugin: Plugin {
  private var events: Channel?

  @objc public func setEventChannel(_ invoke: Invoke) throws {
    events = try invoke.parseArgs(SetEventChannelArgs.self).channel
    invoke.resolve()
  }

  @objc public func execute(_ invoke: Invoke) throws {
    let command = try invoke.getArgs()["command"] as? JSObject
    switch executeResult(kind: command?["kind"] as? String, adapterState: { self.adapterState() }) {
    case .resolve(let reply):
      invoke.resolve(reply)
    case .reject(let message, let code):
      invoke.reject(message, code: code)
    }
  }

  // Reads the authorization without a manager. Creating a CBCentralManager shows the Bluetooth prompt.
  private func adapterState() -> String {
    guard #available(iOS 13.1, *) else { return "unknown" }
    switch CBManager.authorization {
    case .denied, .restricted:
      return "unauthorized"
    default:
      return "unknown"
    }
  }
}

@_cdecl("init_plugin_gattify")
public func initPlugin() -> Plugin {
  GattifyPlugin()
}
