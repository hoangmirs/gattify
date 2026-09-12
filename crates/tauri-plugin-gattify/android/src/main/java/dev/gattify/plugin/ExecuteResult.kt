package dev.gattify.plugin

import app.tauri.plugin.JSObject

internal sealed interface ExecuteResult {
  class Resolve(val reply: JSObject) : ExecuteResult
  class Reject(val message: String, val code: String) : ExecuteResult
}

/** The adapter facts that the status commands read. */
internal interface StatusProbe {
  val hasAdapter: Boolean
  val adapterOn: Boolean

  /** `BLUETOOTH_CONNECT` is granted on API 31 and later. Always true before. */
  val connectPermitted: Boolean
  val hasAdvertiser: Boolean

  fun permissions(): PermissionOutcomes
}

/**
 * Answers a status command at once and rejects an unknown command. Returns null
 * for every other command: the backend runs it.
 */
internal fun executeResult(kind: String?, probe: StatusProbe): ExecuteResult? {
  if (kind == null || kind !in COMMAND_KINDS) {
    return ExecuteResult.Reject(
      "the Android backend does not know ${if (kind.isNullOrEmpty()) "this command" else kind}",
      ErrorCode.UNSUPPORTED,
    )
  }
  return when (kind) {
    "getState" -> ExecuteResult.Resolve(Replies.state(adapterState(probe)))
    "getCapabilities" -> ExecuteResult.Resolve(
      Replies.capabilities(CapabilityFacts(probe.hasAdapter, probe.adapterOn, probe.hasAdvertiser)),
    )
    "checkPermissions" -> ExecuteResult.Resolve(Replies.permissions(probe.permissions()))
    else -> null
  }
}

internal fun adapterState(probe: StatusProbe): String = when {
  !probe.hasAdapter -> "unavailable"
  !probe.connectPermitted -> "unauthorized"
  probe.adapterOn -> "poweredOn"
  else -> "poweredOff"
}
