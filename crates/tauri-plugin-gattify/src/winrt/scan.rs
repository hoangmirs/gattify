use std::{
    collections::{BTreeSet, HashMap},
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

use super::gatt::uuid_from_u128;
use crate::{
    normalize_uuid, AdvertisementData, BleError, BleResult, DeviceId, DiscoveredDevice, ErrorCode,
    ManufacturerData, ScanId, ServiceData,
};

/// At most one result per device per scan in this interval.
pub(super) const RESULT_INTERVAL: Duration = Duration::from_millis(1_000);

/// The Bluetooth base UUID, `00000000-0000-1000-8000-00805f9b34fb`.
const BASE_UUID: u128 = 0x0000_0000_0000_1000_8000_0080_5f9b_34fb;

// Advertising data types that carry service data or solicited services.
const SOLICITED_16: u8 = 0x14;
const SOLICITED_128: u8 = 0x15;
const SERVICE_DATA_16: u8 = 0x16;
const SOLICITED_32: u8 = 0x1F;
const SERVICE_DATA_32: u8 = 0x20;
const SERVICE_DATA_128: u8 = 0x21;

/// The service UUIDs of one scan. An empty filter matches every advertisement.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ScanFilter(BTreeSet<String>);

impl ScanFilter {
    pub(super) fn parse(uuids: &[String]) -> BleResult<Self> {
        uuids
            .iter()
            .map(|uuid| {
                normalize_uuid(uuid).ok_or_else(|| {
                    BleError::new(ErrorCode::InvalidArgument, format!("{uuid} is not a UUID"))
                })
            })
            .collect::<BleResult<_>>()
            .map(Self)
    }

    pub(super) fn matches(&self, advertised: &BTreeSet<String>) -> bool {
        self.0.is_empty() || !self.0.is_disjoint(advertised)
    }

    fn contains(&self, uuid: &str) -> bool {
        self.0.contains(uuid)
    }
}

/// Lets through at most one result per device every [`RESULT_INTERVAL`].
#[derive(Default)]
pub(super) struct Throttle {
    last: HashMap<String, Instant>,
}

impl Throttle {
    pub(super) fn allow(&mut self, device_id: &str, now: Instant) -> bool {
        if let Some(previous) = self.last.get(device_id) {
            if now.saturating_duration_since(*previous) < RESULT_INTERVAL {
                return false;
            }
        }
        self.last.insert(device_id.to_owned(), now);
        true
    }
}

/// One advertising data section, as far as a scan result needs it.
#[derive(Debug, Eq, PartialEq)]
enum Section {
    ServiceData(String, Vec<u8>),
    Solicited(Vec<String>),
    Other,
}

/// Reads the service data and solicited services of an advertising data
/// section. The UUIDs in a section are little-endian.
fn parse_section(data_type: u8, bytes: &[u8]) -> Section {
    let (width, solicited) = match data_type {
        SERVICE_DATA_16 => (2, false),
        SERVICE_DATA_32 => (4, false),
        SERVICE_DATA_128 => (16, false),
        SOLICITED_16 => (2, true),
        SOLICITED_32 => (4, true),
        SOLICITED_128 => (16, true),
        _ => return Section::Other,
    };
    if solicited {
        return Section::Solicited(bytes.chunks_exact(width).map(uuid_from_le).collect());
    }
    if bytes.len() < width {
        return Section::Other;
    }
    let (uuid, data) = bytes.split_at(width);
    Section::ServiceData(uuid_from_le(uuid), data.to_vec())
}

/// A 16-, 32- or 128-bit little-endian UUID as a lowercase 128-bit string.
fn uuid_from_le(bytes: &[u8]) -> String {
    let value = bytes
        .iter()
        .rev()
        .fold(0_u128, |value, byte| (value << 8) | u128::from(*byte));
    if bytes.len() == 16 {
        uuid_from_u128(value)
    } else {
        uuid_from_u128((value << 96) | BASE_UUID)
    }
}

/// Windows reports 127 when it has no signal strength.
pub(super) fn reported_rssi(raw: i16) -> Option<i16> {
    (raw != 127).then_some(raw)
}

/// What one advertising or scan response packet carries.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct Packet {
    pub(super) scan_response: bool,
    pub(super) local_name: Option<String>,
    pub(super) service_uuids: Vec<String>,
    pub(super) solicited_uuids: Vec<String>,
    pub(super) service_data: Vec<(String, Vec<u8>)>,
    pub(super) manufacturer_data: Vec<(u16, Vec<u8>)>,
    pub(super) connectable: Option<bool>,
}

