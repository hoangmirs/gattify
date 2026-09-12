package dev.gattify.plugin

internal enum class Role(val key: String) {
  SCAN("scan"),
  CONNECT("connect"),
  ADVERTISE("advertise"),
}

/** The aliases declared in `@TauriPlugin(permissions)`. */
internal object PermissionAlias {
  const val SCAN = "bluetoothScan"
  const val CONNECT = "bluetoothConnect"
  const val ADVERTISE = "bluetoothAdvertise"
  const val LOCATION = "location"
}

/** API 31 split the Bluetooth permission by role. */
internal const val SPLIT_PERMISSIONS_SDK = 31

internal const val NOT_REQUIRED = "notRequired"

/** The permission alias of [role], or null when [sdk] needs no runtime permission for it. */
internal fun permissionAlias(role: Role, sdk: Int): String? = when {
  sdk >= SPLIT_PERMISSIONS_SDK -> when (role) {
    Role.SCAN -> PermissionAlias.SCAN
    Role.CONNECT -> PermissionAlias.CONNECT
    Role.ADVERTISE -> PermissionAlias.ADVERTISE
  }
  role == Role.SCAN -> PermissionAlias.LOCATION
  else -> null
}

/** The Android permission of [role], or null when [sdk] needs none. */
internal fun androidPermission(role: Role, sdk: Int): String? = when {
  sdk >= SPLIT_PERMISSIONS_SDK -> when (role) {
    Role.SCAN -> "android.permission.BLUETOOTH_SCAN"
    Role.CONNECT -> "android.permission.BLUETOOTH_CONNECT"
    Role.ADVERTISE -> "android.permission.BLUETOOTH_ADVERTISE"
  }
  role == Role.SCAN -> "android.permission.ACCESS_FINE_LOCATION"
  else -> null
}

/**
 * Maps a Tauri `PermissionState`. Tauri stores `denied` after a request was denied
 * while `shouldShowRequestPermissionRationale` was false.
 */
internal fun permissionOutcome(tauriState: String?): String = when (tauriState) {
  "granted" -> "granted"
  "denied" -> "deniedPermanently"
  "prompt", "prompt-with-rationale" -> "promptable"
  else -> "unknown"
}

/** The outcome of each role. [state] reads the Tauri state of an alias. */
internal fun permissionOutcomes(sdk: Int, state: (String) -> String?): PermissionOutcomes {
  fun outcome(role: Role) = permissionAlias(role, sdk)?.let { permissionOutcome(state(it)) } ?: NOT_REQUIRED
  return PermissionOutcomes(outcome(Role.SCAN), outcome(Role.CONNECT), outcome(Role.ADVERTISE))
}

/** The aliases to request for [ask]: the requested roles that are neither granted nor not required. */
internal fun aliasesToRequest(ask: PermissionAsk, sdk: Int, state: (String) -> String?): List<String> {
  val roles = listOfNotNull(
    Role.SCAN.takeIf { ask.scan },
    Role.CONNECT.takeIf { ask.connect },
    Role.ADVERTISE.takeIf { ask.advertise },
  )
  return roles
    .mapNotNull { permissionAlias(it, sdk) }
    .filter { permissionOutcome(state(it)) != "granted" }
    .distinct()
}

/** What a command needs before it runs: the permission of [role], and an adapter that is on. */
internal class Requirement(val role: Role, val adapterOn: Boolean)

/** The permission table of the contract. Cleanup commands also run while the adapter is off. */
internal fun requirementOf(kind: String?): Requirement? = when (kind) {
  "startScan" -> Requirement(Role.SCAN, adapterOn = true)
  "connect", "discoverServices", "read", "write", "subscribe", "createServer", "notify" ->
    Requirement(Role.CONNECT, adapterOn = true)
  "disconnect", "unsubscribe", "closeServer", "setValue" -> Requirement(Role.CONNECT, adapterOn = false)
  "startAdvertising" -> Requirement(Role.ADVERTISE, adapterOn = true)
  "stopAdvertising" -> Requirement(Role.ADVERTISE, adapterOn = false)
  else -> null
}
