package dev.gattify.plugin

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ExecuteResultTest {
  private fun resolveJson(kind: String?, adapterState: () -> String = { "test-state" }): JSONObject {
    val result = executeResult(kind, adapterState)
    assertTrue(result is ExecuteResult.Resolve)
    return JSONObject((result as ExecuteResult.Resolve).reply.toString())
  }

  private fun reject(kind: String?): ExecuteResult.Reject {
    val result = executeResult(kind) { "test-state" }
    assertTrue(result is ExecuteResult.Reject)
    return result as ExecuteResult.Reject
  }

  @Test
  fun `getState resolves with the adapter state as payload`() {
    val reply = resolveJson("getState", adapterState = { "poweredOn" })
    assertEquals("state", reply.getString("kind"))
    assertEquals("poweredOn", reply.getString("payload"))
  }

  @Test
  fun `getCapabilities resolves with every capability unknown and background unsupported`() {
    val reply = resolveJson("getCapabilities")
    assertEquals("capabilities", reply.getString("kind"))
    val payload = reply.getJSONObject("payload")
    for (key in listOf("central", "peripheral", "advertising", "targetedNotify", "simultaneousRoles")) {
      assertEquals(key, "unknown", payload.getJSONObject(key).getString("level"))
    }
    assertEquals("unsupported", payload.getJSONObject("background").getString("level"))
  }

  @Test
  fun `checkPermissions resolves with every permission unknown`() {
    val reply = resolveJson("checkPermissions")
    assertEquals("permissions", reply.getString("kind"))
    val payload = reply.getJSONObject("payload")
    assertEquals("unknown", payload.getString("scan"))
    assertEquals("unknown", payload.getString("connect"))
    assertEquals("unknown", payload.getString("advertise"))
  }

  @Test
  fun `cancel and closeOwner resolve empty with no payload`() {
    for (kind in listOf("cancel", "closeOwner")) {
      val reply = resolveJson(kind)
      assertEquals(kind, "empty", reply.getString("kind"))
      assertFalse(kind, reply.has("payload"))
    }
  }

  @Test
  fun `radio commands reject as unsupported`() {
    for (kind in listOf("startScan", "connect", "createServer", "startAdvertising", "notify")) {
      val result = reject(kind)
      assertEquals(kind, "unsupported", result.code)
      assertEquals(kind, "the Android backend does not implement $kind yet", result.message)
    }
  }

  @Test
  fun `a null or empty kind rejects as unsupported naming this command`() {
    for (kind in listOf(null, "")) {
      val result = reject(kind)
      assertEquals("unsupported", result.code)
      assertEquals("the Android backend does not implement this command yet", result.message)
    }
  }
}
