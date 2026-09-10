package dev.gattify.plugin

import android.Manifest
import android.app.Activity
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class SetEventChannelArgs {
  lateinit var channel: Channel
}

@TauriPlugin(
  permissions = [
    Permission(
      strings = [
        Manifest.permission.BLUETOOTH_SCAN,
        Manifest.permission.BLUETOOTH_CONNECT,
        Manifest.permission.BLUETOOTH_ADVERTISE,
      ],
      alias = "bleRoles",
    ),
  ],
)
class GattifyPlugin(private val activity: Activity) : Plugin(activity) {
  private var events: Channel? = null

  private val adapter: BluetoothAdapter?
    get() = (activity.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager)?.adapter

  @Command
  fun setEventChannel(invoke: Invoke) {
    events = invoke.parseArgs(SetEventChannelArgs::class.java).channel
    invoke.resolve()
  }

  @Command
  fun execute(invoke: Invoke) {
    when (val kind = invoke.getArgs().getJSObject("command")?.getString("kind")) {
      "getState" -> invoke.resolve(reply("state").put("payload", state()))
      "getCapabilities" -> invoke.resolve(reply("capabilities").put("payload", capabilities()))
      "checkPermissions" -> invoke.resolve(reply("permissions").put("payload", unknownPermissions()))
      "cancel", "closeOwner" -> invoke.resolve(reply("empty"))
      else -> invoke.reject("the Android backend does not implement $kind yet", "unsupported")
    }
  }

  private fun reply(kind: String): JSObject = JSObject().put("kind", kind)

  private fun state(): String = when {
    adapter == null -> "unavailable"
    Build.VERSION.SDK_INT >= 31 &&
      activity.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) !=
        PackageManager.PERMISSION_GRANTED -> "unauthorized"
    adapter?.isEnabled == true -> "poweredOn"
    else -> "poweredOff"
  }

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
}
