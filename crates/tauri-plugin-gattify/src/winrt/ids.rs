use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
};

/// Allocates `<prefix>-<n>` IDs. Each prefix counts from 1 and never reuses a number.
#[derive(Default)]
pub(super) struct IdAllocator {
    counters: HashMap<&'static str, u64>,
}

impl IdAllocator {
    pub(super) fn next(&mut self, prefix: &'static str) -> String {
        let counter = self.counters.entry(prefix).or_default();
        *counter += 1;
        format!("{prefix}-{counter}")
    }
}

/// The text after the first `:` of an owner ID. An owner ID without `:` is its own family.
pub(super) fn owner_family(owner: &str) -> &str {
    owner.split_once(':').map_or(owner, |(_, family)| family)
}

/// Gives each remote one opaque ID for the life of the process, so that no
/// address leaves the backend, and records the owner families that saw it.
pub(super) struct Remotes<K> {
    prefix: &'static str,
    ids: HashMap<K, String>,
    keys: HashMap<String, K>,
    families: HashMap<String, HashSet<String>>,
}

impl<K: Clone + Eq + Hash> Remotes<K> {
    pub(super) fn new(prefix: &'static str) -> Self {
        Self {
            prefix,
            ids: HashMap::new(),
            keys: HashMap::new(),
            families: HashMap::new(),
        }
    }

    pub(super) fn id_for(&mut self, key: &K, allocator: &mut IdAllocator) -> String {
        if let Some(id) = self.ids.get(key) {
            return id.clone();
        }
        let id = allocator.next(self.prefix);
        self.ids.insert(key.clone(), id.clone());
        self.keys.insert(id.clone(), key.clone());
        id
    }

    pub(super) fn key_of(&self, id: &str) -> Option<&K> {
        self.keys.get(id)
    }

    pub(super) fn mark_seen(&mut self, id: &str, owner: &str) {
        self.families
            .entry(id.to_owned())
            .or_default()
            .insert(owner_family(owner).to_owned());
    }

    /// Whether an owner of the family of `owner` received `id` in a scan result.
    pub(super) fn seen_by_family_of(&self, id: &str, owner: &str) -> bool {
        self.families
            .get(id)
            .is_some_and(|families| families.contains(owner_family(owner)))
    }
}

/// Names one attribute of a remote database: its ATT handle and its UUID.
pub(super) type AttributeKey = (u16, String);

/// The service and characteristic handles of one connection. An attribute
/// keeps its handle across discoveries, keyed by its ATT handle and UUID.
pub(super) struct HandleTable<T> {
    connection_id: String,
    services: HashMap<AttributeKey, String>,
    characteristics: HashMap<AttributeKey, String>,
    attributes: HashMap<String, T>,
}

impl<T> HandleTable<T> {
    pub(super) fn new(connection_id: &str) -> Self {
        Self {
            connection_id: connection_id.to_owned(),
            services: HashMap::new(),
            characteristics: HashMap::new(),
            attributes: HashMap::new(),
        }
    }

    /// Forgets the attributes of the last discovery. Their handles stay reserved.
    pub(super) fn forget_attributes(&mut self) {
        self.attributes.clear();
    }

    pub(super) fn service(&mut self, key: AttributeKey) -> String {
        let next = self.services.len() + 1;
        let connection_id = &self.connection_id;
        self.services
            .entry(key)
            .or_insert_with(|| format!("{connection_id}/service-{next}"))
            .clone()
    }

    /// The handle for `key`, now bound to `attribute` from the latest discovery.
    pub(super) fn characteristic(&mut self, key: AttributeKey, attribute: T) -> String {
        let next = self.characteristics.len() + 1;
        let connection_id = &self.connection_id;
        let handle = self
            .characteristics
            .entry(key)
            .or_insert_with(|| format!("{connection_id}/characteristic-{next}"))
            .clone();
        self.attributes.insert(handle.clone(), attribute);
        handle
    }

    pub(super) fn get(&self, handle: &str) -> Option<&T> {
        self.attributes.get(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_prefix_counts_from_one_without_reuse() {
        let mut ids = IdAllocator::default();
        assert_eq!(ids.next("scan"), "scan-1");
        assert_eq!(ids.next("connection"), "connection-1");
        assert_eq!(ids.next("scan"), "scan-2");
    }

    #[test]
    fn the_owner_family_follows_the_first_colon() {
        assert_eq!(owner_family("webview:main"), "main");
        assert_eq!(owner_family("gattify-peer:main"), "main");
        assert_eq!(owner_family("a:b:c"), "b:c");
        assert_eq!(owner_family("plain"), "plain");
    }

    #[test]
    fn a_remote_keeps_its_id_and_hides_its_address() {
        let mut ids = IdAllocator::default();
        let mut devices = Remotes::new("device");
        let first = devices.id_for(&0xAABB_CCDD_EEFF_u64, &mut ids);
        let second = devices.id_for(&0x1122_3344_5566_u64, &mut ids);
        assert_eq!(first, "device-1");
        assert_eq!(second, "device-2");
        assert_eq!(devices.id_for(&0xAABB_CCDD_EEFF_u64, &mut ids), "device-1");
        assert_eq!(devices.key_of("device-2"), Some(&0x1122_3344_5566_u64));
        assert_eq!(devices.key_of("device-3"), None);
    }

    #[test]
    fn a_device_is_usable_by_every_owner_of_the_family_that_saw_it() {
        let mut ids = IdAllocator::default();
        let mut devices = Remotes::new("device");
        let id = devices.id_for(&7_u64, &mut ids);
        assert!(!devices.seen_by_family_of(&id, "webview:main"));
        devices.mark_seen(&id, "webview:main");
        assert!(devices.seen_by_family_of(&id, "gattify-peer:main"));
        assert!(!devices.seen_by_family_of(&id, "webview:other"));
    }

    #[test]
    fn a_second_discovery_returns_the_same_handles() {
        let mut table = HandleTable::new("connection-2");
        let service = table.service((1, "s".into()));
        let tx = table.characteristic((3, "tx".into()), "first tx");
        let rx = table.characteristic((5, "rx".into()), "first rx");
        assert_eq!(service, "connection-2/service-1");
        assert_eq!(tx, "connection-2/characteristic-1");
        assert_eq!(rx, "connection-2/characteristic-2");

        table.forget_attributes();
        assert_eq!(table.get(&tx), None);
        assert_eq!(table.characteristic((5, "rx".into()), "second rx"), rx);
        assert_eq!(table.service((1, "s".into())), service);
        assert_eq!(table.get(&rx), Some(&"second rx"));
        assert_eq!(table.get(&tx), None);
        assert_eq!(
            table.characteristic((9, "new".into()), "new"),
            "connection-2/characteristic-3"
        );
    }
}
