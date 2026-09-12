package dev.gattify.plugin

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class UuidsTest {
  private val lab = "80ff87c3-8e84-4914-aedc-0d6a3ba5534d"

  private fun parsed(value: String) = parseUuid(value)?.let(::uuidString)

  @Test
  fun `parseUuid expands short UUIDs with the Bluetooth base UUID`() {
    assertEquals("0000180d-0000-1000-8000-00805f9b34fb", parsed("180D"))
    assertEquals("0000180d-0000-1000-8000-00805f9b34fb", parsed("0000180d"))
    assertEquals("12345678-0000-1000-8000-00805f9b34fb", parsed("12345678"))
  }

  @Test
  fun `parseUuid accepts 128-bit UUIDs in any case, with or without hyphens`() {
    assertEquals(lab, parsed(lab))
    assertEquals(lab, parsed("80FF87C38E844914AEDC0D6A3BA5534D"))
    assertEquals("ffffffff-ffff-ffff-ffff-ffffffffffff", parsed("FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF"))
  }

  @Test
  fun `parseUuid rejects anything else`() {
    for (value in listOf("", "xyz", "180", "18 0D", "0x180d", "80ff87c3-8e84-4914-aedc-0d6a3ba5534", "$lab-00")) {
      assertNull(value, parseUuid(value))
    }
  }

  @Test
  fun `the CCCD is 0x2902`() {
    assertEquals("00002902-0000-1000-8000-00805f9b34fb", uuidString(CCCD_UUID))
  }

  @Test
  fun `an empty filter matches every device, and a filter matches UUIDs or service data`() {
    assertTrue(matchesFilter(emptySet(), emptyList(), emptyList()))
    assertTrue(matchesFilter(setOf(lab), listOf(lab), emptyList()))
    assertTrue(matchesFilter(setOf(lab), emptyList(), listOf(lab)))
    assertFalse(matchesFilter(setOf(lab), listOf("0000180d-0000-1000-8000-00805f9b34fb"), emptyList()))
  }

  @Test
  fun `cutUtf8 keeps a name that fits`() {
    val cut = cutUtf8("Host A", 13)
    assertArrayEquals("Host A".toByteArray(), cut.bytes)
    assertFalse(cut.truncated)
    val exact = cutUtf8("abcdefghijklm", 13)
    assertEquals(13, exact.bytes.size)
    assertFalse(exact.truncated)
    assertEquals(0, cutUtf8("", 13).bytes.size)
  }

  @Test
  fun `cutUtf8 cuts at a character boundary`() {
    val ascii = cutUtf8("abcdefghijklmnop", 13)
    assertEquals("abcdefghijklm", String(ascii.bytes))
    assertTrue(ascii.truncated)

    // "é" is 2 bytes: 12 ASCII bytes leave 1 byte, which cannot hold it.
    val accent = cutUtf8("abcdefghijklé", 13)
    assertEquals("abcdefghijkl", String(accent.bytes))
    assertTrue(accent.truncated)

    // An emoji is 4 bytes: 3 of them fit, which must not split it.
    val emoji = cutUtf8("Hostname😀😀", 13)
    assertEquals("Hostname😀", String(emoji.bytes))
    assertTrue(emoji.truncated)

    val cjk = cutUtf8("東京タワー", 13)
    assertEquals("東京タワ", String(cjk.bytes))
    assertEquals(12, cjk.bytes.size)
  }

  @Test
  fun `decodeUtf8 rejects invalid UTF-8`() {
    assertEquals("Host", decodeUtf8("Host".toByteArray()))
    assertNull(decodeUtf8(byteArrayOf(0xC3.toByte())))
    assertNull(decodeUtf8(byteArrayOf(0xFF.toByte(), 0x41)))
  }

  @Test
  fun `scanName prefers the local name, then filter service data, then the cached name`() {
    val filter = setOf(lab)
    val named = listOf(lab to "Host B".toByteArray())
    assertEquals("Host A", scanName("Host A", named, filter, "Pixel"))
    assertEquals("Host B", scanName(null, named, filter, "Pixel"))
    assertEquals("Host B", scanName("", named, filter, "Pixel"))
    assertEquals("Pixel", scanName(null, listOf(lab to byteArrayOf(0xFF.toByte())), filter, "Pixel"))
    assertEquals("Pixel", scanName(null, listOf(lab to ByteArray(0)), filter, "Pixel"))
    assertEquals("Pixel", scanName(null, named, emptySet(), "Pixel"))
    assertNull(scanName(null, emptyList(), filter, null))
    assertNull(scanName(null, emptyList(), filter, ""))
  }
}
