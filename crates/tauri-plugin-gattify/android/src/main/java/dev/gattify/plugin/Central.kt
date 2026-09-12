package dev.gattify.plugin

import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothProfile
import android.os.Build
import org.json.JSONObject

/**
 * The GATT client: connections, discovery, reads, writes and subscriptions. Each
 * connection runs one GATT procedure at a time, in order, and its callbacks arrive
 * on the handler thread because `connectGatt` gets the backend handler.
 */
// The backend checks the permission of each command first. Lint cannot see the
// SecurityException catch of guard() and quietly() through a lambda, so the functions
// whose calls those helpers (or the guarded queue) wrap suppress MissingPermission.
internal class Central(private val backend: GattifyBackend) {
  private val connections = LinkedHashMap<String, Connection>()
  private val connectionsByDevice = HashMap<String, Connection>()
  private val subscriptions = HashMap<String, Subscription>()

  private enum class Phase { CONNECTING, MTU, CONNECTED, CLOSING }

  private enum class OpKind { DISCOVER, READ, WRITE, DESCRIPTOR_WRITE }

  /** One queued GATT procedure. A procedure-less op is the rollback of a cancelled subscribe. */
  private class Op(
    val kind: OpKind,
    val procedure: Procedure?,
    val characteristic: BluetoothGattCharacteristic? = null,
    val descriptor: BluetoothGattDescriptor? = null,
    val issue: (BluetoothGatt) -> Unit,
    val complete: (status: Int, value: ByteArray?) -> Unit,
    val fail: (BleException) -> Unit = { procedure?.reject(it) },
  ) {
    var watchdog: Cancellable? = null

    /** When the procedure passes its deadline, on the scheduler clock. */
    var deadlineAt: Long? = null
  }

  private class Subscription(val id: String, val owner: String, val connection: Connection, val handle: String) {
    /** The descriptor write succeeded, and Rust has the ID. */
    var active = false

    /** A cancel ended the subscribe while its descriptor write ran. The reservation holds until the rollback. */
    var cancelled = false

    /** An unsubscribe runs. No value is emitted meanwhile. */
    var disabling = false
  }

  private inner class Connection(val id: String, val owner: String, val deviceId: String) : BluetoothGattCallback() {
    var gatt: BluetoothGatt? = null
    var phase = Phase.CONNECTING
    var mtu = DEFAULT_ATT_MTU
    var connectProcedure: Procedure? = null
    var mtuTimer: Cancellable? = null
    var closeTimer: Cancellable? = null
    val disconnects = ArrayList<Procedure>()
    val handles = HandleTable<BluetoothGattCharacteristic>(id)
    val subscriptions = HashMap<String, Subscription>()
    val queue = OpQueue<Op>()

    private fun current(gatt: BluetoothGatt) = connections[id] === this && this.gatt === gatt

    override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
      if (current(gatt)) stateChanged(this, status, newState)
    }

    override fun onMtuChanged(gatt: BluetoothGatt, mtu: Int, status: Int) {
      if (current(gatt)) mtuChanged(this, mtu, status)
    }

