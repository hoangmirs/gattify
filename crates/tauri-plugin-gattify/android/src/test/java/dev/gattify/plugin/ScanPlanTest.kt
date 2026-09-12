package dev.gattify.plugin

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class ScanPlanTest {
  private val a = "0000aaaa-0000-1000-8000-00805f9b34fb"
  private val b = "0000bbbb-0000-1000-8000-00805f9b34fb"

  private fun uuids(vararg values: String) = ScanFilterSet(everyDevice = false, uuids = values.toSet())

  private val everyDevice = ScanFilterSet(everyDevice = true, uuids = emptySet())

  @Test
  fun `the platform filter is the union of the logical filters`() {
    assertNull(ScanFilterSet.union(emptyList()))
    assertEquals(uuids(a, b), ScanFilterSet.union(listOf(setOf(a), setOf(b), setOf(a))))
    // One scan for every device makes the platform scan unfiltered.
    assertEquals(everyDevice, ScanFilterSet.union(listOf(setOf(a), emptySet())))
  }

  @Test
  fun `a wider filter covers a narrower one`() {
    assertTrue(uuids(a, b).covers(uuids(a)))
    assertFalse(uuids(a).covers(uuids(a, b)))
    assertTrue(everyDevice.covers(uuids(a)))
    assertTrue(everyDevice.covers(everyDevice))
    assertFalse(uuids(a, b).covers(everyDevice))
  }

  @Test
  fun `the quota allows four starts in the window, then waits for the oldest to expire`() {
    val quota = StartQuota(maxStarts = 4, windowMs = 31_000)
    for (time in listOf(0L, 1_000, 2_000, 3_000)) {
      assertEquals(time, quota.nextAllowed(time))
      quota.record(time)
    }
    assertEquals(31_000L, quota.nextAllowed(4_000))
    assertEquals(31_000L, quota.nextAllowed(31_000))
    quota.record(31_000)
    assertEquals(32_000L, quota.nextAllowed(31_500))
  }

  @Test
  fun `no logical scan stops the platform scan`() {
    assertSame(ScanStep.Stop, planScan(null, uuids(a), allowedAt = 0, now = 0))
    assertSame(ScanStep.Keep, planScan(null, null, allowedAt = 0, now = 0))
  }

  @Test
  fun `a covering platform scan keeps running, even when a filter narrowed`() {
    assertSame(ScanStep.Keep, planScan(uuids(a), uuids(a), allowedAt = 0, now = 0))
    // Narrowing would spend a start of the quota, so the wider scan stays and results are filtered per scan.
    assertSame(ScanStep.Keep, planScan(uuids(a), uuids(a, b), allowedAt = 0, now = 0))
    assertSame(ScanStep.Keep, planScan(uuids(b), everyDevice, allowedAt = 99_000, now = 0))
  }

  @Test
  fun `a filter the platform scan misses starts it again when the quota allows`() {
    val start = planScan(uuids(a, b), uuids(a), allowedAt = 5_000, now = 5_000)
    assertTrue(start is ScanStep.Start)
    assertEquals(uuids(a, b), (start as ScanStep.Start).filter)
    assertTrue(planScan(uuids(a), null, allowedAt = 0, now = 10) is ScanStep.Start)
  }

  @Test
  fun `past the quota the change waits instead of starting a scan that reports nothing`() {
    val wait = planScan(uuids(a, b), uuids(a), allowedAt = 31_000, now = 4_000)
    assertTrue(wait is ScanStep.Wait)
    assertEquals(31_000L, (wait as ScanStep.Wait).until)
  }
}
