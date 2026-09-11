package dev.gattify.plugin

import app.tauri.plugin.JSObject

internal sealed interface ExecuteResult {
  class Resolve(val reply: JSObject) : ExecuteResult
  class Reject(val message: String, val code: String) : ExecuteResult
}

internal fun executeResult(kind: String?, adapterState: () -> String): ExecuteResult =
  when (kind) {
    "getState" -> ExecuteResult.Resolve(reply("state").put("payload", adapterState()))
    "getCapabilities" -> ExecuteResult.Resolve(reply("capabilities").put("payload", capabilities()))
    "checkPermissions" -> ExecuteResult.Resolve(reply("permissions").put("payload", unknownPermissions()))
    "cancel", "closeOwner" -> ExecuteResult.Resolve(reply("empty"))
    else -> ExecuteResult.Reject(
      "the Android backend does not implement ${if (kind.isNullOrEmpty()) "this command" else kind} yet",
      "unsupported",
    )
  }

private fun reply(kind: String): JSObject = JSObject().put("kind", kind)

private fun capabilities(): JSObject {
  val notImplemented = unknown("backendNotImplemented")
  return JSObject()
    .put("central", notImplemented)
    .put("peripheral", notImplemented)
    .put("advertising", notImplemented)
    .put("targetedNotify", notImplemented)
    .put("simultaneousRoles", notImplemented)
    .put("background", JSObject().put("level", "unsupported").put("reason", "foregroundOnlyContract"))
}

private fun unknownPermissions(): JSObject =
  JSObject().put("scan", "unknown").put("connect", "unknown").put("advertise", "unknown")

private fun unknown(reason: String): JSObject =
  JSObject().put("level", "unknown").put("reason", reason)
