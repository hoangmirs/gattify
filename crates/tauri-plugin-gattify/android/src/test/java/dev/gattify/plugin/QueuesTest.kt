package dev.gattify.plugin

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class QueuesTest {
  private class Op(val name: String, var settled: Boolean = false)

  /** A characteristic stand-in with identity equality, like LocalCharacteristic. */
  private class Key(val name: String)

  @Test
  fun `one operation is in flight at a time, in order`() {
    val queue = OpQueue<Op>()
    val first = Op("first")
    val second = Op("second")
    queue.enqueue(first)
    queue.enqueue(second)
    assertSame(first, queue.next())
    assertNull(queue.next())
    assertFalse(queue.complete(second))
    assertTrue(queue.complete(first))
    assertSame(second, queue.next())
    assertEquals(1, queue.size)
  }

  @Test
  fun `next drops settled operations`() {
    val queue = OpQueue<Op>()
    queue.enqueue(Op("cancelled", settled = true))
    val live = Op("live")
    queue.enqueue(live)
    assertSame(live, queue.next { it.settled })
    assertEquals(1, queue.size)
  }

  @Test
  fun `drain returns the operation in flight first, and removeWaiting keeps it`() {
    val queue = OpQueue<Op>()
    val names = listOf("a", "b", "c").map { Op(it) }
    names.forEach(queue::enqueue)
    queue.next()
    assertEquals(listOf("b"), queue.removeWaiting { it.name == "b" }.map { it.name })
    assertEquals("a", queue.inFlight?.name)
    assertEquals(listOf("a", "c"), queue.drain().map { it.name })
    assertNull(queue.inFlight)
    assertEquals(0, queue.size)
  }

  @Test
  fun `the throttle lets one result per device through every interval`() {
    val throttle = Throttle(1_000)
    assertTrue(throttle.allow("device-1", 0))
    assertFalse(throttle.allow("device-1", 999))
    assertTrue(throttle.allow("device-2", 999))
    assertTrue(throttle.allow("device-1", 1_000))
    assertFalse(throttle.allow("device-1", 1_500))
  }

  @Test
  fun `a long write emits one value per characteristic in the order of its first part`() {
    val writes = PreparedWrites<Key>()
    val rx = Key("rx")
    val other = Key("other")
    writes.append(rx, byteArrayOf(1, 2))
    assertEquals(2, writes.length(rx))
    writes.append(other, byteArrayOf(9))
    writes.append(rx, byteArrayOf(3))
    val (status, values) = writes.finish(execute = true)
    assertEquals(Att.SUCCESS, status)
    assertEquals(listOf("rx", "other"), values.map { it.first.name })
    assertArrayEquals(byteArrayOf(1, 2, 3), values[0].second)
    assertArrayEquals(byteArrayOf(9), values[1].second)
    assertEquals(0, writes.length(rx))
  }

  @Test
  fun `a failed part fails the execute and emits nothing`() {
    val writes = PreparedWrites<Key>()
    val rx = Key("rx")
    writes.append(rx, byteArrayOf(1))
    writes.fail(Att.INVALID_OFFSET)
    writes.fail(Att.INVALID_ATTRIBUTE_LENGTH)
    val (status, values) = writes.finish(execute = true)
    assertEquals(Att.INVALID_OFFSET, status)
    assertTrue(values.isEmpty())
    // The next long write starts clean.
    writes.append(rx, byteArrayOf(2))
    assertEquals(Att.SUCCESS, writes.finish(execute = true).first)
  }

  @Test
  fun `a cancelled execute discards the parts and succeeds`() {
    val writes = PreparedWrites<Key>()
    val rx = Key("rx")
    writes.append(rx, byteArrayOf(1))
    writes.fail(Att.INVALID_OFFSET)
    val (status, values) = writes.finish(execute = false)
    assertEquals(Att.SUCCESS, status)
    assertTrue(values.isEmpty())
  }

  @Test
  fun `discard drops the parts of a closed server`() {
    val writes = PreparedWrites<Key>()
    val gone = Key("gone")
    val kept = Key("kept")
    writes.append(gone, byteArrayOf(1))
    writes.append(kept, byteArrayOf(2))
    writes.discard { it === gone }
    assertEquals(listOf("kept"), writes.finish(execute = true).second.map { it.first.name })
  }
}
