package dev.gattify.plugin

import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.le.BluetoothLeScanner
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanRecord
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.os.ParcelUuid
import android.util.Log

/**
 * The scans. Each scan has its own platform scan with one filter per service UUID,
 * so several scans run at once without restarts.
 */
// The backend checks the scan permission before startScan. Lint cannot see the
// SecurityException catch of guard() and quietly() through a lambda, so the functions
// that use them suppress MissingPermission.
internal class Scanner(private val backend: GattifyBackend) {
  private val scans = LinkedHashMap<String, Scan>()

  /** A scan. The platform calls back on the main thread, so each callback moves to the handler. */
  private inner class Scan(val id: String, val owner: String, val filter: Set<String>) : ScanCallback() {
    val throttle = Throttle(SCAN_RESULT_INTERVAL_MS)
    var platformScanner: BluetoothLeScanner? = null
    var timer: Cancellable? = null

    override fun onScanResult(callbackType: Int, result: ScanResult) {
      backend.post { deliver(this, result) }
    }

    override fun onBatchScanResults(results: MutableList<ScanResult>) {
      val copy = results.toList()
      backend.post { copy.forEach { deliver(this, it) } }
    }

    override fun onScanFailed(errorCode: Int) {
      backend.post {
        Log.w(TAG, "scan $id failed with error $errorCode")
        end(this, emit = true)
      }
    }
  }

  @SuppressLint("MissingPermission")
  fun start(request: Request, procedure: Procedure) {
    val spec = decodeScan(request.payload())
    val platformScanner = backend.adapter?.bluetoothLeScanner
      ?: throw BleException(ErrorCode.BLUETOOTH_OFF, "Bluetooth is off")
    val scan = Scan(backend.ids.next("scan"), request.ownerId, spec.serviceUuids.mapTo(HashSet(), ::uuidString))
    val filters = spec.serviceUuids.map { ScanFilter.Builder().setServiceUuid(ParcelUuid(it)).build() }
    val settings = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build()
    guard { platformScanner.startScan(filters, settings, scan) }
    scan.platformScanner = platformScanner
    scans[scan.id] = scan
    spec.timeoutMs?.let { timeout -> scan.timer = backend.scheduler.schedule(timeout) { end(scan, emit = true) } }
    procedure.resolve(Replies.scanStarted(scan.id))
  }

  fun stop(request: Request, procedure: Procedure) {
    val id = request.payload().requireString("scanId")
    val scan = scans[id]?.takeIf { it.owner == request.ownerId } ?: throw invalidHandle(id)
    end(scan, emit = false)
    procedure.resolve(Replies.empty())
  }

  /** Stops every scan of [ownerId], or every scan when it is null. Emits nothing. */
  fun closeOwner(ownerId: String?) {
    for (scan in scans.values.filter { ownerId == null || it.owner == ownerId }) end(scan, emit = false)
  }

  fun adapterOff() {
    for (scan in scans.values.toList()) end(scan, emit = true)
  }

  fun owners(): Set<String> = scans.values.mapTo(LinkedHashSet()) { it.owner }

  fun count(ownerId: String) = scans.values.count { it.owner == ownerId }

  @SuppressLint("MissingPermission")
  private fun end(scan: Scan, emit: Boolean) {
    if (scans[scan.id] !== scan) return
    scans.remove(scan.id)
    scan.timer?.cancel()
    scan.platformScanner?.let { platformScanner -> quietly { platformScanner.stopScan(scan) } }
    if (emit) backend.emit(scan.owner, Events.scanStopped(scan.id))
  }

  private fun deliver(scan: Scan, result: ScanResult) {
    if (scans[scan.id] !== scan) return
    val record = result.scanRecord
    val serviceUuids = record?.serviceUuids.orEmpty().map { uuidString(it.uuid) }
    val serviceData = record?.serviceData.orEmpty().map { (uuid, bytes) -> uuidString(uuid.uuid) to (bytes ?: ByteArray(0)) }
    if (!matchesFilter(scan.filter, serviceUuids, serviceData.map { it.first })) return

    val device = result.device
    val deviceId = backend.devices.idFor(device.address)
    if (!scan.throttle.allow(deviceId, backend.scheduler.now())) return
    backend.rememberDevice(deviceId, device)
    backend.devices.markSeen(deviceId, scan.owner)

    val localName = record?.deviceName
    val report = DeviceReport(
      id = deviceId,
      name = scanName(localName, serviceData, scan.filter, cachedName(device)),
      rssi = result.rssi,
      serviceUuids = serviceUuids,
      localName = localName,
      serviceData = serviceData,
      manufacturerData = manufacturerData(record),
      connectable = result.isConnectable,
      observedAtMillis = System.currentTimeMillis(),
      scanId = scan.id,
    )
    backend.emit(scan.owner, Events.scanResult(report))
  }

  private fun cachedName(device: BluetoothDevice): String? = try {
    device.name
  } catch (_: SecurityException) {
    null
  }

  private fun manufacturerData(record: ScanRecord?): List<Pair<Int, ByteArray>> {
    val data = record?.manufacturerSpecificData ?: return emptyList()
    return (0 until data.size()).map { index -> data.keyAt(index) to (data.valueAt(index) ?: ByteArray(0)) }
  }

  private companion object {
    const val SCAN_RESULT_INTERVAL_MS = 1_000L
  }
}
