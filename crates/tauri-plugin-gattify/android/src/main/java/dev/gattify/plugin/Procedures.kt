package dev.gattify.plugin

import app.tauri.plugin.JSObject

/** Receives the answer of one `execute` call. */
internal interface Responder {
  fun resolve(reply: JSObject)
  fun reject(message: String, code: String)
}

internal fun interface Cancellable {
  fun cancel()
}

/** Runs delayed work on the plugin thread, and reads a monotonic clock in milliseconds. */
internal interface Scheduler {
  fun schedule(delayMs: Long, action: () -> Unit): Cancellable
  fun now(): Long
}

/**
 * One pending `execute` call. It settles once. A deadline, a `cancel` or a
 * `closeOwner` aborts it: the procedure rejects, then runs the cleanup of its command.
 */
internal class Procedure internal constructor(
  val operationId: String,
  val ownerId: String,
  private val responder: Responder,
  private val registry: Procedures,
) {
  var settled = false
    private set

  private var cleanups = ArrayList<(BleException) -> Unit>()
  private var timer: Cancellable? = null

  /** Adds a cleanup that stops the command when the procedure is aborted. Cleanups run in order. */
  fun onAbort(cleanup: (BleException) -> Unit) {
    if (!settled) cleanups += cleanup
  }

  /** Aborts the procedure with `timeout` after [ms]. Null means no deadline. */
  fun armDeadline(ms: Long?) {
    timer?.cancel()
    timer = null
    if (ms == null || settled) return
    timer = registry.scheduler.schedule(ms) { abort(BleException(ErrorCode.TIMEOUT, "the deadline passed")) }
  }

  fun resolve(reply: JSObject) {
    if (settle()) responder.resolve(reply)
  }

  fun reject(error: BleException) {
    if (settle()) responder.reject(error.message, error.code)
  }

  fun abort(error: BleException) {
    if (settled) return
    val cleanups = cleanups
    reject(error)
    for (cleanup in cleanups) cleanup(error)
  }

  private fun settle(): Boolean {
    if (settled) return false
    settled = true
    timer?.cancel()
    timer = null
    cleanups = ArrayList()
    registry.forget(this)
    return true
  }
}

/** The pending procedures, by operation ID. */
internal class Procedures(val scheduler: Scheduler) {
  private val pending = LinkedHashMap<String, Procedure>()

  val size: Int
    get() = pending.size

  fun start(operationId: String, ownerId: String, responder: Responder): Procedure =
    Procedure(operationId, ownerId, responder, this).also { pending[operationId] = it }

  internal fun forget(procedure: Procedure) {
    if (pending[procedure.operationId] === procedure) pending.remove(procedure.operationId)
  }

  /** Aborts the procedure with [operationId]. An unknown ID is not an error. */
  fun abort(operationId: String, error: BleException) {
    pending[operationId]?.abort(error)
  }

  fun abortOwner(ownerId: String, error: BleException, except: Procedure? = null) {
    pending.values.filter { it.ownerId == ownerId && it !== except }.forEach { it.abort(error) }
  }

  fun abortAll(error: BleException) {
    pending.values.toList().forEach { it.abort(error) }
  }
}

internal object Deadlines {
  const val CONNECT_MS = 15_000L
  const val DISCOVERY_MS = 10_000L
  const val PROCEDURE_MS = 5_000L
  const val DISCONNECT_MS = 2_000L
  const val MTU_MS = 5_000L
}

/** The deadline of a command: [deadlineMillis], else the contract default. Null means none. */
internal fun deadlineFor(kind: String?, deadlineMillis: Long?, connectTimeoutMs: Long? = null): Long? =
  deadlineMillis ?: when (kind) {
    "connect" -> connectTimeoutMs ?: Deadlines.CONNECT_MS
    "discoverServices" -> Deadlines.DISCOVERY_MS
    "read", "write", "subscribe", "unsubscribe", "notify", "createServer", "startAdvertising" -> Deadlines.PROCEDURE_MS
    else -> null
  }
