package dev.gattify.plugin

/** The filter of the platform scan: every device, or the devices that advertise one of [uuids]. */
internal data class ScanFilterSet(val everyDevice: Boolean, val uuids: Set<String>) {
  /** Whether a platform scan with this filter sees every device that [other] asks for. */
  fun covers(other: ScanFilterSet): Boolean = everyDevice || (!other.everyDevice && uuids.containsAll(other.uuids))

  companion object {
    /** The union of the logical scan filters, or null when no scan runs. An empty filter asks for every device. */
    fun union(filters: Collection<Set<String>>): ScanFilterSet? = when {
      filters.isEmpty() -> null
      filters.any { it.isEmpty() } -> ScanFilterSet(everyDevice = true, uuids = emptySet())
      else -> ScanFilterSet(everyDevice = false, uuids = filters.flatMapTo(HashSet()) { it })
    }
  }
}

/**
 * Counts platform scan starts. Android lets an app start about five scans in 30 s,
 * and past that a scan silently reports nothing.
 */
internal class StartQuota(private val maxStarts: Int, private val windowMs: Long) {
  private val starts = ArrayDeque<Long>()

  /** The earliest time at which one more start stays within the quota. */
  fun nextAllowed(now: Long): Long {
    while (starts.isNotEmpty() && starts.first() + windowMs <= now) starts.removeFirst()
    return if (starts.size < maxStarts) now else starts.first() + windowMs
  }

  fun record(now: Long) {
    starts.addLast(now)
  }
}

internal sealed interface ScanStep {
  object Keep : ScanStep

  object Stop : ScanStep

  class Start(val filter: ScanFilterSet) : ScanStep

  class Wait(val until: Long) : ScanStep
}

/**
 * What the platform scan does after the logical scans change. A running scan that
 * covers every logical scan keeps running, even with a wider filter, because each
 * restart spends a start. Results are filtered again per logical scan.
 */
internal fun planScan(desired: ScanFilterSet?, running: ScanFilterSet?, allowedAt: Long, now: Long): ScanStep = when {
  desired == null -> if (running == null) ScanStep.Keep else ScanStep.Stop
  running != null && running.covers(desired) -> ScanStep.Keep
  allowedAt <= now -> ScanStep.Start(desired)
  else -> ScanStep.Wait(allowedAt)
}
