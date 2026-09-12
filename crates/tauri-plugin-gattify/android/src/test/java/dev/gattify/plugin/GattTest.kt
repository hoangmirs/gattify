package dev.gattify.plugin

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class GattTest {
  private val writable = Properties(read = false, write = true, writeWithoutResponse = false, notify = false, indicate = false)

  @Test
  fun `value length is the MTU minus 3, from 20 to 512`() {
    assertEquals(20, valueLength(23))
    assertEquals(182, valueLength(185))
    assertEquals(512, valueLength(517))
    assertEquals(20, valueLength(0))
  }

  @Test
  fun `properties map to the Bluetooth bits and back`() {
    val all = Properties(read = true, write = true, writeWithoutResponse = true, notify = true, indicate = true)
    assertEquals(0x3E, all.bits)
    assertEquals(all, Properties.fromBits(0x3E))
    assertEquals(Properties(read = true, write = false, writeWithoutResponse = false, notify = true, indicate = false), Properties.fromBits(0x12))
    // Broadcast (0x01) and extended properties (0x80) have no contract field.
    assertEquals(Properties.fromBits(0), Properties.fromBits(0x81))
  }

  @Test
  fun `properties give the attribute permissions of a local characteristic`() {
    assertEquals(PERMISSION_READ, Properties.fromBits(0x12).permissions)
    assertEquals(PERMISSION_WRITE, Properties.fromBits(0x04).permissions)
    assertEquals(PERMISSION_READ or PERMISSION_WRITE, Properties.fromBits(0x0A).permissions)
    assertTrue(Properties.fromBits(0x20).notifies)
    assertFalse(Properties.fromBits(0x02).writable)
  }

  private val notifyOnly = Properties(read = false, write = false, writeWithoutResponse = false, notify = true, indicate = false)
  private val indicateOnly = Properties(read = false, write = false, writeWithoutResponse = false, notify = false, indicate = true)
  private val both = Properties(read = false, write = false, writeWithoutResponse = false, notify = true, indicate = true)

  @Test
  fun `a CCCD value is the full unsigned 16-bit little-endian value`() {
    assertEquals(CCCD_NOTIFY, cccdValueOf(byteArrayOf(1, 0)))
    assertEquals(CCCD_INDICATE, cccdValueOf(byteArrayOf(2, 0)))
    assertEquals(0, cccdValueOf(byteArrayOf(0, 0)))
    assertEquals(0x0101, cccdValueOf(byteArrayOf(1, 1)))
    assertEquals(0xFFFF, cccdValueOf(byteArrayOf(0xFF.toByte(), 0xFF.toByte())))
    assertNull(cccdValueOf(byteArrayOf(1)))
    assertNull(cccdValueOf(byteArrayOf(1, 0, 0)))
    assertArrayEquals(byteArrayOf(2, 0), cccdValue(CCCD_INDICATE))
  }

  @Test
  fun `a CCCD write may enable only the modes the characteristic has`() {
    assertEquals(Att.SUCCESS, cccdWriteStatus(notifyOnly, byteArrayOf(1, 0)))
    assertEquals(Att.SUCCESS, cccdWriteStatus(notifyOnly, byteArrayOf(0, 0)))
    assertEquals(Att.SUCCESS, cccdWriteStatus(indicateOnly, byteArrayOf(2, 0)))
    assertEquals(Att.SUCCESS, cccdWriteStatus(both, byteArrayOf(3, 0)))
    // Indications on a notify-only characteristic must not switch the host to indications.
    assertEquals(Att.CCCD_IMPROPERLY_CONFIGURED, cccdWriteStatus(notifyOnly, byteArrayOf(2, 0)))
    assertEquals(Att.CCCD_IMPROPERLY_CONFIGURED, cccdWriteStatus(notifyOnly, byteArrayOf(3, 0)))
    assertEquals(Att.CCCD_IMPROPERLY_CONFIGURED, cccdWriteStatus(indicateOnly, byteArrayOf(1, 0)))
  }

  @Test
  fun `a CCCD write with a reserved bit or a bad length fails`() {
    assertEquals(Att.CCCD_IMPROPERLY_CONFIGURED, cccdWriteStatus(both, byteArrayOf(4, 0)))
    // A reserved bit in the high byte, which a low-byte mask used to hide.
    assertEquals(Att.CCCD_IMPROPERLY_CONFIGURED, cccdWriteStatus(notifyOnly, byteArrayOf(1, 1)))
    assertEquals(Att.CCCD_IMPROPERLY_CONFIGURED, cccdWriteStatus(both, byteArrayOf(0, 0x80.toByte())))
    assertEquals(Att.INVALID_ATTRIBUTE_LENGTH, cccdWriteStatus(both, byteArrayOf(1)))
    assertEquals(Att.INVALID_ATTRIBUTE_LENGTH, cccdWriteStatus(both, byteArrayOf(1, 0, 0)))
  }

  @Test
  fun `only indications alone confirm`() {
    assertFalse(confirms(CCCD_NOTIFY))
    assertTrue(confirms(CCCD_INDICATE))
    assertFalse(confirms(CCCD_NOTIFY or CCCD_INDICATE))
  }

  @Test
  fun `a read answers from the offset on`() {
    val value = byteArrayOf(1, 2, 3)
    assertEquals(Att.SUCCESS, readAnswer(true, value, 0).first)
    assertArrayEquals(byteArrayOf(2, 3), readAnswer(true, value, 1).second)
    assertArrayEquals(ByteArray(0), readAnswer(true, value, 3).second)
    assertEquals(Att.INVALID_OFFSET, readAnswer(true, value, 4).first)
    assertEquals(Att.READ_NOT_PERMITTED, readAnswer(false, value, 0).first)
  }

  @Test
  fun `a write part must be allowed, contiguous from 0, and fit`() {
    assertEquals(Att.SUCCESS, writeStatus(writable, 10, assembled = 0, offset = 0, size = 10))
    assertEquals(Att.SUCCESS, writeStatus(writable, 10, assembled = 4, offset = 4, size = 6))
    assertEquals(Att.INVALID_OFFSET, writeStatus(writable, 10, assembled = 0, offset = 2, size = 1))
    assertEquals(Att.INVALID_OFFSET, writeStatus(writable, 10, assembled = 4, offset = 3, size = 1))
    assertEquals(Att.INVALID_ATTRIBUTE_LENGTH, writeStatus(writable, 10, assembled = 4, offset = 4, size = 7))
    assertEquals(Att.WRITE_NOT_PERMITTED, writeStatus(Properties.fromBits(0x02), 10, 0, 0, 1))
    val commandOnly = Properties(read = false, write = false, writeWithoutResponse = true, notify = false, indicate = false)
    assertEquals(Att.SUCCESS, writeStatus(commandOnly, 10, 0, 0, 1))
  }

  @Test
  fun `platform failures map to contract codes`() {
    assertEquals("gattStatus133", gattStatusError(133, "connect").code)
    assertEquals("payloadTooLarge", advertiseFailure(1).code)
    assertEquals("busy", advertiseFailure(2).code)
    assertEquals("busy", advertiseFailure(3).code)
    assertEquals("internal", advertiseFailure(4).code)
    assertEquals("unsupported", advertiseFailure(5).code)
    assertEquals("bluetoothOff", statusCodeError(1, "write").code)
    assertEquals("disconnected", statusCodeError(4, "write").code)
    assertEquals("permissionDenied", statusCodeError(6, "write").code)
    assertEquals("busy", statusCodeError(201, "write").code)
    assertEquals("internal", statusCodeError(200, "write").code)
  }
}
