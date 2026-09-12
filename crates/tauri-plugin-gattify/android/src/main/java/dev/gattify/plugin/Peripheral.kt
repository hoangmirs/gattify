package dev.gattify.plugin

import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothGattServer
import android.bluetooth.BluetoothGattServerCallback
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothProfile
import android.os.Build
import android.util.Log
import java.util.IdentityHashMap

/** A server of one `createServer`. Its services live in the shared platform GATT server. */
internal class LocalServer(val id: String, val owner: String, val serviceUuids: Set<String>) {
  val services = ArrayList<BluetoothGattService>()
  val characteristics = LinkedHashMap<String, LocalCharacteristic>()

  /** True after Bluetooth turned off: the platform forgot the services. */
  var lost = false
}

internal class LocalCharacteristic(
  val server: LocalServer,
  val spec: CharacteristicSpec,
  val attribute: BluetoothGattCharacteristic,
  val cccd: BluetoothGattDescriptor?,
) {
  /** The value that reads return. Writes and notifications leave it unchanged. */
  var value: ByteArray = spec.initialValue
}

/**
 * The GATT servers: one shared platform server holds the services of every
 * `createServer`. Values live here, reads are served from them, and notifications
 * leave one at a time for the whole server.
 */
// The backend checks the permission of each command first. Lint cannot see the
// SecurityException catch of guard() and quietly() through a lambda, so the functions
// whose calls those helpers wrap suppress MissingPermission.
internal class Peripheral(private val backend: GattifyBackend) {
  private var platformServer: BluetoothGattServer? = null
  private var callback: ServerCallback? = null
  private val servers = LinkedHashMap<String, LocalServer>()
  private val byAttribute = IdentityHashMap<BluetoothGattCharacteristic, LocalCharacteristic>()
  private val byCccd = IdentityHashMap<BluetoothGattDescriptor, LocalCharacteristic>()
  private val centralIds = RemoteRegistry("central", backend.ids)
  private val centrals = HashMap<String, RemoteCentral>()
  private val registrations = ArrayDeque<Registration>()
  private var adding: BluetoothGattService? = null
  private var addWatchdog: Cancellable? = null
  private val notifications = OpQueue<Notification>()
  private var notificationWatchdog: Cancellable? = null

  /** A central that talks to a local server, by address. */
  private class RemoteCentral(val id: String, var device: BluetoothDevice) {
    var mtu = DEFAULT_ATT_MTU

    /** The CCCD bits of each subscribed characteristic, in the order they were enabled. */
    val subscriptions = LinkedHashMap<LocalCharacteristic, Int>()
    val prepared = PreparedWrites<LocalCharacteristic>()
  }

  /** A `createServer` that adds its services one at a time. */
  private class Registration(val server: LocalServer, val remaining: ArrayDeque<BluetoothGattService>, val procedure: Procedure) {
    var aborted = false
  }

  private class Notification(
    val central: RemoteCentral,
    val characteristic: LocalCharacteristic,
    val value: ByteArray,
    val confirm: Boolean,
    val procedure: Procedure,
  )

