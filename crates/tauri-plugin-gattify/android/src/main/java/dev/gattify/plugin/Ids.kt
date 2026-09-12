package dev.gattify.plugin

/** Allocates `<prefix>-<n>` IDs. Each prefix counts from 1, and no ID repeats in the process. */
internal class IdAllocator {
  private val counters = HashMap<String, Long>()

  fun next(prefix: String): String {
    val n = (counters[prefix] ?: 0L) + 1
    counters[prefix] = n
    return "$prefix-$n"
  }
}

/** The owner family: the text after the first `:`. `webview:main` and `gattify-peer:main` share `main`. */
internal fun ownerFamily(ownerId: String): String = ownerId.substringAfter(':', ownerId)

/**
 * Gives each remote address one opaque ID for the life of the process, so that no
 * address leaves the native layer, and records the owner families that saw it.
 */
internal class RemoteRegistry(private val prefix: String, private val ids: IdAllocator) {
  private val idsByAddress = HashMap<String, String>()
  private val addressesById = HashMap<String, String>()
  private val families = HashMap<String, MutableSet<String>>()

  fun idFor(address: String): String = idsByAddress.getOrPut(address) {
    ids.next(prefix).also { addressesById[it] = address }
  }

  fun addressOf(id: String): String? = addressesById[id]

  fun markSeen(id: String, ownerId: String) {
    families.getOrPut(id) { HashSet() }.add(ownerFamily(ownerId))
  }

  fun seenByFamilyOf(id: String, ownerId: String): Boolean =
    families[id]?.contains(ownerFamily(ownerId)) == true
}

/**
 * The service and characteristic handles of one connection. A key names an
 * attribute across discoveries, so a second discovery returns the same handle.
 */
internal class HandleTable<T : Any>(private val connectionId: String) {
  private var services = 0
  private var characteristics = 0
  private val serviceIds = HashMap<String, String>()
  private val characteristicIds = HashMap<String, String>()
  private val attributes = HashMap<String, T>()

  fun service(key: String): String =
    serviceIds.getOrPut(key) { "$connectionId/service-${++services}" }

  /** The handle for [key], now bound to [attribute] from the latest discovery. */
  fun characteristic(key: String, attribute: T): String {
    val id = characteristicIds.getOrPut(key) { "$connectionId/characteristic-${++characteristics}" }
    attributes[id] = attribute
    return id
  }

  fun idOf(key: String): String? = characteristicIds[key]

  operator fun get(id: String): T? = attributes[id]
}