    override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
      if (current(gatt)) finish(this, OpKind.DISCOVER, null, status, null)
    }

    override fun onCharacteristicRead(
      gatt: BluetoothGatt,
      characteristic: BluetoothGattCharacteristic,
      value: ByteArray,
      status: Int,
    ) {
      if (current(gatt)) finish(this, OpKind.READ, characteristic, status, value)
    }

    @Suppress("OVERRIDE_DEPRECATION", "DEPRECATION")
    override fun onCharacteristicRead(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, status: Int) {
      // API 33 and later call the overload with the value instead.
      if (Build.VERSION.SDK_INT < 33 && current(gatt)) finish(this, OpKind.READ, characteristic, status, characteristic.value)
    }

    override fun onCharacteristicWrite(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, status: Int) {
      if (current(gatt)) finish(this, OpKind.WRITE, characteristic, status, null)
    }

    override fun onDescriptorWrite(gatt: BluetoothGatt, descriptor: BluetoothGattDescriptor, status: Int) {
      if (current(gatt)) finish(this, OpKind.DESCRIPTOR_WRITE, descriptor, status, null)
    }

    override fun onCharacteristicChanged(
      gatt: BluetoothGatt,
      characteristic: BluetoothGattCharacteristic,
      value: ByteArray,
    ) {
      if (current(gatt)) changed(this, characteristic, value)
    }

    @Suppress("OVERRIDE_DEPRECATION", "DEPRECATION")
    override fun onCharacteristicChanged(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic) {
      if (Build.VERSION.SDK_INT < 33 && current(gatt)) changed(this, characteristic, characteristic.value ?: ByteArray(0))
    }
  }

  fun connect(request: Request, procedure: Procedure) {
    val payload = request.payload()
    val deviceId = payload.requireString("deviceId")
    val timeoutMs = (payload.opt("options") as? JSONObject)?.optionalLong("timeoutMs")
    val device = backend.knownDevice(deviceId, request.ownerId)
    if (connectionsByDevice.containsKey(deviceId)) throw busy("$deviceId already has a connection")

    val connection = Connection(backend.ids.next("connection"), request.ownerId, deviceId)
    connection.connectProcedure = procedure
    connections[connection.id] = connection
    connectionsByDevice[deviceId] = connection
    // A timeout, cancel or closeOwner cancels the attempt.
    procedure.onAbort { error -> close(connection, emit = false, error = error) }
    procedure.armDeadline(deadlineFor("connect", request.deadlineMillis, timeoutMs))

    val gatt = try {
      device.connectGatt(
        backend.context,
        false,
        connection,
        BluetoothDevice.TRANSPORT_LE,
        BluetoothDevice.PHY_LE_1M_MASK,
        backend.handler,
      )
    } catch (e: SecurityException) {
      close(connection, emit = false, error = permissionDenied(e.message ?: "the connect permission is missing"))
      return
    }
    if (gatt == null) {
      close(connection, emit = false, error = internalError("connectGatt returned no client"))
      return
    }
    connection.gatt = gatt
  }

  fun disconnect(request: Request, procedure: Procedure) {
    val connection = connection(request)
    connection.disconnects += procedure
    if (connection.phase == Phase.CLOSING) return
    connection.phase = Phase.CLOSING
    val error = disconnected("disconnect ended the procedure")
    for (op in connection.queue.drain()) end(op, error)
    endSubscriptions(connection)
    // Reply when the link closes, or after 2 s.
    val wait = minOf(Deadlines.DISCONNECT_MS, request.deadlineMillis ?: Deadlines.DISCONNECT_MS)
    connection.closeTimer = backend.scheduler.schedule(wait) { close(connection, emit = false, error = error) }
    try {
      connection.gatt?.disconnect()
    } catch (_: SecurityException) {
      close(connection, emit = false, error = error)
    }
  }

  @SuppressLint("MissingPermission")
  fun discoverServices(request: Request, procedure: Procedure) {
    val connection = connected(request)
    enqueue(
      connection,
      Op(
        OpKind.DISCOVER,
        procedure,
        issue = { gatt -> if (!gatt.discoverServices()) throw internalError("discoverServices returned false") },
        complete = { status, _ ->
          if (status == BluetoothGatt.GATT_SUCCESS) {
            procedure.resolve(Replies.services(describe(connection)))
          } else {
            procedure.reject(gattStatusError(status, "service discovery"))
          }
        },
      ),
      deadlineFor("discoverServices", request.deadlineMillis),
    )
  }

  @SuppressLint("MissingPermission")
  fun read(request: Request, procedure: Procedure) {
    val connection = connected(request)
    val characteristic = characteristic(connection, request.payload().requireString("characteristic"))
    if (!Properties.fromBits(characteristic.properties).read) throw unsupported("the characteristic is not readable")
    enqueue(
      connection,
      Op(
        OpKind.READ,
        procedure,
        characteristic = characteristic,
        issue = { gatt ->
          if (!gatt.readCharacteristic(characteristic)) throw internalError("readCharacteristic returned false")
        },
        complete = { status, value ->
          if (status == BluetoothGatt.GATT_SUCCESS) {
            procedure.resolve(Replies.bytes(value ?: ByteArray(0)))
          } else {
            procedure.reject(gattStatusError(status, "read"))
          }
        },
      ),
      deadlineFor("read", request.deadlineMillis),
    )
  }

  fun write(request: Request, procedure: Procedure) {
    val payload = request.payload()
    val connection = connected(request)
    val characteristic = characteristic(connection, payload.requireString("characteristic"))
    val value = decodeBase64(payload.requireString("valueBase64"), "valueBase64")
    val withResponse = decodeWriteType(payload.requireString("writeType"))
    // The stack sends a longer write with response as a long write, and cuts a longer write command.
    val limit = if (withResponse) MAX_ATTRIBUTE_LENGTH else valueLength(connection.mtu)
    if (value.size > limit) throw payloadTooLarge("this write carries at most $limit bytes")
    if (!Properties.fromBits(characteristic.properties).writable) throw unsupported("the characteristic is not writable")
    val writeType = if (withResponse) {
      BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
    } else {
      BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
    }
    // A write command is queued too: Android answers it with onCharacteristicWrite once the stack takes it.
    enqueue(
      connection,
      Op(
        OpKind.WRITE,
        procedure,
        characteristic = characteristic,
        issue = { gatt -> writeCharacteristic(gatt, characteristic, value, writeType) },
        complete = { status, _ ->
          if (status == BluetoothGatt.GATT_SUCCESS) {
            procedure.resolve(Replies.empty())
          } else {
            procedure.reject(gattStatusError(status, "write"))
          }
        },
      ),
      deadlineFor("write", request.deadlineMillis),
    )
  }

  @SuppressLint("MissingPermission")
  fun subscribe(request: Request, procedure: Procedure) {
    val connection = connected(request)
    val handle = request.payload().requireString("characteristic")
    val characteristic = characteristic(connection, handle)
    if (connection.subscriptions.containsKey(handle)) {
      throw busy("the characteristic already has a subscription on this connection")
    }
    val properties = Properties.fromBits(characteristic.properties)
    if (!properties.notifies) throw unsupported("the characteristic supports neither notify nor indicate")
    val cccd = characteristic.getDescriptor(CCCD_UUID)
      ?: throw unsupported("the characteristic has no client characteristic configuration descriptor")
    val enable = if (properties.notify) {
      BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
    } else {
      BluetoothGattDescriptor.ENABLE_INDICATION_VALUE
    }

    // The reservation makes another subscribe to this characteristic busy until this one ends,
    // including the rollback of a cancelled one, so the rollback never disables a later subscription.
    val subscription = Subscription(backend.ids.next("subscription"), connection.owner, connection, handle)
    connection.subscriptions[handle] = subscription
    fun release() {
      if (connection.subscriptions[handle] === subscription) connection.subscriptions.remove(handle)
    }
    fun disableLocally() {
      connection.gatt?.let { gatt -> quietly { gatt.setCharacteristicNotification(characteristic, false) } }
    }
    val op = Op(
      OpKind.DESCRIPTOR_WRITE,
      procedure,
      descriptor = cccd,
      issue = { gatt ->
        if (!gatt.setCharacteristicNotification(characteristic, true)) {
          throw internalError("setCharacteristicNotification returned false")
        }
        writeDescriptor(gatt, cccd, enable)
      },
      complete = { status, _ ->
        val success = status == BluetoothGatt.GATT_SUCCESS
        when {
          subscription.cancelled -> if (success) {
            rollback(connection, characteristic, cccd) { release() }
          } else {
            disableLocally()
            release()
          }
          success -> {
            subscription.active = true
            subscriptions[subscription.id] = subscription
            procedure.resolve(Replies.subscriptionStarted(subscription.id))
          }
          else -> {
            release()
            disableLocally()
            procedure.reject(gattStatusError(status, "subscribe"))
          }
        }
      },
      fail = { error ->
        release()
        disableLocally()
        procedure.reject(error)
      },
    )
    procedure.onAbort {
      // Before its write, the subscribe just ends. During it, the stack may still enable notifications.
      if (connection.queue.inFlight === op) subscription.cancelled = true else release()
    }
    enqueue(connection, op, deadlineFor("subscribe", request.deadlineMillis))
  }

  fun unsubscribe(request: Request, procedure: Procedure) {
    val id = request.payload().requireString("subscriptionId")
    val subscription = subscriptions[id]?.takeIf { it.owner == request.ownerId } ?: throw invalidHandle(id)
    if (subscription.disabling) throw busy("$id is already ending")
    val connection = subscription.connection
    val characteristic = connection.handles[subscription.handle]
    val cccd = characteristic?.getDescriptor(CCCD_UUID)
    if (connection.phase != Phase.CONNECTED || characteristic == null || cccd == null) {
      forget(subscription)
      procedure.resolve(Replies.empty())
      return
    }
    // The subscription stays until the peripheral stops notifying, but emits nothing more.
    subscription.disabling = true
    val op = Op(
      OpKind.DESCRIPTOR_WRITE,
      procedure,
      descriptor = cccd,
      issue = { gatt -> disableNotifications(gatt, characteristic, cccd) },
      complete = { status, _ ->
        if (status == BluetoothGatt.GATT_SUCCESS) {
          forget(subscription)
          procedure.resolve(Replies.empty())
        } else {
          unsubscribeFailed(subscription, characteristic, gattStatusError(status, "unsubscribe"), procedure)
        }
      },
      fail = { error -> unsubscribeFailed(subscription, characteristic, error, procedure) },
    )
    procedure.onAbort {
      // Before its write, the subscription stays as it was.
      if (connection.queue.inFlight !== op) subscription.disabling = false
    }
    enqueue(connection, op, deadlineFor("unsubscribe", request.deadlineMillis))
  }

  /**
   * The peripheral may still notify after a failed disable. The subscription stays
   * with its ID, and notifications flow again. When the local switch cannot be turned
   * back on, the state cannot be reconciled, so the link closes.
   */
  @SuppressLint("MissingPermission")
  private fun unsubscribeFailed(
    subscription: Subscription,
    characteristic: BluetoothGattCharacteristic,
    error: BleException,
    procedure: Procedure,
  ) {
    procedure.reject(error)
    val connection = subscription.connection
    val gatt = connection.gatt
    // A closing link ends the subscription itself.
    if (connection.phase != Phase.CONNECTED || gatt == null || subscriptions[subscription.id] !== subscription) return
    val restored = try {
      gatt.setCharacteristicNotification(characteristic, true)
    } catch (_: SecurityException) {
      false
    }
    if (restored) {
      subscription.disabling = false
    } else {
      close(connection, emit = true, error = disconnected("a failed unsubscribe left the link in an unknown state"))
    }
  }

  private fun forget(subscription: Subscription) {
    subscriptions.remove(subscription.id)
    val connection = subscription.connection
    if (connection.subscriptions[subscription.handle] === subscription) connection.subscriptions.remove(subscription.handle)
  }

  /** Closes every connection of [ownerId], or every connection when it is null. Emits nothing. */
  fun closeOwner(ownerId: String?) {
    for (connection in connections.values.filter { ownerId == null || it.owner == ownerId }) {
      close(connection, emit = false, error = cancelled("closeOwner ended the procedure"))
    }
  }

  fun adapterOff() {
    for (connection in connections.values.toList()) {
      when (connection.phase) {
        Phase.CONNECTING, Phase.MTU ->
          close(connection, emit = false, error = BleException(ErrorCode.BLUETOOTH_OFF, "Bluetooth turned off"))
        Phase.CONNECTED -> close(connection, emit = true, error = disconnected("Bluetooth turned off"))
        Phase.CLOSING -> close(connection, emit = false, error = disconnected("Bluetooth turned off"))
      }
    }
  }

  fun owners(): Set<String> = connections.values.mapTo(LinkedHashSet()) { it.owner }

  fun connectionCount(ownerId: String) = connections.values.count { it.owner == ownerId }

  fun subscriptionCount(ownerId: String) = subscriptions.values.count { it.owner == ownerId }

  /** A connection of the caller that Rust knows: connected or closing. */
  private fun connection(request: Request): Connection {
    val id = request.payload().requireString("connectionId")
    val connection = connections[id]
    val known = connection?.phase == Phase.CONNECTED || connection?.phase == Phase.CLOSING
    if (connection == null || connection.owner != request.ownerId || !known) throw invalidHandle(id)
    return connection
  }

  private fun connected(request: Request): Connection {
    val connection = connection(request)
    if (connection.phase != Phase.CONNECTED) throw disconnected("the connection is closing")
    return connection
  }

  private fun characteristic(connection: Connection, handle: String): BluetoothGattCharacteristic =
    connection.handles[handle] ?: throw invalidHandle(handle)

  private fun stateChanged(connection: Connection, status: Int, newState: Int) {
    val success = status == BluetoothGatt.GATT_SUCCESS
    if (success && newState == BluetoothProfile.STATE_CONNECTED) {
      if (connection.phase == Phase.CONNECTING) requestMtu(connection)
      return
    }
    if (success && newState != BluetoothProfile.STATE_DISCONNECTED) return
    when (connection.phase) {
      Phase.CONNECTING, Phase.MTU -> close(
        connection,
        emit = false,
        error = if (success) disconnected("the device closed the link while connecting") else gattStatusError(status, "connect"),
      )
      Phase.CONNECTED -> close(connection, emit = true, error = disconnected())
      Phase.CLOSING -> close(connection, emit = false, error = disconnected())
    }
  }

  private fun requestMtu(connection: Connection) {
    connection.phase = Phase.MTU
    val requested = try {
      connection.gatt?.requestMtu(REQUESTED_ATT_MTU) == true
    } catch (_: SecurityException) {
      false
    }
    // Without an answer the MTU stays 23, unless the stack already announced an exchange of its own.
    if (!requested) {
      connected(connection, connection.mtu)
      return
    }
    connection.mtuTimer = backend.scheduler.schedule(Deadlines.MTU_MS) { connected(connection, connection.mtu) }
  }

  private fun mtuChanged(connection: Connection, mtu: Int, status: Int) {
    if (status == BluetoothGatt.GATT_SUCCESS) connection.mtu = mtu
    if (connection.phase == Phase.MTU) connected(connection, connection.mtu)
  }

  private fun connected(connection: Connection, mtu: Int) {
    if (connections[connection.id] !== connection || connection.phase != Phase.MTU) return
    connection.mtuTimer?.cancel()
    connection.mtuTimer = null
    connection.phase = Phase.CONNECTED
    connection.mtu = mtu
    val procedure = connection.connectProcedure
    connection.connectProcedure = null
    procedure?.resolve(Replies.connected(connection.id, mtu))
  }

  private fun describe(connection: Connection): List<ServiceReport> {
    val gatt = connection.gatt ?: return emptyList()
    return gatt.services.map { service ->
      val serviceKey = "${service.uuid}#${service.instanceId}"
      ServiceReport(
        handle = connection.handles.service(serviceKey),
        uuid = uuidString(service.uuid),
        characteristics = service.characteristics.map { characteristic ->
          CharacteristicReport(
            handle = connection.handles.characteristic(characteristicKey(characteristic), characteristic),
            uuid = uuidString(characteristic.uuid),
            properties = Properties.fromBits(characteristic.properties),
          )
        },
      )
    }
  }

  private fun characteristicKey(characteristic: BluetoothGattCharacteristic): String {
    val service = characteristic.service
    return "${service?.uuid}#${service?.instanceId}/${characteristic.uuid}#${characteristic.instanceId}"
  }

  private fun changed(connection: Connection, characteristic: BluetoothGattCharacteristic, value: ByteArray) {
    val handle = connection.handles.idOf(characteristicKey(characteristic)) ?: return
    val subscription = connection.subscriptions[handle]?.takeIf { it.active && !it.disabling } ?: return
    backend.emit(subscription.owner, Events.characteristicValue(subscription.id, value))
  }

  private fun enqueue(connection: Connection, op: Op, deadline: Long?) {
    op.procedure?.let { procedure ->
      val limit = deadline ?: Deadlines.PROCEDURE_MS
      op.deadlineAt = backend.scheduler.now() + limit
      procedure.onAbort { error ->
        if (connection.queue.inFlight !== op) return@onAbort
        // A late callback must never answer a later procedure, so a stuck procedure closes the link.
        // A cancelled one keeps its place until its callback, which then answers nothing.
        if (error.code == ErrorCode.TIMEOUT) {
          close(connection, emit = true, error = disconnected("a GATT procedure passed its deadline"))
        } else {
          watch(connection, op)
        }
      }
      procedure.armDeadline(limit)
    }
    connection.queue.enqueue(op)
    pump(connection)
  }

  /** Closes the link when [op] is still in flight at its deadline, or after the default one for a rollback. */
  private fun watch(connection: Connection, op: Op) {
    val delay = op.deadlineAt?.let { (it - backend.scheduler.now()).coerceAtLeast(0) } ?: Deadlines.PROCEDURE_MS
    op.watchdog?.cancel()
    op.watchdog = backend.scheduler.schedule(delay) {
      if (connection.queue.inFlight === op) {
        close(connection, emit = true, error = disconnected("a GATT procedure passed its deadline"))
      }
    }
  }

  private fun pump(connection: Connection) {
    if (connection.phase != Phase.CONNECTED) return
    val gatt = connection.gatt ?: return
    while (true) {
      val op = connection.queue.next { it.procedure?.settled == true } ?: return
      try {
        guard { op.issue(gatt) }
        if (op.procedure == null) watch(connection, op)
        return
      } catch (e: BleException) {
        connection.queue.complete(op)
        end(op, e)
      } catch (e: RuntimeException) {
        connection.queue.complete(op)
        end(op, internalError(e.message ?: e.toString()))
      }
    }
  }

  private fun end(op: Op, error: BleException) {
    op.watchdog?.cancel()
    op.fail(error)
  }

  private fun finish(connection: Connection, kind: OpKind, attribute: Any?, status: Int, value: ByteArray?) {
    val op = connection.queue.inFlight ?: return
    if (op.kind != kind || !matches(op, attribute)) return
    connection.queue.complete(op)
    op.watchdog?.cancel()
    op.complete(status, value)
    pump(connection)
  }

  private fun matches(op: Op, attribute: Any?): Boolean = when (attribute) {
    null -> true
    is BluetoothGattCharacteristic -> op.characteristic?.let { same(it, attribute) } == true
    is BluetoothGattDescriptor -> op.descriptor?.let { it.uuid == attribute.uuid && same(it.characteristic, attribute.characteristic) } == true
    else -> false
  }

  private fun same(a: BluetoothGattCharacteristic?, b: BluetoothGattCharacteristic?): Boolean =
    a === b || (a != null && b != null && a.uuid == b.uuid && a.instanceId == b.instanceId)

  /** Undoes the descriptor write of a cancelled subscribe, ahead of every waiting procedure. Then calls [done]. */
  private fun rollback(
    connection: Connection,
    characteristic: BluetoothGattCharacteristic,
    cccd: BluetoothGattDescriptor,
    done: () -> Unit,
  ) {
    connection.queue.enqueueFirst(
      Op(
        OpKind.DESCRIPTOR_WRITE,
        null,
        descriptor = cccd,
        issue = { gatt -> disableNotifications(gatt, characteristic, cccd) },
        complete = { _, _ -> done() },
        fail = { done() },
      ),
    )
  }

  /** Disables notifications locally, then writes the CCCD. Throws when either call fails at once. */
  @SuppressLint("MissingPermission")
  private fun disableNotifications(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, cccd: BluetoothGattDescriptor) {
    if (!gatt.setCharacteristicNotification(characteristic, false)) {
      throw internalError("setCharacteristicNotification returned false")
    }
    writeDescriptor(gatt, cccd, BluetoothGattDescriptor.DISABLE_NOTIFICATION_VALUE)
  }

  private fun endSubscriptions(connection: Connection) {
    for (subscription in connection.subscriptions.values) subscriptions.remove(subscription.id)
    connection.subscriptions.clear()
  }

  /** Forgets [connection] and closes its client. Rejects its pending procedures with [error]. */
  @SuppressLint("MissingPermission")
  private fun close(connection: Connection, emit: Boolean, error: BleException) {
    if (connections[connection.id] !== connection) return
    connections.remove(connection.id)
    connectionsByDevice.remove(connection.deviceId)
    val wasClosing = connection.phase == Phase.CLOSING
    connection.phase = Phase.CLOSING
    connection.mtuTimer?.cancel()
    connection.closeTimer?.cancel()
    connection.connectProcedure?.reject(error)
    connection.connectProcedure = null
    for (op in connection.queue.drain()) end(op, error)
    endSubscriptions(connection)
    connection.gatt?.let { gatt ->
      quietly { gatt.disconnect() }
      quietly { gatt.close() }
    }
    connection.gatt = null
    for (procedure in connection.disconnects) procedure.resolve(Replies.empty())
    connection.disconnects.clear()
    if (emit && !wasClosing) backend.emit(connection.owner, Events.connectionClosed(connection.id))
  }

  @SuppressLint("MissingPermission")
  private fun writeCharacteristic(
    gatt: BluetoothGatt,
    characteristic: BluetoothGattCharacteristic,
    value: ByteArray,
    writeType: Int,
  ) {
    if (Build.VERSION.SDK_INT >= 33) {
      val code = gatt.writeCharacteristic(characteristic, value, writeType)
      if (code != STATUS_SUCCESS) throw statusCodeError(code, "write")
    } else if (!legacyWriteCharacteristic(gatt, characteristic, value, writeType)) {
      throw internalError("writeCharacteristic returned false")
    }
  }

  @Suppress("DEPRECATION")
  @SuppressLint("MissingPermission")
  private fun legacyWriteCharacteristic(
    gatt: BluetoothGatt,
    characteristic: BluetoothGattCharacteristic,
    value: ByteArray,
    writeType: Int,
  ): Boolean {
    characteristic.writeType = writeType
    characteristic.setValue(value)
    return gatt.writeCharacteristic(characteristic)
  }

  @SuppressLint("MissingPermission")
  private fun writeDescriptor(gatt: BluetoothGatt, descriptor: BluetoothGattDescriptor, value: ByteArray) {
    if (Build.VERSION.SDK_INT >= 33) {
      val code = gatt.writeDescriptor(descriptor, value)
      if (code != STATUS_SUCCESS) throw statusCodeError(code, "the descriptor write")
    } else if (!legacyWriteDescriptor(gatt, descriptor, value)) {
      throw internalError("writeDescriptor returned false")
    }
  }

  @Suppress("DEPRECATION")
  @SuppressLint("MissingPermission")
  private fun legacyWriteDescriptor(gatt: BluetoothGatt, descriptor: BluetoothGattDescriptor, value: ByteArray): Boolean {
    descriptor.setValue(value)
    return gatt.writeDescriptor(descriptor)
  }
}