impl Packet {
    /// Adds the service data or the solicited services of one section.
    pub(super) fn add_section(&mut self, data_type: u8, bytes: &[u8]) {
        match parse_section(data_type, bytes) {
            Section::ServiceData(uuid, data) => self.service_data.push((uuid, data)),
            Section::Solicited(uuids) => self.solicited_uuids.extend(uuids),
            Section::Other => {}
        }
    }
}

/// A device's latest advertisement merged with its latest scan response.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct Sighting {
    pub(super) local_name: Option<String>,
    pub(super) cached_name: Option<String>,
    pub(super) service_uuids: Vec<String>,
    /// Every UUID a scan filter can match: services, solicited services and service data.
    pub(super) match_uuids: BTreeSet<String>,
    pub(super) service_data: Vec<(String, Vec<u8>)>,
    pub(super) manufacturer_data: Vec<(u16, Vec<u8>)>,
    pub(super) connectable: Option<bool>,
}

#[derive(Default)]
struct Seen {
    advertisement: Option<Packet>,
    scan_response: Option<Packet>,
    cached_name: Option<String>,
}

/// Windows reports an advertisement and its scan response as two packets.
/// This keeps the latest of each per device, so that a filter UUID in the
/// advertisement still matches when the name arrives in the scan response.
#[derive(Default)]
pub(super) struct Sightings {
    devices: HashMap<u64, Seen>,
}

impl Sightings {
    pub(super) fn record(&mut self, address: u64, packet: Packet) -> Sighting {
        let seen = self.devices.entry(address).or_default();
        if let Some(name) = packet.local_name.as_ref().filter(|name| !name.is_empty()) {
            seen.cached_name = Some(name.clone());
        }
        if packet.scan_response {
            seen.scan_response = Some(packet);
        } else {
            seen.advertisement = Some(packet);
        }
        let parts: Vec<&Packet> = seen
            .advertisement
            .iter()
            .chain(seen.scan_response.iter())
            .collect();
        let mut sighting = Sighting {
            local_name: parts
                .iter()
                .find_map(|part| part.local_name.clone().filter(|name| !name.is_empty())),
            cached_name: seen.cached_name.clone(),
            service_uuids: Vec::new(),
            match_uuids: BTreeSet::new(),
            service_data: Vec::new(),
            manufacturer_data: Vec::new(),
            connectable: seen
                .advertisement
                .as_ref()
                .and_then(|part| part.connectable),
        };
        for part in parts {
            for uuid in &part.service_uuids {
                if !sighting.service_uuids.contains(uuid) {
                    sighting.service_uuids.push(uuid.clone());
                }
            }
            for (uuid, data) in &part.service_data {
                if !sighting.service_data.iter().any(|(known, _)| known == uuid) {
                    sighting.service_data.push((uuid.clone(), data.clone()));
                }
            }
            for (company, data) in &part.manufacturer_data {
                if !sighting
                    .manufacturer_data
                    .iter()
                    .any(|(known, _)| known == company)
                {
                    sighting.manufacturer_data.push((*company, data.clone()));
                }
            }
            sighting
                .match_uuids
                .extend(part.solicited_uuids.iter().cloned());
        }
        sighting
            .match_uuids
            .extend(sighting.service_uuids.iter().cloned());
        sighting
            .match_uuids
            .extend(sighting.service_data.iter().map(|(uuid, _)| uuid.clone()));
        sighting
    }

    /// Forgets the packets when no scan runs. Cached names stay.
    pub(super) fn forget_packets(&mut self) {
        self.devices.retain(|_, seen| {
            seen.advertisement = None;
            seen.scan_response = None;
            seen.cached_name.is_some()
        });
    }
}

/// The advertised local name; else the UTF-8 service data of a filter UUID,
/// which is how an Android or Windows host advertises its name; else the
/// cached name.
pub(super) fn device_name(sighting: &Sighting, filter: &ScanFilter) -> Option<String> {
    if let Some(name) = &sighting.local_name {
        return Some(name.clone());
    }
    sighting
        .service_data
        .iter()
        .filter(|(uuid, data)| filter.contains(uuid) && !data.is_empty())
        .find_map(|(_, data)| std::str::from_utf8(data).ok().map(str::to_owned))
        .or_else(|| sighting.cached_name.clone())
}

