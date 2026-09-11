import Tauri

enum ExecuteResult {
  case resolve(JsonObject)
  case reject(message: String, code: String)
}

func executeResult(kind: String?, adapterState: () -> String) -> ExecuteResult {
  switch kind {
  case "getState":
    return .resolve(reply("state", adapterState()))
  case "getCapabilities":
    return .resolve(reply("capabilities", capabilities()))
  case "checkPermissions":
    return .resolve(
      reply("permissions", ["scan": "unknown", "connect": "unknown", "advertise": "unknown"]))
  case "cancel", "closeOwner":
    return .resolve(reply("empty"))
  default:
    let name = kind.flatMap { $0.isEmpty ? nil : $0 } ?? "this command"
    return .reject(message: "the iOS backend does not implement \(name) yet", code: "unsupported")
  }
}

private func reply(_ kind: String, _ payload: Any? = nil) -> JsonObject {
  var reply: JsonObject = ["kind": kind]
  if let payload {
    reply["payload"] = payload
  }
  return reply
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
