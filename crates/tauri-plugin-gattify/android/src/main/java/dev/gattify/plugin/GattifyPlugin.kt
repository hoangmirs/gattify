package dev.gattify.plugin

import android.Manifest
import android.app.Activity
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import java.util.concurrent.ConcurrentHashMap

@InvokeArg
class SetEventChannelArgs {
  lateinit var channel: Channel
}

@TauriPlugin(
  permissions = [
    Permission(strings = [Manifest.permission.BLUETOOTH_SCAN], alias = PermissionAlias.SCAN),
    Permission(strings = [Manifest.permission.BLUETOOTH_CONNECT], alias = PermissionAlias.CONNECT),
    Permission(strings = [Manifest.permission.BLUETOOTH_ADVERTISE], alias = PermissionAlias.ADVERTISE),
    Permission(strings = [Manifest.permission.ACCESS_FINE_LOCATION], alias = PermissionAlias.LOCATION),
  ],
)
class GattifyPlugin(private val activity: Activity) : Plugin(activity) {
  /** The permission requests in flight, by their invoke. */
  private val permissionAnswers = ConcurrentHashMap<Invoke, () -> Unit>()

  private val backend = GattifyBackend(
    activity.applicationContext,
    object : PermissionHost {
      override fun permissionState(alias: String): String? = getPermissionState(alias)?.toString()

      override fun permissionDeclared(alias: String): Boolean = isPermissionDeclared(alias)

      override fun requestPermissions(aliases: Array<String>, invoke: Invoke, answered: () -> Unit) {
        permissionAnswers[invoke] = answered
        activity.runOnUiThread { requestPermissionForAliases(aliases, invoke, "permissionsAnswered") }
      }
    },
  )

  @Command
  fun setEventChannel(invoke: Invoke) {
    backend.channel = invoke.parseArgs(SetEventChannelArgs::class.java).channel
    invoke.resolve()
  }

  @Command
  fun execute(invoke: Invoke) {
    backend.execute(invoke)
  }

  @Suppress("unused")
  @PermissionCallback
  private fun permissionsAnswered(invoke: Invoke) {
    permissionAnswers.remove(invoke)?.invoke()
  }

  @Suppress("OVERRIDE_DEPRECATION")
  override fun onDestroy() {
    // The newer overload takes an AppCompatActivity, which is not on this module's classpath.
    if (activity.isFinishing) backend.releaseAll()
  }
}
