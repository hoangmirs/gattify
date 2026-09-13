use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::task::AbortHandle;
use windows::Devices::Bluetooth::{
    Advertisement::{
        BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementType,
        BluetoothLEAdvertisementWatcher, BluetoothLEAdvertisementWatcherStoppedEventArgs,
        BluetoothLEScanningMode,
    },
    BluetoothAddressType,
};

use super::{
    convert::{bytes, handler, items, uuid, winrt_error},
    engine::Engine,
    scan::{discovered, reported_rssi, Packet, ScanFilter, Throttle},
};
use crate::{BleError, BleResult, Event, OwnerId, Reply, ScanId, ScanOptions};

pub(super) struct ScanRecord {
    pub(super) owner: OwnerId,
    filter: ScanFilter,
    throttle: Throttle,
    timer: Option<AbortHandle>,
}

/// The one watcher of the process. It runs unfiltered in active mode, so
/// scan responses arrive too, and each packet goes to every matching scan.
pub(super) struct WatcherRecord {
    watcher: BluetoothLEAdvertisementWatcher,
    generation: u64,
    received: i64,
    stopped: i64,
}

impl WatcherRecord {
    fn release(self) {
        let _ = self.watcher.RemoveReceived(self.received);
        let _ = self.watcher.RemoveStopped(self.stopped);
        let _ = self.watcher.Stop();
    }
}

/// One received packet, read on the thread of the watcher.
struct Received {
    address: u64,
    address_type: Option<BluetoothAddressType>,
    rssi: Option<i16>,
    packet: Packet,
}

impl Engine {
    pub(super) fn start_scan(&mut self, key: u64, options: &ScanOptions) {
        let filter = match ScanFilter::parse(&options.service_uuids) {
            Ok(filter) => filter,
            Err(error) => return self.reject(key, error),
        };
        let timeout = options.timeout_ms;
        self.when_adapter(key, true, move |engine, key| {
            let Some(owner) = engine.owner(key) else {
                return;
            };
            let scan_id = ScanId::new(engine.ids.next("scan"));
            let timer = timeout.map(|timeout| {
                let scan_id = scan_id.clone();
                engine.after(Duration::from_millis(timeout), move |engine| {
                    engine.end_scan(&scan_id, true);
                })
            });
            engine.scans.insert(
                scan_id.clone(),
                ScanRecord {
                    owner,
                    filter,
                    throttle: Throttle::default(),
                    timer,
                },
            );
            match engine.ensure_watcher() {
                Ok(()) => engine.resolve(key, Reply::ScanStarted { scan_id }),
                Err(error) => {
                    engine.end_scan(&scan_id, false);
                    engine.reject(key, error);
                }
            }
        });
    }

    pub(super) fn stop_scan(&mut self, key: u64, scan_id: &ScanId) {
        let owner = self.owner(key);
        if self
            .scans
            .get(scan_id)
            .is_none_or(|scan| Some(&scan.owner) != owner.as_ref())
        {
            return self.reject(key, BleError::invalid_handle(scan_id.clone()));
        }
        self.end_scan(scan_id, false);
        self.resolve(key, Reply::Empty);
    }

    fn end_scan(&mut self, scan_id: &ScanId, notify: bool) {
        let Some(scan) = self.scans.remove(scan_id) else {
            return;
        };
        if let Some(timer) = scan.timer {
            timer.abort();
        }
        if self.scans.is_empty() {
            self.stop_watcher();
        }
        if notify {
            self.emit(
                &scan.owner,
                Event::ScanStopped {
                    scan_id: scan_id.clone(),
                },
            );
        }
    }

    /// Ends the scans of `owner` without events.
    pub(super) fn release_scans(&mut self, owner: &OwnerId) {
        let targets: Vec<ScanId> = self
            .scans
            .iter()
            .filter(|(_, scan)| &scan.owner == owner)
            .map(|(scan_id, _)| scan_id.clone())
            .collect();
        for scan_id in targets {
            self.end_scan(&scan_id, false);
        }
    }

    /// The radio stopped: every scan ends with `scanStopped`.
    pub(super) fn scans_lost(&mut self) {
        self.stop_watcher();
        let all: Vec<ScanId> = self.scans.keys().cloned().collect();
        for scan_id in all {
            self.end_scan(&scan_id, true);
        }
    }

    fn stop_watcher(&mut self) {
        if let Some(watcher) = self.watcher.take() {
            watcher.release();
        }
        self.sightings.forget_packets();
    }

