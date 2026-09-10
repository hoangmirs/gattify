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
    let kind = command?["kind"] as? String
    switch kind {
    case "getState":
      invoke.resolve(reply("state", adapterState()))
    case "getCapabilities":
      invoke.resolve(reply("capabilities", capabilities()))
    case "checkPermissions":
      invoke.resolve(
        reply("permissions", ["scan": "unknown", "connect": "unknown", "advertise": "unknown"]))
    case "cancel", "closeOwner":
      invoke.resolve(reply("empty"))
    default:
      invoke.reject(
        "the iOS backend does not implement \(kind ?? "this command") yet", code: "unsupported")
    }
  }

  private func reply(_ kind: String, _ payload: Any? = nil) -> JsonObject {
    var reply: JsonObject = ["kind": kind]
    if let payload {
      reply["payload"] = payload
    }
    return reply
  }

  // Reads the authorization without a manager. Creating a CBCentralManager shows the Bluetooth prompt.
  private func adapterState() -> String {
    switch CBManager.authorization {
    case .denied, .restricted:
      return "unauthorized"
    default:
      return "unknown"
    }
  }

  private func capabilities() -> JsonObject {
    let notImplemented: JsonObject = ["level": "unknown", "reason": "backendNotImplemented"]
    return [
      "central": notImplemented,
      "peripheral": notImplemented,
      "advertising": notImplemented,
      "targetedNotify": notImplemented,
      "simultaneousRoles": notImplemented,
      "background": ["level": "unsupported", "reason": "foregroundOnlyContract"] as JsonObject,
    ]
  }
}

@_cdecl("init_plugin_gattify")
public func initPlugin() -> Plugin {
  GattifyPlugin()
}
