package dev.gattify.plugin

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class IdsTest {
  @Test
  fun `each prefix counts from 1 and never repeats`() {
    val ids = IdAllocator()
    assertEquals("scan-1", ids.next("scan"))
    assertEquals("connection-1", ids.next("connection"))
    assertEquals("scan-2", ids.next("scan"))
  }

  @Test
  fun `IDs stay unique when backends on several threads share the allocator`() {
    val ids = IdAllocator()
    val devices = RemoteRegistry("device", ids)
    val allocated = java.util.Collections.synchronizedList(ArrayList<String>())
    val threads = (0 until 4).map { thread ->
      Thread {
        repeat(500) { n ->
          allocated += ids.next("scan")
          devices.idFor("AA:BB:CC:DD:${thread}:$n")
        }
      }
    }
    threads.forEach(Thread::start)
    threads.forEach(Thread::join)
    assertEquals(2_000, allocated.toSet().size)
    // 2,000 addresses took device-1 to device-2000 with no lost increment.
    assertEquals("device-2001", devices.idFor("AA:BB:CC:DD:EE:FF"))
  }

  @Test
  fun `the process registries share one allocator, so a new backend repeats no ID`() {
    val device = ProcessIds.devices.idFor("11:22:33:44:55:66")
    assertEquals(device, ProcessIds.devices.idFor("11:22:33:44:55:66"))
    assertTrue(ProcessIds.centrals.idFor("11:22:33:44:55:66").startsWith("central-"))
  }

  @Test
  fun `the owner family is the text after the first colon`() {
    assertEquals("main", ownerFamily("webview:main"))
    assertEquals("main", ownerFamily("gattify-peer:main"))
    assertEquals("a:b", ownerFamily("webview:a:b"))
    assertEquals("plain", ownerFamily("plain"))
  }

  @Test
  fun `a remote address keeps one opaque ID`() {
    val devices = RemoteRegistry("device", IdAllocator())
    val first = devices.idFor("AA:BB:CC:DD:EE:01")
    assertEquals("device-1", first)
    assertEquals(first, devices.idFor("AA:BB:CC:DD:EE:01"))
    assertEquals("device-2", devices.idFor("AA:BB:CC:DD:EE:02"))
    assertEquals("AA:BB:CC:DD:EE:01", devices.addressOf(first))
    assertNull(devices.addressOf("device-9"))
  }

  @Test
  fun `connect accepts a device that any owner of the family saw`() {
    val devices = RemoteRegistry("device", IdAllocator())
    val id = devices.idFor("AA:BB:CC:DD:EE:01")
    assertFalse(devices.seenByFamilyOf(id, "webview:main"))
    devices.markSeen(id, "webview:main")
    assertTrue(devices.seenByFamilyOf(id, "gattify-peer:main"))
    assertFalse(devices.seenByFamilyOf(id, "gattify-peer:other"))
  }

  @Test
  fun `a second discovery returns the same handle and binds the new attribute`() {
    val handles = HandleTable<String>("connection-2")
    assertEquals("connection-2/service-1", handles.service("svc#1"))
    assertEquals("connection-2/service-1", handles.service("svc#1"))
    assertEquals("connection-2/service-2", handles.service("svc#2"))

    val first = handles.characteristic("svc#1/chr#3", "first discovery")
    assertEquals("connection-2/characteristic-1", first)
    assertEquals(first, handles.characteristic("svc#1/chr#3", "second discovery"))
    assertSame("second discovery", handles[first])
    assertNotEquals(first, handles.characteristic("svc#1/chr#5", "other"))
    assertEquals(first, handles.idOf("svc#1/chr#3"))
    assertNull(handles["connection-3/characteristic-1"])
  }
}
