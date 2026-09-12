package dev.gattify.plugin

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PermissionsTest {
  @Test
  fun `API 31 splits the permission by role`() {
    assertEquals("bluetoothScan", permissionAlias(Role.SCAN, 31))
    assertEquals("bluetoothConnect", permissionAlias(Role.CONNECT, 34))
    assertEquals("bluetoothAdvertise", permissionAlias(Role.ADVERTISE, 36))
    assertEquals("android.permission.BLUETOOTH_SCAN", androidPermission(Role.SCAN, 31))
    assertEquals("android.permission.BLUETOOTH_CONNECT", androidPermission(Role.CONNECT, 31))
    assertEquals("android.permission.BLUETOOTH_ADVERTISE", androidPermission(Role.ADVERTISE, 31))
  }

  @Test
  fun `API 30 scans with fine location and needs nothing to connect or advertise`() {
    assertEquals("location", permissionAlias(Role.SCAN, 30))
    assertEquals("android.permission.ACCESS_FINE_LOCATION", androidPermission(Role.SCAN, 26))
    for (role in listOf(Role.CONNECT, Role.ADVERTISE)) {
      assertNull(permissionAlias(role, 30))
      assertNull(androidPermission(role, 26))
    }
  }

  @Test
  fun `Tauri permission states map to outcomes`() {
    assertEquals("granted", permissionOutcome("granted"))
    assertEquals("promptable", permissionOutcome("prompt"))
    assertEquals("promptable", permissionOutcome("prompt-with-rationale"))
    assertEquals("deniedPermanently", permissionOutcome("denied"))
    assertEquals("unknown", permissionOutcome(null))
  }

  @Test
  fun `outcomes report notRequired for roles without a runtime permission`() {
    val states = mapOf("location" to "denied", "bluetoothScan" to "granted", "bluetoothConnect" to "prompt")
    val old = permissionOutcomes(30) { states[it] }
    assertEquals(listOf("deniedPermanently", "notRequired", "notRequired"), listOf(old.scan, old.connect, old.advertise))
    val new = permissionOutcomes(33) { states[it] }
    assertEquals(listOf("granted", "promptable", "unknown"), listOf(new.scan, new.connect, new.advertise))
  }

  @Test
  fun `a request asks only for the requested roles that are not granted`() {
    val states = mapOf("bluetoothScan" to "granted", "bluetoothConnect" to "prompt", "bluetoothAdvertise" to "denied")
    assertEquals(
      listOf("bluetoothConnect", "bluetoothAdvertise"),
      aliasesToRequest(PermissionAsk(scan = true, connect = true, advertise = true), 31) { states[it] },
    )
    assertEquals(
      emptyList<String>(),
      aliasesToRequest(PermissionAsk(scan = true, connect = false, advertise = false), 31) { states[it] },
    )
    assertEquals(
      emptyList<String>(),
      aliasesToRequest(PermissionAsk(scan = false, connect = true, advertise = true), 30) { states[it] },
    )
    assertEquals(
      listOf("location"),
      aliasesToRequest(PermissionAsk(scan = true, connect = true, advertise = true), 30) { null },
    )
  }

  @Test
  fun `each command needs the permission of the contract table`() {
    val table = mapOf(
      "startScan" to Role.SCAN,
      "connect" to Role.CONNECT,
      "disconnect" to Role.CONNECT,
      "discoverServices" to Role.CONNECT,
      "read" to Role.CONNECT,
      "write" to Role.CONNECT,
      "subscribe" to Role.CONNECT,
      "unsubscribe" to Role.CONNECT,
      "createServer" to Role.CONNECT,
      "closeServer" to Role.CONNECT,
      "setValue" to Role.CONNECT,
      "notify" to Role.CONNECT,
      "startAdvertising" to Role.ADVERTISE,
      "stopAdvertising" to Role.ADVERTISE,
    )
    for (kind in COMMAND_KINDS) assertEquals(kind, table[kind], requirementOf(kind)?.role)
  }

  @Test
  fun `cleanup commands run while the adapter is off`() {
    for (kind in listOf("disconnect", "unsubscribe", "closeServer", "setValue", "stopAdvertising")) {
      assertEquals(kind, false, requirementOf(kind)?.adapterOn)
    }
    for (kind in listOf("startScan", "connect", "discoverServices", "read", "write", "subscribe", "createServer", "notify", "startAdvertising")) {
      assertEquals(kind, true, requirementOf(kind)?.adapterOn)
    }
  }
}
