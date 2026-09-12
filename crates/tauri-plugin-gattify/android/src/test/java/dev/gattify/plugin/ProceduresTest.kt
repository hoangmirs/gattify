package dev.gattify.plugin

import app.tauri.plugin.JSObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** A scheduler with a manual clock. */
internal class FakeScheduler : Scheduler {
  private class Task(val at: Long, val action: () -> Unit)

  private val tasks = ArrayList<Task>()
  private var time = 0L

  override fun schedule(delayMs: Long, action: () -> Unit): Cancellable {
    val task = Task(time + delayMs, action)
    tasks += task
    return Cancellable { tasks.remove(task) }
  }

  override fun now(): Long = time

  fun advance(ms: Long) {
    val until = time + ms
    while (true) {
      val task = tasks.filter { it.at <= until }.minByOrNull { it.at } ?: break
      tasks.remove(task)
      time = task.at
      task.action()
    }
    time = until
  }

  val pending: Int
    get() = tasks.size
}

internal class RecordingResponder : Responder {
  val answers = ArrayList<String>()

  override fun resolve(reply: JSObject) {
    answers += "resolve:${reply.getString("kind")}"
  }

  override fun reject(message: String, code: String) {
    answers += "reject:$code"
  }
}

class ProceduresTest {
  private val scheduler = FakeScheduler()
  private val procedures = Procedures(scheduler)

  @Test
  fun `a procedure settles once`() {
    val responder = RecordingResponder()
    val procedure = procedures.start("op-1", "webview:main", responder)
    procedure.resolve(Replies.empty())
    procedure.reject(internalError("late"))
    procedure.abort(cancelled("late"))
    assertEquals(listOf("resolve:empty"), responder.answers)
    assertEquals(0, procedures.size)
  }

  @Test
  fun `the deadline rejects with timeout and runs the cleanups in order`() {
    val responder = RecordingResponder()
    val procedure = procedures.start("op-1", "webview:main", responder)
    val cleaned = ArrayList<String>()
    procedure.onAbort { cleaned += "first:${it.code}" }
    procedure.onAbort { cleaned += "second:${it.code}" }
    procedure.armDeadline(5_000)
    scheduler.advance(4_999)
    assertEquals(emptyList<String>(), responder.answers)
    scheduler.advance(1)
    assertEquals(listOf("reject:timeout"), responder.answers)
    assertEquals(listOf("first:timeout", "second:timeout"), cleaned)
    assertTrue(procedure.settled)
  }

  @Test
  fun `settling cancels the deadline, and a null deadline arms nothing`() {
    val procedure = procedures.start("op-1", "webview:main", RecordingResponder())
    procedure.armDeadline(5_000)
    assertEquals(1, scheduler.pending)
    procedure.resolve(Replies.empty())
    assertEquals(0, scheduler.pending)
    procedures.start("op-2", "webview:main", RecordingResponder()).armDeadline(null)
    assertEquals(0, scheduler.pending)
  }

  @Test
  fun `a plain rejection skips the cleanups`() {
    val procedure = procedures.start("op-1", "webview:main", RecordingResponder())
    var cleaned = false
    procedure.onAbort { cleaned = true }
    procedure.reject(gattStatusError(133, "connect"))
    assertEquals(false, cleaned)
  }

  @Test
  fun `cancel aborts the procedure with that operation ID only`() {
    val first = RecordingResponder()
    val second = RecordingResponder()
    procedures.start("op-1", "webview:main", first)
    procedures.start("op-2", "webview:main", second)
    procedures.abort("op-1", cancelled("cancel"))
    procedures.abort("op-unknown", cancelled("cancel"))
    assertEquals(listOf("reject:cancelled"), first.answers)
    assertEquals(emptyList<String>(), second.answers)
  }

  @Test
  fun `closeOwner aborts the procedures of that owner but itself`() {
    val mine = RecordingResponder()
    val theirs = RecordingResponder()
    val closing = RecordingResponder()
    procedures.start("op-1", "webview:main", mine)
    procedures.start("op-2", "gattify-peer:main", theirs)
    val closeOwner = procedures.start("op-3", "webview:main", closing)
    procedures.abortOwner("webview:main", cancelled("closeOwner"), except = closeOwner)
    assertEquals(listOf("reject:cancelled"), mine.answers)
    assertEquals(emptyList<String>(), theirs.answers)
    assertEquals(emptyList<String>(), closing.answers)
  }

  @Test
  fun `deadlines follow the contract defaults`() {
    assertEquals(15_000L, deadlineFor("connect", null))
    assertEquals(8_000L, deadlineFor("connect", null, connectTimeoutMs = 8_000))
    assertEquals(3_000L, deadlineFor("connect", 3_000, connectTimeoutMs = 8_000))
    assertEquals(10_000L, deadlineFor("discoverServices", null))
    for (kind in listOf("read", "write", "subscribe", "unsubscribe", "notify", "createServer", "startAdvertising")) {
      assertEquals(kind, 5_000L, deadlineFor(kind, null))
    }
    assertNull(deadlineFor("requestPermissions", null))
    assertEquals(1_000L, deadlineFor("requestPermissions", 1_000))
  }
}
