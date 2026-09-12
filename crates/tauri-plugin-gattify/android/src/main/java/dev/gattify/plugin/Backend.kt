package dev.gattify.plugin

import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.util.Log
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject

internal const val TAG = "Gattify"

/** What the backend needs from the Tauri plugin for runtime permissions. */
internal interface PermissionHost {
  /** The Tauri `PermissionState` of [alias], as a string. */
  fun permissionState(alias: String): String?

  fun permissionDeclared(alias: String): Boolean

  /** Shows the system prompt. Calls [answered], on any thread, after the user answers. */
  fun requestPermissions(aliases: Array<String>, invoke: Invoke, answered: () -> Unit)
}

private class InvokeResponder(private val invoke: Invoke) : Responder {
  override fun resolve(reply: JSObject) = invoke.resolve(reply)

  override fun reject(message: String, code: String) = invoke.reject(message, code)
}

private class HandlerScheduler(private val handler: Handler) : Scheduler {
  override fun schedule(delayMs: Long, action: () -> Unit): Cancellable {
    val runnable = Runnable(action)
    handler.postDelayed(runnable, delayMs)
    return Cancellable { handler.removeCallbacks(runnable) }
  }

  override fun now(): Long = SystemClock.elapsedRealtime()
}

/** Runs [block] and turns a missing runtime permission into `permissionDenied`. */
internal inline fun <T> guard(block: () -> T): T = try {
  block()
} catch (e: SecurityException) {
  throw permissionDenied(e.message ?: "a Bluetooth permission is missing")
}

/** Runs a Bluetooth call whose failure does not matter, such as a stop during cleanup. */
internal inline fun quietly(block: () -> Unit) {
  try {
    block()
  } catch (e: SecurityException) {
    Log.w(TAG, "a Bluetooth call lacked a permission", e)
  } catch (e: IllegalStateException) {
    // BluetoothLeScanner and BluetoothLeAdvertiser throw this while the adapter is off.
    Log.w(TAG, "a Bluetooth call failed", e)
  }
}

/**
 * The native side of the bridge contract. Every command and every Bluetooth
 * callback runs on one handler thread, so the state needs no locks.
 */
internal class GattifyBackend(val context: Context, private val permissions: PermissionHost) {
  private val thread = HandlerThread("gattify").apply { start() }
  val handler = Handler(thread.looper)
  val scheduler: Scheduler = HandlerScheduler(handler)
  val ids = IdAllocator()
  val devices = RemoteRegistry("device", ids)
  private val scannedDevices = HashMap<String, BluetoothDevice>()
  private val procedures = Procedures(scheduler)
  private val permissionJobs = ArrayDeque<PermissionJob>()
  private var permissionActive: PermissionJob? = null

  @Volatile
  var channel: Channel? = null

  val scanner = Scanner(this)
  val central = Central(this)
  val peripheral = Peripheral(this)
  val advertiser = Advertiser(this)

  val manager: BluetoothManager?
    get() = context.getSystemService(BluetoothManager::class.java)

  val adapter: BluetoothAdapter?
    get() = manager?.adapter

  private class PermissionJob(val procedure: Procedure, val invoke: Invoke, val aliases: List<String>)

  private val probe = object : StatusProbe {
    override val hasAdapter: Boolean
      get() = adapter != null
    override val adapterOn: Boolean
      get() = adapter?.isEnabled == true
    override val connectPermitted: Boolean
      get() = hasPermission(Role.CONNECT)
    override val hasAdvertiser: Boolean
      get() = adapter?.bluetoothLeAdvertiser != null

    override fun permissions() = currentPermissions()
  }

