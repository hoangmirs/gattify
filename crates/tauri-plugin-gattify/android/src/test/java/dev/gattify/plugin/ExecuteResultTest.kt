package dev.gattify.plugin

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ExecuteResultTest {
  private class FakeProbe(
    override val hasAdapter: Boolean = true,
    override val adapterOn: Boolean = true,
    override val connectPermitted: Boolean = true,
    override val hasAdvertiser: Boolean = true,
    private val outcomes: PermissionOutcomes = PermissionOutcomes("granted", "promptable", "deniedPermanently"),
  ) : StatusProbe {
    override fun permissions() = outcomes
  }

  private fun resolveJson(kind: String, probe: StatusProbe = FakeProbe()): JSONObject {
    val result = executeResult(kind, probe)
    assertTrue(result is ExecuteResult.Resolve)
    return JSONObject((result as ExecuteResult.Resolve).reply.toString())
  }

  private fun capabilities(probe: FakeProbe): JSONObject {
    val reply = resolveJson("getCapabilities", probe)
    assertEquals("capabilities", reply.getString("kind"))
    return reply.getJSONObject("payload")
  }

  private fun assertSupport(payload: JSONObject, key: String, level: String, reason: String) {
    val support = payload.getJSONObject(key)
    assertEquals(key, level, support.getString("level"))
    assertEquals(key, reason, support.getString("reason"))
    assertTrue(key, support.has("description") && support.isNull("description"))
  }

  @Test
  fun `getState maps the adapter and the connect permission`() {
    val cases = listOf(
      FakeProbe(hasAdapter = false) to "unavailable",
      FakeProbe(connectPermitted = false) to "unauthorized",
      FakeProbe(adapterOn = true) to "poweredOn",
      FakeProbe(adapterOn = false) to "poweredOff",
    )
    for ((probe, state) in cases) {
      val reply = resolveJson("getState", probe)
      assertEquals("state", reply.getString("kind"))
      assertEquals(state, reply.getString("payload"))
    }
  }

  @Test
  fun `getCapabilities with an advertiser supports every role but background`() {
    val payload = capabilities(FakeProbe())
    for (key in listOf("central", "peripheral", "advertising", "targetedNotify", "simultaneousRoles")) {
      assertSupport(payload, key, "supported", "available")
    }
    assertSupport(payload, "background", "unsupported", "foregroundOnlyContract")
    assertTrue(payload.isNull("maxConnections"))
    assertEquals(31, payload.getInt("maxAdvertisingDataLength"))
  }

  @Test
  fun `getCapabilities without an advertiser keeps only the central role`() {
    val payload = capabilities(FakeProbe(hasAdvertiser = false))
    assertSupport(payload, "central", "supported", "available")
    for (key in listOf("peripheral", "advertising", "targetedNotify", "simultaneousRoles")) {
      assertSupport(payload, key, "unsupported", "noAdvertiser")
    }
  }

  @Test
  fun `getCapabilities while the adapter is off cannot tell whether it advertises`() {
    val payload = capabilities(FakeProbe(adapterOn = false, hasAdvertiser = false))
    assertSupport(payload, "central", "supported", "available")
    for (key in listOf("peripheral", "advertising", "targetedNotify", "simultaneousRoles")) {
      assertSupport(payload, key, "unknown", "bluetoothOff")
    }
  }

  @Test
  fun `getCapabilities without an adapter reports noAdapter`() {
    val payload = capabilities(FakeProbe(hasAdapter = false, adapterOn = false, hasAdvertiser = false))
    for (key in listOf("central", "peripheral", "advertising", "targetedNotify", "simultaneousRoles")) {
      assertSupport(payload, key, "unsupported", "noAdapter")
    }
    assertSupport(payload, "background", "unsupported", "foregroundOnlyContract")
    assertTrue(payload.isNull("maxAdvertisingDataLength"))
  }

  @Test
  fun `checkPermissions resolves with the outcome of each role`() {
    val reply = resolveJson("checkPermissions")
    assertEquals("permissions", reply.getString("kind"))
    val payload = reply.getJSONObject("payload")
    assertEquals("granted", payload.getString("scan"))
    assertEquals("promptable", payload.getString("connect"))
    assertEquals("deniedPermanently", payload.getString("advertise"))
  }

  @Test
  fun `every other command of the contract goes to the backend`() {
    for (kind in COMMAND_KINDS - setOf("getState", "getCapabilities", "checkPermissions")) {
      assertNull(kind, executeResult(kind, FakeProbe()))
    }
    assertEquals(22, COMMAND_KINDS.size)
  }

  @Test
  fun `an unknown kind rejects as unsupported`() {
    val result = executeResult("teleport", FakeProbe())
    assertTrue(result is ExecuteResult.Reject)
    result as ExecuteResult.Reject
    assertEquals("unsupported", result.code)
    assertEquals("the Android backend does not know teleport", result.message)
  }

  @Test
  fun `a null or empty kind rejects as unsupported naming this command`() {
    for (kind in listOf(null, "")) {
      val result = executeResult(kind, FakeProbe())
      assertTrue(result is ExecuteResult.Reject)
      result as ExecuteResult.Reject
      assertEquals("unsupported", result.code)
      assertEquals("the Android backend does not know this command", result.message)
    }
  }

  @Test
  fun `a status reply is valid JSON with no extra keys`() {
    val reply = resolveJson("getState")
    assertEquals(setOf("kind", "payload"), reply.keySet())
    assertFalse(reply.getString("payload").isEmpty())
  }
}
