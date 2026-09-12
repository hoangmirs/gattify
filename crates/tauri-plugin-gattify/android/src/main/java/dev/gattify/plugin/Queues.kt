package dev.gattify.plugin

import java.io.ByteArrayOutputStream

/** A FIFO with at most one operation in flight. */
internal class OpQueue<T : Any> {
  private val waiting = ArrayDeque<T>()

  var inFlight: T? = null
    private set

  val size: Int
    get() = waiting.size + (if (inFlight == null) 0 else 1)

  fun enqueue(op: T) {
    waiting.addLast(op)
  }

  /** Takes the next operation when none is in flight. Drops each waiting one that [skip] matches. */
  fun next(skip: (T) -> Boolean = { false }): T? {
    if (inFlight != null) return null
    while (true) {
      val op = waiting.removeFirstOrNull() ?: return null
      if (!skip(op)) {
        inFlight = op
        return op
      }
    }
  }

  /** Ends [op] when it is the one in flight. */
  fun complete(op: T): Boolean {
    if (inFlight !== op) return false
    inFlight = null
    return true
  }

  fun removeWaiting(predicate: (T) -> Boolean): List<T> {
    val removed = waiting.filter(predicate)
    waiting.removeAll(predicate)
    return removed
  }

  /** Removes every operation, the one in flight first. */
  fun drain(): List<T> {
    val all = listOfNotNull(inFlight) + waiting
    waiting.clear()
    inFlight = null
    return all
  }
}

/** Lets one event through per key per interval. */
internal class Throttle(private val intervalMs: Long) {
  private val last = HashMap<String, Long>()

  fun allow(key: String, now: Long): Boolean {
    val previous = last[key]
    if (previous != null && now - previous < intervalMs) return false
    last[key] = now
    return true
  }
}

/**
 * The parts of the long writes of one central, kept per characteristic in the
 * order of the first part. A failed part fails the whole execute, as ATT requires.
 */
internal class PreparedWrites<K : Any> {
  private val parts = LinkedHashMap<K, ByteArrayOutputStream>()
  private var failure = Att.SUCCESS

  /** The length assembled so far for [key]: the offset the next part must have. */
  fun length(key: K): Int = parts[key]?.size() ?: 0

  fun append(key: K, bytes: ByteArray) {
    parts.getOrPut(key) { ByteArrayOutputStream() }.write(bytes, 0, bytes.size)
  }

  /** Records the ATT error of a rejected part. The first error answers the execute. */
  fun fail(status: Int) {
    if (failure == Att.SUCCESS) failure = status
  }

  /**
   * Ends the long write and forgets its parts. Returns the ATT status of the
   * execute answer and, when [execute] is true and no part failed, one value per
   * characteristic in the order of its first part.
   */
  fun finish(execute: Boolean): Pair<Int, List<Pair<K, ByteArray>>> {
    val status = if (execute) failure else Att.SUCCESS
    val writes = if (execute && failure == Att.SUCCESS) parts.map { (key, bytes) -> key to bytes.toByteArray() } else emptyList()
    clear()
    return status to writes
  }

  fun discard(predicate: (K) -> Boolean) {
    parts.keys.removeAll(predicate)
  }

  fun clear() {
    parts.clear()
    failure = Att.SUCCESS
  }
}
