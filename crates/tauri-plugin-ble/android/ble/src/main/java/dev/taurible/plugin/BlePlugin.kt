package dev.taurible.plugin

import android.Manifest
import android.app.Activity
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import app.tauri.annotation.Command
import app.tauri.annotation.Permission
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

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
class BlePlugin(private val activity: Activity) : Plugin(activity) {
  private val adapter: BluetoothAdapter?
    get() = (activity.getSystemService(Context.BLUETOOTH_SERVICE) as BluetoothManager).adapter

  @Command
  fun getState(invoke: Invoke) {
    val state = when {
      adapter == null -> "unavailable"
      Build.VERSION.SDK_INT >= 31 &&
        activity.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) !=
          PackageManager.PERMISSION_GRANTED -> "unauthorized"
      adapter?.isEnabled == true -> "poweredOn"
      else -> "poweredOff"
    }
    invoke.resolve(JSObject().put("state", state))
  }

  @Command
  fun getCapabilities(invoke: Invoke) {
    val bluetoothAdapter = adapter
    val result = JSObject()
    result.put("central", support(bluetoothAdapter != null, "adapterUnavailable"))
    result.put(
      "peripheral",
      support(
        bluetoothAdapter?.isMultipleAdvertisementSupported == true,
        "multipleAdvertisementUnsupported",
      ),
    )
    result.put(
      "advertising",
      support(bluetoothAdapter?.bluetoothLeAdvertiser != null, "advertiserUnavailable"),
    )
    result.put(
      "targetedNotify",
      support(bluetoothAdapter != null, "requiresGattServerRuntimeProbe"),
    )
    result.put("simultaneousRoles", unknown("requiresRuntimeQualification"))
    result.put("background", unsupported("foregroundOnlyContract"))
    invoke.resolve(result)
  }

  private fun support(value: Boolean, reason: String): JSObject =
    JSObject()
      .put("level", if (value) "supported" else "unsupported")
      .put("reason", if (value) "available" else reason)

  private fun unknown(reason: String): JSObject =
    JSObject().put("level", "unknown").put("reason", reason)

  private fun unsupported(reason: String): JSObject =
    JSObject().put("level", "unsupported").put("reason", reason)
}