  /** The platform calls back on binder threads, so each callback moves to the handler. */
  private inner class ServerCallback : BluetoothGattServerCallback() {
    /** Runs [action] on the handler, unless this platform server closed meanwhile. */
    private fun post(action: () -> Unit) = backend.post { if (callback === this@ServerCallback) action() }

    override fun onConnectionStateChange(device: BluetoothDevice, status: Int, newState: Int) =
      post { connectionChanged(device, newState) }

    override fun onServiceAdded(status: Int, service: BluetoothGattService) = post { serviceAdded(status, service) }

    override fun onMtuChanged(device: BluetoothDevice, mtu: Int) = post { mtuChanged(device, mtu) }

    override fun onCharacteristicReadRequest(
      device: BluetoothDevice,
      requestId: Int,
      offset: Int,
      characteristic: BluetoothGattCharacteristic,
    ) = post { readRequest(device, requestId, offset, characteristic) }

    override fun onCharacteristicWriteRequest(
      device: BluetoothDevice,
      requestId: Int,
      characteristic: BluetoothGattCharacteristic,
      preparedWrite: Boolean,
      responseNeeded: Boolean,
      offset: Int,
      value: ByteArray?,
    ) {
      val bytes = value?.copyOf() ?: ByteArray(0)
      post { writeRequest(device, requestId, characteristic, preparedWrite, responseNeeded, offset, bytes) }
    }

    override fun onDescriptorReadRequest(
      device: BluetoothDevice,
      requestId: Int,
      offset: Int,
      descriptor: BluetoothGattDescriptor,
    ) = post { descriptorReadRequest(device, requestId, offset, descriptor) }

    override fun onDescriptorWriteRequest(
      device: BluetoothDevice,
      requestId: Int,
      descriptor: BluetoothGattDescriptor,
      preparedWrite: Boolean,
      responseNeeded: Boolean,
      offset: Int,
      value: ByteArray?,
    ) {
      val bytes = value?.copyOf() ?: ByteArray(0)
      post { descriptorWriteRequest(device, requestId, descriptor, preparedWrite, responseNeeded, offset, bytes) }
    }

    override fun onExecuteWrite(device: BluetoothDevice, requestId: Int, execute: Boolean) =
      post { executeWrite(device, requestId, execute) }

    override fun onNotificationSent(device: BluetoothDevice, status: Int) = post { notificationSent(device, status) }
  }

  fun create(request: Request, procedure: Procedure) {
    val specs = decodeServer(request.payload())
    val uuids = specs.mapTo(HashSet()) { uuidString(it.uuid) }
    // A lost server no longer holds its services on the platform.
    val taken = servers.values.filter { !it.lost }.map { it.serviceUuids } + registrations.map { it.server.serviceUuids }
    if (taken.any { it.any(uuids::contains) }) throw busy("another server already registered one of these service UUIDs")

    openServer()
    val server = LocalServer(backend.ids.next("server"), request.ownerId, uuids)
    val services = ArrayDeque<BluetoothGattService>()
    for (spec in specs) {
      val type = if (spec.primary) BluetoothGattService.SERVICE_TYPE_PRIMARY else BluetoothGattService.SERVICE_TYPE_SECONDARY
      val service = BluetoothGattService(spec.uuid, type)
      for (characteristic in spec.characteristics) {
        val attribute = BluetoothGattCharacteristic(
          characteristic.uuid,
          characteristic.properties.bits,
          characteristic.properties.permissions,
        )
        val cccd = if (characteristic.properties.notifies) {
          BluetoothGattDescriptor(CCCD_UUID, PERMISSION_READ or PERMISSION_WRITE).also { attribute.addDescriptor(it) }
        } else {
          null
        }
        service.addCharacteristic(attribute)
        server.characteristics[characteristic.key] = LocalCharacteristic(server, characteristic, attribute, cccd)
      }
      services.addLast(service)
    }

    val registration = Registration(server, services, procedure)
    registrations.addLast(registration)
    procedure.onAbort { abortRegistration(registration) }
    procedure.armDeadline(deadlineFor("createServer", request.deadlineMillis))
    nextService()
  }

  fun close(request: Request, procedure: Procedure) {
    release(owned(request, allowLost = true), cancelled("closeServer ended the procedure"))
    procedure.resolve(Replies.empty())
  }

  fun setValue(request: Request, procedure: Procedure) {
    val payload = request.payload()
    val server = owned(request, allowLost = false)
    val characteristic = characteristicOf(server, payload.requireString("characteristicKey"))
    val value = decodeBase64(payload.requireString("valueBase64"), "valueBase64")
    if (value.size > characteristic.spec.maxValueLength) {
      throw payloadTooLarge("the value is longer than maxValueLength ${characteristic.spec.maxValueLength}")
    }
    characteristic.value = value
    procedure.resolve(Replies.empty())
  }

