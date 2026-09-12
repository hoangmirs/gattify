package dev.gattify.plugin

import android.annotation.SuppressLint
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.bluetooth.le.BluetoothLeAdvertiser
import android.os.ParcelUuid

/**
 * The one advertisement of the process: legacy, connectable, with the service UUID
 * in the advertisement and the local name as service data in the scan response.
 */
// The backend checks the advertise permission first. Lint cannot see the
// SecurityException catch of guard() and quietly() through a lambda, so the functions
// that use them suppress MissingPermission.
internal class Advertiser(private val backend: GattifyBackend) {
  private var current: Advertisement? = null

  /** The platform calls back on the main thread, so each callback moves to the handler. */
  private inner class Advertisement(
    val server: LocalServer,
    val platformAdvertiser: BluetoothLeAdvertiser,
    val nameIncluded: Boolean,
    val nameTruncated: Boolean,
  ) : AdvertiseCallback() {
    /** The pending `startAdvertising`, until the platform answers. */
    var procedure: Procedure? = null

    override fun onStartSuccess(settingsInEffect: AdvertiseSettings) {
      backend.post { started(this) }
    }

    override fun onStartFailure(errorCode: Int) {
      backend.post { failed(this, errorCode) }
    }
  }

  fun start(request: Request, procedure: Procedure) {
    val payload = request.payload()
    val server = backend.peripheral.owned(request, allowLost = false)
    val spec = decodeAdvertising(payload.requireObject("options"))
    val serviceUuid = uuidString(spec.serviceUuid)
    if (serviceUuid !in server.serviceUuids) throw invalidArgument("the server has no service $serviceUuid")
    val existing = current
    if (existing != null && existing.server !== server) throw busy("another server advertises")
    if (existing?.procedure != null) throw busy("the advertisement of this server is starting")
    val name = spec.localName?.let { cutUtf8(it, ADVERTISED_NAME_BUDGET) }
    if (name != null && name.truncated && !spec.localNameOptional) {
      throw payloadTooLarge("the local name is longer than $ADVERTISED_NAME_BUDGET bytes")
    }
    val platformAdvertiser = backend.adapter?.bluetoothLeAdvertiser ?: throw unsupported("this device cannot advertise")

    // The same server advertises again: restart with the new options.
    existing?.let { stop(it) }
    val nameBytes = name?.bytes ?: ByteArray(0)
    val advertisement = Advertisement(server, platformAdvertiser, nameBytes.isNotEmpty(), name?.truncated == true)
    advertisement.procedure = procedure
    current = advertisement
    procedure.onAbort { if (current === advertisement) stop(advertisement) }
    procedure.armDeadline(deadlineFor("startAdvertising", request.deadlineMillis))

    val settings = AdvertiseSettings.Builder()
      .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_LATENCY)
      .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_HIGH)
      .setConnectable(true)
      .setTimeout(0)
      .build()
    val uuid = ParcelUuid(spec.serviceUuid)
    // setIncludeDeviceName would send the system device name, so the local name goes in the scan response.
    val data = AdvertiseData.Builder()
      .addServiceUuid(uuid)
      .setIncludeDeviceName(false)
      .setIncludeTxPowerLevel(false)
      .build()
    val scanResponse = AdvertiseData.Builder().setIncludeDeviceName(false).setIncludeTxPowerLevel(false)
    if (nameBytes.isNotEmpty()) scanResponse.addServiceData(uuid, nameBytes)
    guard { platformAdvertiser.startAdvertising(settings, data, scanResponse.build(), advertisement) }
  }

  fun stop(request: Request, procedure: Procedure) {
    stopFor(backend.peripheral.owned(request, allowLost = true))
    procedure.resolve(Replies.empty())
  }

  /** Stops the advertisement of [server], if it has one. */
  fun stopFor(server: LocalServer) {
    val advertisement = current?.takeIf { it.server === server } ?: return
    advertisement.procedure?.reject(cancelled("the advertisement stopped"))
    advertisement.procedure = null
    stop(advertisement)
  }

  fun adapterOff() {
    val advertisement = current ?: return
    current = null
    advertisement.procedure?.reject(BleException(ErrorCode.BLUETOOTH_OFF, "Bluetooth turned off"))
    advertisement.procedure = null
  }

  @SuppressLint("MissingPermission")
  private fun stop(advertisement: Advertisement) {
    if (current === advertisement) current = null
    quietly { advertisement.platformAdvertiser.stopAdvertising(advertisement) }
  }

  @SuppressLint("MissingPermission")
  private fun started(advertisement: Advertisement) {
    if (current !== advertisement) {
      // A timeout or a stop ended it before the platform answered.
      quietly { advertisement.platformAdvertiser.stopAdvertising(advertisement) }
      return
    }
    val procedure = advertisement.procedure
    advertisement.procedure = null
    procedure?.resolve(Replies.advertisingStarted(advertisement.nameIncluded, advertisement.nameTruncated))
  }

  private fun failed(advertisement: Advertisement, errorCode: Int) {
    if (current !== advertisement) return
    current = null
    advertisement.procedure?.reject(advertiseFailure(errorCode))
    advertisement.procedure = null
  }
}
