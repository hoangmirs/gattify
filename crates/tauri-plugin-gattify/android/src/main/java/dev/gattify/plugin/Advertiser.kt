package dev.gattify.plugin

import android.annotation.SuppressLint
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.bluetooth.le.BluetoothLeAdvertiser
import android.os.ParcelUuid
import android.util.Log

/**
 * The one advertisement of the process: legacy, connectable, with the service UUID
 * in the advertisement and the local name as service data in the scan response.
 */
// The backend checks the advertise permission first. Lint cannot see the
// SecurityException catch of guard() and quietly() through a lambda, so the functions
// that use them suppress MissingPermission.
internal class Advertiser(private val backend: GattifyBackend) {
  private var current: Advertisement? = null
  /** The copy a rearm starts. It takes the place of [current] only once it runs. */
  private var rearming: Advertisement? = null

  /** The platform calls back on the main thread, so each callback moves to the handler. */
  private inner class Advertisement(
    val server: LocalServer,
    val platformAdvertiser: BluetoothLeAdvertiser,
    val nameIncluded: Boolean,
    val nameTruncated: Boolean,
    val settings: AdvertiseSettings,
    val data: AdvertiseData,
    val scanResponse: AdvertiseData,
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
    val advertisement = Advertisement(
      server,
      platformAdvertiser,
      nameBytes.isNotEmpty(),
      name?.truncated == true,
      settings,
      data,
      scanResponse.build(),
    )
    advertisement.procedure = procedure
    current = advertisement
    procedure.onAbort { if (current === advertisement) stop(advertisement) }
    procedure.armDeadline(deadlineFor("startAdvertising", request.deadlineMillis))
    guard { platformAdvertiser.startAdvertising(settings, data, advertisement.scanResponse, advertisement) }
  }

  /**
   * A central connected. Some devices stop a connectable advertisement then, and a second
   * central would find nothing. A copy with the same data starts, and the old one stops only
   * once the copy runs: a copy the device refuses leaves the old one as it was.
   */
  @SuppressLint("MissingPermission")
  fun rearm() {
    val old = current?.takeIf { it.procedure == null } ?: return
    if (rearming != null) return
    val copy = Advertisement(
      old.server,
      old.platformAdvertiser,
      old.nameIncluded,
      old.nameTruncated,
      old.settings,
      old.data,
      old.scanResponse,
    )
    rearming = copy
    quietly { copy.platformAdvertiser.startAdvertising(copy.settings, copy.data, copy.scanResponse, copy) }
  }

  /** Stops a rearm's copy that has not taken over yet. */
  @SuppressLint("MissingPermission")
  private fun dropRearm() {
    val copy = rearming ?: return
    rearming = null
    quietly { copy.platformAdvertiser.stopAdvertising(copy) }
  }

  fun stop(request: Request, procedure: Procedure) {
    stopFor(backend.peripheral.owned(request, allowLost = true))
    procedure.resolve(Replies.empty())
  }

  /** Stops the advertisement of [server], if it has one. */
  fun stopFor(server: LocalServer) {
    if (rearming?.server === server) dropRearm()
    val advertisement = current?.takeIf { it.server === server } ?: return
    advertisement.procedure?.reject(cancelled("the advertisement stopped"))
    advertisement.procedure = null
    stop(advertisement)
  }

  fun adapterOff() {
    rearming = null
    val advertisement = current ?: return
    current = null
    advertisement.procedure?.reject(BleException(ErrorCode.BLUETOOTH_OFF, "Bluetooth turned off"))
    advertisement.procedure = null
  }

  @SuppressLint("MissingPermission")
  private fun stop(advertisement: Advertisement) {
    if (current === advertisement) {
      current = null
      dropRearm()
    }
    quietly { advertisement.platformAdvertiser.stopAdvertising(advertisement) }
  }

  @SuppressLint("MissingPermission")
  private fun started(advertisement: Advertisement) {
    if (rearming === advertisement) {
      rearming = null
      val old = current
      if (old == null || old.procedure != null || old.server !== advertisement.server) {
        quietly { advertisement.platformAdvertiser.stopAdvertising(advertisement) }
        return
      }
      current = advertisement
      quietly { old.platformAdvertiser.stopAdvertising(old) }
      return
    }
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
    if (rearming === advertisement) {
      // The old advertisement stays as it was. A device with one advertising slot refuses the
      // copy while the old one still runs, which is the case the copy was not needed for.
      rearming = null
      Log.w(TAG, "a rearm's advertisement failed with $errorCode")
      return
    }
    if (current !== advertisement) return
    current = null
    advertisement.procedure?.reject(advertiseFailure(errorCode))
    advertisement.procedure = null
  }
}