  fun notify(request: Request, procedure: Procedure) {
    val payload = request.payload()
    val server = owned(request, allowLost = false)
    val characteristic = characteristicOf(server, payload.requireString("characteristicKey"))
    val peerId = payload.requireString("peerId")
    val value = decodeBase64(payload.requireString("valueBase64"), "valueBase64")
    val central = centrals.values.firstOrNull { it.id == peerId } ?: throw invalidHandle(peerId)
    val bits = central.subscriptions[characteristic]
      ?: throw BleException(ErrorCode.INVALID_HANDLE, "$peerId has no subscription to ${characteristic.spec.key}")
    val limit = valueLength(central.mtu)
    if (value.size > limit) throw payloadTooLarge("a notification to $peerId carries at most $limit bytes")

    val notification = Notification(central, characteristic, value, confirms(bits), procedure)
    procedure.onAbort {
      // A notification in flight keeps its place until the stack answers, or a short grace passes.
      if (notifications.inFlight === notification) watchNotification(notification)
    }
    procedure.armDeadline(deadlineFor("notify", request.deadlineMillis))
    notifications.enqueue(notification)
    nextNotification()
  }

  /** Closes every server of [ownerId], or every server when it is null. Emits nothing. */
  fun closeOwner(ownerId: String?) {
    for (server in servers.values.filter { ownerId == null || it.owner == ownerId }) {
      release(server, cancelled("closeOwner ended the procedure"))
    }
  }

  /** Bluetooth turned off: the platform server and its services are gone. */
  fun adapterOff() {
    for (registration in registrations.toList()) {
      registration.procedure.reject(BleException(ErrorCode.BLUETOOTH_OFF, "Bluetooth turned off"))
    }
    registrations.clear()
    adding = null
    addWatchdog?.cancel()
    for (notification in notifications.drain()) notification.procedure.reject(disconnected("Bluetooth turned off"))
    notificationWatchdog?.cancel()
    for (central in centrals.values) endSubscriptions(central)
    centrals.clear()
    for (server in servers.values) {
      if (server.lost) continue
      server.lost = true
      forgetAttributes(server)
      server.services.clear()
      backend.emit(server.owner, Events.criticalStateLoss(server.id, "bluetoothOff"))
    }
    closePlatformServer()
  }

  fun owners(): Set<String> = servers.values.mapTo(LinkedHashSet()) { it.owner }

  fun serverCount(ownerId: String) = servers.values.count { it.owner == ownerId }

  /** A server of the caller. A lost server is stale for every command but closeServer. */
  fun owned(request: Request, allowLost: Boolean): LocalServer {
    val id = request.payload().requireString("serverId")
    val server = servers[id]?.takeIf { it.owner == request.ownerId } ?: throw invalidHandle(id)
    if (server.lost && !allowLost) throw BleException(ErrorCode.INVALID_HANDLE, "$id lost its registration")
    return server
  }

  private fun characteristicOf(server: LocalServer, key: String): LocalCharacteristic =
    server.characteristics[key] ?: throw invalidArgument("the server has no characteristic $key")

  @SuppressLint("MissingPermission")
  private fun openServer(): BluetoothGattServer {
    platformServer?.let { return it }
    val manager = backend.manager ?: throw BleException(ErrorCode.UNAVAILABLE, "this device has no Bluetooth adapter")
    val serverCallback = ServerCallback()
    val opened = guard { manager.openGattServer(backend.context, serverCallback) }
      ?: throw internalError("openGattServer returned no server")
    platformServer = opened
    callback = serverCallback
    return opened
  }

  @SuppressLint("MissingPermission")
  private fun closePlatformServer() {
    val platform = platformServer ?: return
    platformServer = null
    callback = null
    quietly { platform.close() }
  }

  /** Adds the next service. The platform takes one service at a time, across every registration. */
  private fun nextService() {
    val platform = platformServer ?: return
    while (adding == null) {
      val registration = registrations.firstOrNull() ?: return
      val service = registration.remaining.removeFirstOrNull()
      if (service == null) {
        registrations.removeFirst()
        registered(registration)
        continue
      }
      val accepted = try {
        platform.addService(service)
      } catch (_: SecurityException) {
        false
      }
      if (!accepted) {
        registrations.removeFirst()
        failRegistration(registration, internalError("addService returned false"))
        continue
      }
      adding = service
      addWatchdog = backend.scheduler.schedule(Deadlines.PROCEDURE_MS) {
        if (adding === service) serviceAdded(BluetoothGatt.GATT_FAILURE, service)
      }
    }
  }