    fn ensure_watcher(&mut self) -> BleResult<()> {
        if self.watcher.is_some() {
            return Ok(());
        }
        let generation = self.token();
        let failed = |error: windows::core::Error| winrt_error(&error, "starting the scan");
        let watcher = BluetoothLEAdvertisementWatcher::new().map_err(failed)?;
        watcher
            .SetScanningMode(BluetoothLEScanningMode::Active)
            .map_err(failed)?;
        let poster = self.poster.clone();
        let received = watcher
            .Received(&handler::<
                BluetoothLEAdvertisementWatcher,
                BluetoothLEAdvertisementReceivedEventArgs,
            >(move |args| {
                if let Some(received) = args.and_then(|args| read_packet(args).ok()) {
                    poster.post(move |engine| engine.advertisement_received(generation, received));
                }
            }))
            .map_err(failed)?;
        let poster = self.poster.clone();
        let stopped = watcher
            .Stopped(&handler::<
                BluetoothLEAdvertisementWatcher,
                BluetoothLEAdvertisementWatcherStoppedEventArgs,
            >(move |_| {
                poster.post(move |engine| engine.watcher_stopped(generation));
            }))
            .map_err(failed)?;
        let record = WatcherRecord {
            watcher,
            generation,
            received,
            stopped,
        };
        if let Err(error) = record.watcher.Start() {
            record.release();
            return Err(failed(error));
        }
        self.watcher = Some(record);
        Ok(())
    }

    /// The watcher stopped on its own, for example after a radio change. The
    /// radio change can arrive later on another thread, so it is read first:
    /// `adapterStateChanged` then comes before `scanStopped`, and reaches the
    /// owners that held only scans.
    fn watcher_stopped(&mut self, generation: u64) {
        self.recheck_radio();
        if self.watcher.as_ref().map(|watcher| watcher.generation) == Some(generation) {
            self.scans_lost();
        }
    }

    fn advertisement_received(&mut self, generation: u64, received: Received) {
        if self.watcher.as_ref().map(|watcher| watcher.generation) != Some(generation) {
            return;
        }
        let sighting = self.sightings.record(received.address, received.packet);
        let now = Instant::now();
        let observed_at = wall_clock_millis();
        let Self {
            scans,
            devices,
            device_links,
            ids,
            ..
        } = self;
        let mut results = Vec::new();
        for (scan_id, scan) in scans.iter_mut() {
            if !scan.filter.matches(&sighting.match_uuids) {
                continue;
            }
            let device_id = devices.id_for(&received.address, ids);
            if !scan.throttle.allow(&device_id, now) {
                continue;
            }
            devices.mark_seen(&device_id, scan.owner.as_str());
            if received.address_type.is_some() {
                device_links
                    .entry(device_id.clone())
                    .or_default()
                    .address_type = received.address_type;
            }
            let device = discovered(
                &device_id,
                scan_id,
                &sighting,
                &scan.filter,
                received.rssi,
                observed_at,
            );
            results.push((scan.owner.clone(), device));
        }
        for (owner, device) in results {
            self.emit(&owner, Event::ScanResult { device });
        }
    }
}

/// Reads what a scan result needs from one packet.
fn read_packet(
    args: &BluetoothLEAdvertisementReceivedEventArgs,
) -> windows::core::Result<Received> {
    let kind = args.AdvertisementType()?;
    let advertisement = args.Advertisement()?;
    let mut packet = Packet {
        // Extended advertising marks a scan response with IsScanResponse, on Windows 10 2004 and later.
        scan_response: kind == BluetoothLEAdvertisementType::ScanResponse
            || args.IsScanResponse().unwrap_or(false),
        local_name: advertisement
            .LocalName()
            .ok()
            .map(|name| name.to_string_lossy())
            .filter(|name| !name.is_empty()),
        service_uuids: items(&advertisement.ServiceUuids()?.GetView()?)?
            .into_iter()
            .map(uuid)
            .collect(),
        connectable: match kind {
            BluetoothLEAdvertisementType::ConnectableUndirected
            | BluetoothLEAdvertisementType::ConnectableDirected => Some(true),
            BluetoothLEAdvertisementType::ScannableUndirected
            | BluetoothLEAdvertisementType::NonConnectableUndirected => Some(false),
            BluetoothLEAdvertisementType::Extended => args.IsConnectable().ok(),
            _ => None,
        },
        ..Packet::default()
    };
    for entry in items(&advertisement.ManufacturerData()?.GetView()?)? {
        packet
            .manufacturer_data
            .push((entry.CompanyId()?, bytes(&entry.Data()?)?));
    }
    for section in items(&advertisement.DataSections()?.GetView()?)? {
        packet.add_section(section.DataType()?, &bytes(&section.Data()?)?);
    }
    Ok(Received {
        address: args.BluetoothAddress()?,
        address_type: args.BluetoothAddressType().ok(),
        rssi: args.RawSignalStrengthInDBm().ok().and_then(reported_rssi),
        packet,
    })
}

fn wall_clock_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}