  private val adapterReceiver = object : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
      if (intent.action != BluetoothAdapter.ACTION_STATE_CHANGED) return
      adapterChanged(intent.getIntExtra(BluetoothAdapter.EXTRA_STATE, BluetoothAdapter.ERROR))
    }
  }

  init {
    val filter = IntentFilter(BluetoothAdapter.ACTION_STATE_CHANGED)
    // Only the system sends this protected broadcast. The receiver runs on the handler thread.
    if (Build.VERSION.SDK_INT >= 33) {
      context.registerReceiver(adapterReceiver, filter, null, handler, Context.RECEIVER_EXPORTED)
    } else {
      context.registerReceiver(adapterReceiver, filter, null, handler)
    }
  }

  fun post(action: () -> Unit) {
    handler.post(action)
  }

  /** Called on the Tauri thread. Parses the call, then runs it on the handler thread. */
  fun execute(invoke: Invoke) {
    val request = try {
      parseRequest(invoke.getArgs())
    } catch (e: BleException) {
      invoke.reject(e.message, e.code)
      return
    } catch (e: Exception) {
      invoke.reject("the execute arguments are not valid JSON", ErrorCode.INVALID_ARGUMENT)
      return
    }
    post { run(request, invoke) }
  }

  fun emit(ownerId: String, event: JSObject) {
    val channel = channel ?: return
    try {
      channel.send(Events.envelope(ownerId, event))
    } catch (e: Exception) {
      Log.w(TAG, "could not send an event", e)
    }
  }

  fun hasPermission(role: Role): Boolean {
    val permission = androidPermission(role, Build.VERSION.SDK_INT) ?: return true
    return context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED
  }

  /** Remembers the Bluetooth device of a scan result, so that connect keeps its address type. */
  fun rememberDevice(deviceId: String, device: BluetoothDevice) {
    scannedDevices[deviceId] = device
  }

  /** The device of [deviceId], when an owner of the family of [ownerId] saw it in a scan result. */
  fun knownDevice(deviceId: String, ownerId: String): BluetoothDevice {
    val address = devices.addressOf(deviceId)
    if (address == null || !devices.seenByFamilyOf(deviceId, ownerId)) throw invalidHandle(deviceId)
    return scannedDevices[deviceId]
      ?: adapter?.getRemoteDevice(address)
      ?: throw BleException(ErrorCode.UNAVAILABLE, "this device has no Bluetooth adapter")
  }

  /** Releases every resource without events, when the activity finishes. */
  fun releaseAll() = post {
    procedures.abortAll(cancelled("the app closed"))
    scanner.closeOwner(null)
    central.closeOwner(null)
    peripheral.closeOwner(null)
  }

  private fun run(request: Request, invoke: Invoke) {
    val procedure = procedures.start(request.operationId, request.ownerId, InvokeResponder(invoke))
    try {
      when (val result = executeResult(request.kind, probe)) {
        is ExecuteResult.Resolve -> procedure.resolve(result.reply)
        is ExecuteResult.Reject -> procedure.reject(BleException(result.code, result.message))
        null -> {
          requirementOf(request.kind)?.let(::requireReady)
          dispatch(request, procedure, invoke)
        }
      }
    } catch (e: BleException) {
      procedure.abort(e)
    } catch (e: SecurityException) {
      procedure.abort(permissionDenied(e.message ?: "a Bluetooth permission is missing"))
    } catch (e: Exception) {
      Log.e(TAG, "${request.kind} failed", e)
      procedure.abort(internalError(e.message ?: e.toString()))
    }
  }

  /** Rejects with `unavailable`, `permissionDenied` or `bluetoothOff` before a command runs. */
  private fun requireReady(requirement: Requirement) {
    val adapter = adapter ?: throw BleException(ErrorCode.UNAVAILABLE, "this device has no Bluetooth adapter")
    if (!hasPermission(requirement.role)) throw permissionDenied("the ${requirement.role.key} permission is missing")
    if (requirement.adapterOn && !adapter.isEnabled) throw BleException(ErrorCode.BLUETOOTH_OFF, "Bluetooth is off")
  }

  private fun dispatch(request: Request, procedure: Procedure, invoke: Invoke) {
    when (request.kind) {
      "requestPermissions" -> requestPermissions(request, procedure, invoke)
      "startScan" -> scanner.start(request, procedure)
      "stopScan" -> scanner.stop(request, procedure)
      "connect" -> central.connect(request, procedure)
      "disconnect" -> central.disconnect(request, procedure)
      "discoverServices" -> central.discoverServices(request, procedure)
      "read" -> central.read(request, procedure)
      "write" -> central.write(request, procedure)
      "subscribe" -> central.subscribe(request, procedure)
      "unsubscribe" -> central.unsubscribe(request, procedure)
      "createServer" -> peripheral.create(request, procedure)
      "closeServer" -> peripheral.close(request, procedure)
      "setValue" -> peripheral.setValue(request, procedure)
      "notify" -> peripheral.notify(request, procedure)
      "startAdvertising" -> advertiser.start(request, procedure)
      "stopAdvertising" -> advertiser.stop(request, procedure)
      "cancel" -> {
        procedures.abort(request.payload().requireString("operationId"), cancelled("cancel ended the procedure"))
        procedure.resolve(Replies.empty())
      }
      "closeOwner" -> {
        procedures.abortOwner(request.ownerId, cancelled("closeOwner ended the procedure"), except = procedure)
        scanner.closeOwner(request.ownerId)
        central.closeOwner(request.ownerId)
        peripheral.closeOwner(request.ownerId)
        procedure.resolve(Replies.empty())
      }
      "debugResources" -> procedure.resolve(
        Replies.resources(
          scans = scanner.count(request.ownerId),
          connections = central.connectionCount(request.ownerId),
          subscriptions = central.subscriptionCount(request.ownerId),
          servers = peripheral.serverCount(request.ownerId),
        ),
      )
      else -> throw unsupported("the Android backend does not know ${request.kind}")
    }
  }

  private fun currentPermissions() = permissionOutcomes(Build.VERSION.SDK_INT) { permissions.permissionState(it) }

  private fun requestPermissions(request: Request, procedure: Procedure, invoke: Invoke) {
    val ask = decodePermissionAsk(request.payload())
    val aliases = aliasesToRequest(ask, Build.VERSION.SDK_INT) { permissions.permissionState(it) }
    if (aliases.isEmpty()) {
      procedure.resolve(Replies.permissions(currentPermissions()))
      return
    }
    // Tauri rejects the call itself for an undeclared permission, and never calls back.
    aliases.firstOrNull { !permissions.permissionDeclared(it) }?.let {
      throw internalError("the app manifest does not declare the permissions of $it")
    }
    procedure.armDeadline(request.deadlineMillis)
    permissionJobs.addLast(PermissionJob(procedure, invoke, aliases))
    nextPermissionRequest()
  }

  /** Tauri keeps one permission callback at a time, so the prompts run one after another. */
  private fun nextPermissionRequest() {
    if (permissionActive != null) return
    while (true) {
      val job = permissionJobs.removeFirstOrNull() ?: return
      if (job.procedure.settled) continue
      permissionActive = job
      permissions.requestPermissions(job.aliases.toTypedArray(), job.invoke) { post { permissionsAnswered(job) } }
      return
    }
  }

  private fun permissionsAnswered(job: PermissionJob) {
    if (permissionActive === job) permissionActive = null
    job.procedure.resolve(Replies.permissions(currentPermissions()))
    nextPermissionRequest()
  }

  private fun adapterChanged(state: Int) {
    val name = when (state) {
      BluetoothAdapter.STATE_ON -> "poweredOn"
      BluetoothAdapter.STATE_OFF -> "poweredOff"
      else -> return
    }
    val holders = LinkedHashSet<String>()
    holders += scanner.owners()
    holders += central.owners()
    holders += peripheral.owners()
    for (owner in holders) emit(owner, Events.adapterStateChanged(name))
    if (state == BluetoothAdapter.STATE_OFF) {
      scanner.adapterOff()
      central.adapterOff()
      advertiser.adapterOff()
      peripheral.adapterOff()
    }
  }
}