  private fun serviceAdded(status: Int, service: BluetoothGattService) {
    val expected = adding ?: return
    if (expected !== service && expected.uuid != service.uuid) return
    adding = null
    addWatchdog?.cancel()
    val registration = registrations.firstOrNull() ?: return nextService()
    if (status == BluetoothGatt.GATT_SUCCESS) {
      registration.server.services += expected
    } else {
      registrations.removeFirst()
      failRegistration(registration, gattStatusError(status, "addService"))
    }
    nextService()
  }

  private fun registered(registration: Registration) {
    val server = registration.server
    if (registration.aborted) {
      removeServices(server)
      closeIfIdle()
      return
    }
    servers[server.id] = server
    for (characteristic in server.characteristics.values) {
      byAttribute[characteristic.attribute] = characteristic
      characteristic.cccd?.let { byCccd[it] = characteristic }
    }
    registration.procedure.resolve(Replies.serverCreated(server.id))
  }

  private fun failRegistration(registration: Registration, error: BleException) {
    removeServices(registration.server)
    registration.procedure.reject(error)
    closeIfIdle()
  }

  private fun abortRegistration(registration: Registration) {
    registration.aborted = true
    registration.remaining.clear()
    // The service in flight finishes first. Its callback then removes the added services.
    if (registrations.firstOrNull() === registration && adding != null) return
    registrations.remove(registration)
    removeServices(registration.server)
    closeIfIdle()
    nextService()
  }

  @SuppressLint("MissingPermission")
  private fun removeServices(server: LocalServer) {
    val platform = platformServer
    if (platform != null) for (service in server.services) quietly { platform.removeService(service) }
    server.services.clear()
  }

  private fun forgetAttributes(server: LocalServer) {
    for (characteristic in server.characteristics.values) {
      byAttribute.remove(characteristic.attribute)
      characteristic.cccd?.let { byCccd.remove(it) }
    }
  }

  /** Stops the advertisement of [server], removes its services and forgets its subscribers. */
  private fun release(server: LocalServer, error: BleException) {
    if (servers[server.id] !== server) return
    servers.remove(server.id)
    backend.advertiser.stopFor(server)
    forgetAttributes(server)
    removeServices(server)
    for (central in centrals.values) {
      central.subscriptions.keys.removeAll { it.server === server }
      central.prepared.discard { it.server === server }
    }
    for (notification in notifications.removeWaiting { it.characteristic.server === server }) {
      notification.procedure.reject(error)
    }
    notifications.inFlight?.takeIf { it.characteristic.server === server }?.let { notification ->
      notification.procedure.reject(error)
      watchNotification(notification)
    }
    closeIfIdle()
  }

  /** Closes the platform server when no server and no registration needs it. A lost server needs nothing. */
  private fun closeIfIdle() {
    if (servers.values.any { !it.lost } || registrations.isNotEmpty()) return
    for (notification in notifications.drain()) notification.procedure.reject(cancelled("the server closed"))
    notificationWatchdog?.cancel()
    centrals.clear()
    adding = null
    addWatchdog?.cancel()
    closePlatformServer()
  }

  private fun central(device: BluetoothDevice): RemoteCentral =
    centrals.getOrPut(device.address) { RemoteCentral(centralIds.idFor(device.address), device) }
      .also { it.device = device }

  private fun registeredCharacteristic(attribute: BluetoothGattCharacteristic): LocalCharacteristic? =
    byAttribute[attribute]?.takeIf { servers[it.server.id] === it.server }

  @SuppressLint("MissingPermission")
  private fun respond(device: BluetoothDevice, requestId: Int, status: Int, offset: Int, value: ByteArray?) {
    val platform = platformServer ?: return
    // Some Android versions fail on a null value, so an empty answer carries an empty array.
    quietly { platform.sendResponse(device, requestId, status, offset, value ?: ByteArray(0)) }
  }

  private fun connectionChanged(device: BluetoothDevice, newState: Int) {
    when (newState) {
      BluetoothProfile.STATE_CONNECTED -> central(device)
      BluetoothProfile.STATE_DISCONNECTED -> {
        val central = centrals.remove(device.address) ?: return
        central.prepared.clear()
        val error = disconnected("${central.id} disconnected")
        for (notification in notifications.removeWaiting { it.central === central }) notification.procedure.reject(error)
        notifications.inFlight?.takeIf { it.central === central }?.let { notificationDone(it, error) }
        endSubscriptions(central)
      }
    }
  }

