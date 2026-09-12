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
import java.util.UUID

/**
 * The scans. Every logical scan shares one platform scan with the union of their
 * filters, and each result goes to every logical scan whose filter matches. Filter
 * changes restart the platform scan at most a few times per 30 s.
 */
// The backend checks the scan permission before startScan. Lint cannot see the
// SecurityException catch of quietly() through a lambda, so the functions that
// use it suppress MissingPermission.
internal class Scanner(private val backend: GattifyBackend) {
  private val scans = LinkedHashMap<String, LogicalScan>()
  private var platform: PlatformScan? = null
  private val quota = StartQuota(MAX_STARTS, QUOTA_WINDOW_MS)
  private var reconcileTimer: Cancellable? = null

  private class LogicalScan(val id: String, val owner: String, val filter: Set<String>) {
    val throttle = Throttle(SCAN_RESULT_INTERVAL_MS)
    var timer: Cancellable? = null
  }

  /** The platform scan. The platform calls back on the main thread, so each callback moves to the handler. */
  private inner class PlatformScan(val filter: ScanFilterSet, val scanner: BluetoothLeScanner) : ScanCallback() {
    override fun onScanResult(callbackType: Int, result: ScanResult) {
      backend.post { if (platform === this) deliver(result) }
    }

    override fun onBatchScanResults(results: MutableList<ScanResult>) {
      val copy = results.toList()
      backend.post { if (platform === this) copy.forEach(::deliver) }
    }

    override fun onScanFailed(errorCode: Int) {
      backend.post {
        if (platform !== this) return@post
        Log.w(TAG, "the platform scan failed with error $errorCode")
        platform = null
        endAll()
      }
    }
  }

  fun start(request: Request, procedure: Procedure) {
    val spec = decodeScan(request.payload())
    if (backend.adapter?.bluetoothLeScanner == null) throw BleException(ErrorCode.BLUETOOTH_OFF, "Bluetooth is off")
    val scan = LogicalScan(backend.ids.next("scan"), request.ownerId, spec.serviceUuids.mapTo(HashSet(), ::uuidString))
    scans[scan.id] = scan
    spec.timeoutMs?.let { timeout -> scan.timer = backend.scheduler.schedule(timeout) { end(scan, emit = true) } }
    procedure.resolve(Replies.scanStarted(scan.id))
    reconcileSoon()
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

  /** The backend is going away: every scan ends without events, and the platform scan stops now. */
  fun releaseAll() {
    closeOwner(null)
    reconcileTimer?.cancel()
    reconcileTimer = null
    stopPlatform()
  }

  /** Bluetooth turned off: the platform scan is gone, and every scan ends. */
  fun adapterOff() {
    stopPlatform()
    endAll()
  }

  fun owners(): Set<String> = scans.values.mapTo(LinkedHashSet()) { it.owner }

  fun count(ownerId: String) = scans.values.count { it.owner == ownerId }

  private fun end(scan: LogicalScan, emit: Boolean) {
    if (scans[scan.id] !== scan) return
    scans.remove(scan.id)
    scan.timer?.cancel()
    if (emit) backend.emit(scan.owner, Events.scanStopped(scan.id))
    reconcileSoon()
  }

  private fun endAll() {
    for (scan in scans.values.toList()) end(scan, emit = true)
  }

  /** Coalesces the scan changes of several commands into one platform change. */
  private fun reconcileSoon(delayMs: Long = RECONCILE_DELAY_MS) {
    reconcileTimer?.cancel()
    reconcileTimer = backend.scheduler.schedule(delayMs) {
      reconcileTimer = null
      reconcile()
    }
  }

  private fun reconcile() {
    val now = backend.scheduler.now()
    val desired = ScanFilterSet.union(scans.values.map { it.filter })
    when (val step = planScan(desired, platform?.filter, quota.nextAllowed(now), now)) {
      ScanStep.Keep -> Unit
      ScanStep.Stop -> stopPlatform()
      is ScanStep.Start -> startPlatform(step.filter, now)
      // Past the quota a start would report nothing, so the change waits. The running scan, if any, keeps going.
      is ScanStep.Wait -> reconcileSoon(step.until - now)
    }
  }

  @SuppressLint("MissingPermission")
  private fun startPlatform(filter: ScanFilterSet, now: Long) {
    stopPlatform()
    val scanner = backend.adapter?.bluetoothLeScanner
    if (scanner == null) {
      endAll()
      return
    }
    val next = PlatformScan(filter, scanner)
    val filters = filter.uuids.map { ScanFilter.Builder().setServiceUuid(ParcelUuid(UUID.fromString(it))).build() }
    val settings = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build()
    try {
      scanner.startScan(filters, settings, next)
    } catch (e: SecurityException) {
      Log.w(TAG, "the platform scan lacks a permission", e)
      endAll()
      return
    } catch (e: IllegalStateException) {
      Log.w(TAG, "the platform scan could not start", e)
      endAll()
      return
    }
    quota.record(now)
    platform = next
  }

  @SuppressLint("MissingPermission")
  private fun stopPlatform() {
    val current = platform ?: return
    platform = null
    quietly { current.scanner.stopScan(current) }
  }

  private fun deliver(result: ScanResult) {
    if (scans.isEmpty()) return
    val record = result.scanRecord
    val serviceUuids = record?.serviceUuids.orEmpty().map { uuidString(it.uuid) }
    val serviceData = record?.serviceData.orEmpty().map { (uuid, bytes) -> uuidString(uuid.uuid) to (bytes ?: ByteArray(0)) }
    val serviceDataUuids = serviceData.map { it.first }
    val device = result.device
    val deviceId by lazy { backend.devices.idFor(device.address) }
    val now = backend.scheduler.now()
    val localName = record?.deviceName
    val cachedName by lazy { cachedName(device) }
    val manufacturerData by lazy { manufacturerData(record) }
    for (scan in scans.values.toList()) {
      if (!matchesFilter(scan.filter, serviceUuids, serviceDataUuids)) continue
      if (!scan.throttle.allow(deviceId, now)) continue
      backend.rememberDevice(deviceId, device)
      backend.devices.markSeen(deviceId, scan.owner)
      val report = DeviceReport(
        id = deviceId,
        name = scanName(localName, serviceData, scan.filter, cachedName),
        rssi = result.rssi,
        serviceUuids = serviceUuids,
        localName = localName,
        serviceData = serviceData,
        manufacturerData = manufacturerData,
        connectable = result.isConnectable,
        observedAtMillis = System.currentTimeMillis(),
        scanId = scan.id,
      )
      backend.emit(scan.owner, Events.scanResult(report))
    }
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

    /** Android's limit is five starts in 30 s. One start stays spare, and the window has a margin. */
    const val MAX_STARTS = 4
    const val QUOTA_WINDOW_MS = 31_000L

    /** Long enough to merge a stopScan and a startScan that follow each other into no change at all. */
    const val RECONCILE_DELAY_MS = 100L
  }
}