/// The scan result of one sighting for one scan.
pub(super) fn discovered(
    device_id: &str,
    scan_id: &ScanId,
    sighting: &Sighting,
    filter: &ScanFilter,
    rssi: Option<i16>,
    observed_at_millis: u64,
) -> DiscoveredDevice {
    DiscoveredDevice {
        id: DeviceId::new(device_id),
        name: device_name(sighting, filter),
        rssi,
        service_uuids: sighting.service_uuids.clone(),
        advertisement: Some(AdvertisementData {
            local_name: sighting.local_name.clone(),
            service_data: sighting
                .service_data
                .iter()
                .map(|(uuid, data)| ServiceData {
                    service_uuid: uuid.clone(),
                    bytes_base64: BASE64.encode(data),
                })
                .collect(),
            manufacturer_data: sighting
                .manufacturer_data
                .iter()
                .map(|(company_id, data)| ManufacturerData {
                    company_id: *company_id,
                    bytes_base64: BASE64.encode(data),
                })
                .collect(),
            connectable: sighting.connectable,
        }),
        observed_at_millis,
        scan_id: scan_id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEER: &str = "80ff87c3-8e84-4914-aedc-0d6a3ba5534d";
    const HEART_RATE: &str = "0000180d-0000-1000-8000-00805f9b34fb";

    fn filter(uuids: &[&str]) -> ScanFilter {
        ScanFilter::parse(
            &uuids
                .iter()
                .map(|uuid| (*uuid).to_owned())
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    fn uuids(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    /// The little-endian bytes of the peer service UUID.
    fn peer_le() -> Vec<u8> {
        0x80ff_87c3_8e84_4914_aedc_0d6a_3ba5_534d_u128
            .to_le_bytes()
            .to_vec()
    }

    #[test]
    fn filters_are_normalized_and_an_empty_filter_matches_every_device() {
        let short = filter(&["180D"]);
        assert!(short.matches(&uuids(&[HEART_RATE])));
        assert!(!short.matches(&uuids(&[PEER])));
        assert!(filter(&[]).matches(&uuids(&[])));
        let error = ScanFilter::parse(&["nope".to_owned()]).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
    }

    #[test]
    fn a_device_passes_once_per_second_per_scan() {
        let mut throttle = Throttle::default();
        let start = Instant::now();
        assert!(throttle.allow("device-1", start));
        assert!(!throttle.allow("device-1", start + Duration::from_millis(999)));
        assert!(throttle.allow("device-2", start + Duration::from_millis(10)));
        assert!(throttle.allow("device-1", start + RESULT_INTERVAL));
    }

    #[test]
    fn service_data_sections_expand_their_uuids() {
        assert_eq!(
            parse_section(0x16, &[0x0D, 0x18, 0x01, 0x02]),
            Section::ServiceData(HEART_RATE.into(), vec![1, 2])
        );
        assert_eq!(
            parse_section(0x20, &[0x0D, 0x18, 0x00, 0x00]),
            Section::ServiceData(HEART_RATE.into(), Vec::new())
        );
        let mut section = peer_le();
        section.extend_from_slice(b"Host");
        assert_eq!(
            parse_section(0x21, &section),
            Section::ServiceData(PEER.into(), b"Host".to_vec())
        );
        assert_eq!(parse_section(0x21, &[1, 2, 3]), Section::Other);
        assert_eq!(parse_section(0x09, b"name"), Section::Other);
    }

    #[test]
    fn solicitation_sections_list_their_uuids() {
        assert_eq!(
            parse_section(0x14, &[0x0D, 0x18, 0x0F, 0x18]),
            Section::Solicited(vec![
                HEART_RATE.into(),
                "0000180f-0000-1000-8000-00805f9b34fb".into()
            ])
        );
        assert_eq!(
            parse_section(0x15, &peer_le()),
            Section::Solicited(vec![PEER.into()])
        );
        assert_eq!(
            parse_section(0x1F, &[0x0D, 0x18, 0x00, 0x00]),
            Section::Solicited(vec![HEART_RATE.into()])
        );
    }

    #[test]
    fn an_unknown_signal_strength_is_none() {
        assert_eq!(reported_rssi(127), None);
        assert_eq!(reported_rssi(-58), Some(-58));
    }

    fn advertisement() -> Packet {
        Packet {
            service_uuids: vec![PEER.into()],
            connectable: Some(true),
            ..Packet::default()
        }
    }

    fn scan_response(name_bytes: &[u8]) -> Packet {
        let mut packet = Packet {
            scan_response: true,
            ..Packet::default()
        };
        let mut section = peer_le();
        section.extend_from_slice(name_bytes);
        packet.add_section(0x21, &section);
        packet.add_section(0x09, b"ignored");
        packet
    }

    #[test]
    fn a_filter_uuid_in_the_advertisement_still_matches_when_the_name_arrives_later() {
        let mut sightings = Sightings::default();
        let first = sightings.record(1, advertisement());
        assert_eq!(device_name(&first, &filter(&[PEER])), None);

        let merged = sightings.record(1, scan_response(b"Host A"));
        assert!(filter(&[PEER]).matches(&merged.match_uuids));
        assert_eq!(merged.service_uuids, vec![PEER.to_owned()]);
        assert_eq!(merged.connectable, Some(true));
        assert_eq!(
            device_name(&merged, &filter(&[PEER])).as_deref(),
            Some("Host A")
        );
    }

    #[test]
    fn the_name_rule_prefers_the_local_name_then_filter_service_data_then_the_cache() {
        let mut sightings = Sightings::default();
        sightings.record(
            1,
            Packet {
                local_name: Some("Cached".into()),
                ..advertisement()
            },
        );
        let mut named = sightings.record(1, scan_response(b"Host A"));
        assert_eq!(named.local_name.as_deref(), Some("Cached"));
        assert_eq!(
            device_name(&named, &filter(&[PEER])).as_deref(),
            Some("Cached")
        );

        named.local_name = None;
        assert_eq!(
            device_name(&named, &filter(&[PEER])).as_deref(),
            Some("Host A")
        );
        assert_eq!(
            device_name(&named, &filter(&[HEART_RATE])).as_deref(),
            Some("Cached")
        );
        assert_eq!(device_name(&named, &filter(&[])).as_deref(), Some("Cached"));

        let invalid = sightings.record(2, scan_response(&[0xFF, 0xFE]));
        assert_eq!(device_name(&invalid, &filter(&[PEER])), None);
    }

    #[test]
    fn service_data_uuids_and_solicited_uuids_match_filters() {
        let mut sightings = Sightings::default();
        let data_only = sightings.record(1, scan_response(b"x"));
        assert!(filter(&[PEER]).matches(&data_only.match_uuids));
        assert_eq!(data_only.connectable, None);

        let mut solicited = Packet::default();
        solicited.add_section(0x14, &[0x0D, 0x18]);
        let sighting = sightings.record(2, solicited);
        assert!(filter(&[HEART_RATE]).matches(&sighting.match_uuids));
        assert!(sighting.service_uuids.is_empty());
    }

    #[test]
    fn the_merge_keeps_the_advertisement_fields_first_and_adds_new_ones() {
        let mut sightings = Sightings::default();
        sightings.record(
            1,
            Packet {
                manufacturer_data: vec![(76, vec![1])],
                service_data: vec![(HEART_RATE.into(), vec![1])],
                ..advertisement()
            },
        );
        let merged = sightings.record(
            1,
            Packet {
                scan_response: true,
                service_uuids: vec![HEART_RATE.into(), PEER.into()],
                manufacturer_data: vec![(76, vec![2]), (6, vec![3])],
                service_data: vec![(HEART_RATE.into(), vec![2])],
                ..Packet::default()
            },
        );
        assert_eq!(
            merged.service_uuids,
            vec![PEER.to_owned(), HEART_RATE.into()]
        );
        assert_eq!(merged.manufacturer_data, vec![(76, vec![1]), (6, vec![3])]);
        assert_eq!(merged.service_data, vec![(HEART_RATE.to_owned(), vec![1])]);
    }

    #[test]
    fn forgetting_packets_keeps_the_cached_name() {
        let mut sightings = Sightings::default();
        sightings.record(
            1,
            Packet {
                local_name: Some("Old".into()),
                ..advertisement()
            },
        );
        sightings.record(2, advertisement());
        sightings.forget_packets();
        assert_eq!(sightings.devices.len(), 1);
        let later = sightings.record(1, advertisement());
        assert_eq!(later.local_name, None);
        assert_eq!(
            device_name(&later, &filter(&[PEER])).as_deref(),
            Some("Old")
        );
    }

    #[test]
    fn a_result_carries_the_merged_fields_and_its_scan() {
        let mut sightings = Sightings::default();
        sightings.record(
            1,
            Packet {
                manufacturer_data: vec![(76, vec![1, 2])],
                ..advertisement()
            },
        );
        let sighting = sightings.record(1, scan_response(b"Host A"));
        let device = discovered(
            "device-3",
            &ScanId::new("scan-1"),
            &sighting,
            &filter(&[PEER]),
            Some(-58),
            1_757_664_000_000,
        );
        let json = serde_json::to_value(&device).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "id": "device-3",
                "name": "Host A",
                "rssi": -58,
                "serviceUuids": [PEER],
                "advertisement": {
                    "localName": null,
                    "serviceData": [{ "serviceUuid": PEER, "bytesBase64": "SG9zdCBB" }],
                    "manufacturerData": [{ "companyId": 76, "bytesBase64": "AQI=" }],
                    "connectable": true
                },
                "observedAtMillis": 1_757_664_000_000_u64,
                "scanId": "scan-1"
            })
        );
    }
}