  /** Emits `subscribed: false` for each subscription of [central]. */
  private fun endSubscriptions(central: RemoteCentral) {
    val subscribed = central.subscriptions.keys.toList()
    central.subscriptions.clear()
    for (characteristic in subscribed) {
      val server = characteristic.server
      if (servers[server.id] !== server || server.lost) continue
      backend.emit(server.owner, Events.subscriptionChanged(server.id, central.id, characteristic.spec.key, null))
    }
  }

  private fun mtuChanged(device: BluetoothDevice, mtu: Int) {
    val central = central(device)
    val before = valueLength(central.mtu)
    central.mtu = mtu
    val after = valueLength(mtu)
    if (before == after) return
    for (characteristic in central.subscriptions.keys) {
      val server = characteristic.server
      if (servers[server.id] !== server) continue
      backend.emit(server.owner, Events.subscriptionChanged(server.id, central.id, characteristic.spec.key, after))
    }
  }

  private fun readRequest(device: BluetoothDevice, requestId: Int, offset: Int, attribute: BluetoothGattCharacteristic) {
    val characteristic = registeredCharacteristic(attribute)
    val (status, bytes) = if (characteristic == null) {
      Att.READ_NOT_PERMITTED to null
    } else {
      readAnswer(characteristic.spec.properties.read, characteristic.value, offset)
    }
    respond(device, requestId, status, offset, bytes)
  }

  private fun writeRequest(
    device: BluetoothDevice,
    requestId: Int,
    attribute: BluetoothGattCharacteristic,
    preparedWrite: Boolean,
    responseNeeded: Boolean,
    offset: Int,
    value: ByteArray,
  ) {
    val central = central(device)
    val characteristic = registeredCharacteristic(attribute)
    if (characteristic == null) {
      if (preparedWrite) central.prepared.fail(Att.WRITE_NOT_PERMITTED)
      if (responseNeeded) respond(device, requestId, Att.WRITE_NOT_PERMITTED, offset, null)
      return
    }
    val spec = characteristic.spec
    if (preparedWrite) {
      // A part of a long write: answer it with its own value and offset, and emit on execute.
      val status = writeStatus(spec.properties, spec.maxValueLength, central.prepared.length(characteristic), offset, value.size)
      if (status == Att.SUCCESS) central.prepared.append(characteristic, value) else central.prepared.fail(status)
      if (responseNeeded) respond(device, requestId, status, offset, if (status == Att.SUCCESS) value else null)
      return
    }
    val status = writeStatus(spec.properties, spec.maxValueLength, 0, offset, value.size)
    if (responseNeeded) respond(device, requestId, status, offset, if (status == Att.SUCCESS) value else null)
    if (status == Att.SUCCESS) emitWrite(central, characteristic, value)
  }

  private fun executeWrite(device: BluetoothDevice, requestId: Int, execute: Boolean) {
    val central = central(device)
    val (status, writes) = central.prepared.finish(execute)
    for ((characteristic, value) in writes) {
      if (servers[characteristic.server.id] === characteristic.server) emitWrite(central, characteristic, value)
    }
    respond(device, requestId, status, 0, null)
  }

  private fun emitWrite(central: RemoteCentral, characteristic: LocalCharacteristic, value: ByteArray) {
    val server = characteristic.server
    backend.emit(server.owner, Events.serverWrite(server.id, central.id, characteristic.spec.key, value))
  }

  private fun descriptorReadRequest(device: BluetoothDevice, requestId: Int, offset: Int, descriptor: BluetoothGattDescriptor) {
    val characteristic = byCccd[descriptor]?.takeIf { servers[it.server.id] === it.server }
    if (characteristic == null) {
      respond(device, requestId, Att.READ_NOT_PERMITTED, offset, null)
      return
    }
    val bits = centrals[device.address]?.subscriptions?.get(characteristic) ?: 0
    val (status, bytes) = readAnswer(true, cccdValue(bits), offset)
    respond(device, requestId, status, offset, bytes)
  }

