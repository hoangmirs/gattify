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
    val kind = invoke.getArgs().getJSObject("command")?.getString("kind")
    when (val result = executeResult(kind) { state() }) {
      is ExecuteResult.Resolve -> invoke.resolve(result.reply)
      is ExecuteResult.Reject -> invoke.reject(result.message, result.code)
    }
  }

  private fun state(): String = when {
    adapter == null -> "unavailable"
    Build.VERSION.SDK_INT >= 31 &&
      activity.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) !=
        PackageManager.PERMISSION_GRANTED -> "unauthorized"
    adapter?.isEnabled == true -> "poweredOn"
    else -> "poweredOff"
  }
}