  private fun descriptorWriteRequest(
    device: BluetoothDevice,
    requestId: Int,
    descriptor: BluetoothGattDescriptor,
    preparedWrite: Boolean,
    responseNeeded: Boolean,
    offset: Int,
    value: ByteArray,
  ) {
    val characteristic = byCccd[descriptor]?.takeIf { servers[it.server.id] === it.server }
    val bits = cccdBits(value)
    val status = when {
      characteristic == null -> Att.WRITE_NOT_PERMITTED
      preparedWrite -> Att.REQUEST_NOT_SUPPORTED
      offset != 0 -> Att.INVALID_OFFSET
      bits == null -> Att.INVALID_ATTRIBUTE_LENGTH
      else -> Att.SUCCESS
    }
    if (responseNeeded) respond(device, requestId, status, offset, if (status == Att.SUCCESS) value else null)
    if (characteristic == null || bits == null || status != Att.SUCCESS) return

    val central = central(device)
    val server = characteristic.server
    val subscribed = central.subscriptions.containsKey(characteristic)
    if (bits == 0) {
      central.subscriptions.remove(characteristic)
      if (subscribed) {
        backend.emit(server.owner, Events.subscriptionChanged(server.id, central.id, characteristic.spec.key, null))
      }
    } else {
      central.subscriptions[characteristic] = bits
      if (!subscribed) {
        backend.emit(
          server.owner,
          Events.subscriptionChanged(server.id, central.id, characteristic.spec.key, valueLength(central.mtu)),
        )
      }
    }
  }

  /** Sends the next notification. One notification is outstanding for the whole server. */
  private fun nextNotification() {
    val platform = platformServer ?: return
    while (true) {
      val notification = notifications.next { it.procedure.settled } ?: return
      try {
        guard { send(platform, notification) }
        return
      } catch (e: BleException) {
        notifications.complete(notification)
        notification.procedure.reject(e)
      } catch (e: RuntimeException) {
        notifications.complete(notification)
        notification.procedure.reject(internalError(e.message ?: e.toString()))
      }
    }
  }

  /** Called inside guard(). */
  @SuppressLint("MissingPermission")
  private fun send(platform: BluetoothGattServer, notification: Notification) {
    val device = notification.central.device
    val attribute = notification.characteristic.attribute
    if (Build.VERSION.SDK_INT >= 33) {
      val code = platform.notifyCharacteristicChanged(device, attribute, notification.confirm, notification.value)
      if (code != STATUS_SUCCESS) throw statusCodeError(code, "notify")
    } else if (!legacyNotify(platform, device, attribute, notification.confirm, notification.value)) {
      throw internalError("notifyCharacteristicChanged returned false")
    }
  }

  /** The platform attribute value changes here, but reads answer from [LocalCharacteristic.value]. */
  @Suppress("DEPRECATION")
  @SuppressLint("MissingPermission")
  private fun legacyNotify(
    platform: BluetoothGattServer,
    device: BluetoothDevice,
    attribute: BluetoothGattCharacteristic,
    confirm: Boolean,
    value: ByteArray,
  ): Boolean {
    attribute.setValue(value)
    return platform.notifyCharacteristicChanged(device, attribute, confirm)
  }

  private fun notificationSent(device: BluetoothDevice, status: Int) {
    val notification = notifications.inFlight ?: return
    if (notification.central.device.address != device.address) return
    notificationDone(
      notification,
      if (status == BluetoothGatt.GATT_SUCCESS) null else gattStatusError(status, "notify"),
    )
  }

  private fun notificationDone(notification: Notification, error: BleException?) {
    if (!notifications.complete(notification)) return
    notificationWatchdog?.cancel()
    if (error == null) notification.procedure.resolve(Replies.empty()) else notification.procedure.reject(error)
    nextNotification()
  }

  /** Frees the queue when the stack never answers a notification that already timed out. */
  private fun watchNotification(notification: Notification) {
    notificationWatchdog?.cancel()
    notificationWatchdog = backend.scheduler.schedule(NOTIFICATION_GRACE_MS) {
      if (notifications.inFlight === notification) {
        Log.w(TAG, "the stack did not confirm a notification to ${notification.central.id}")
        notificationDone(notification, BleException(ErrorCode.TIMEOUT, "the deadline passed"))
      }
    }
  }

  private companion object {
    const val NOTIFICATION_GRACE_MS = 5_000L
  }
}
