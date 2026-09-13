use std::future::IntoFuture;

use windows::{
    core::IInspectable,
    Devices::{
        Bluetooth::BluetoothAdapter,
        Radios::{Radio, RadioState},
    },
};

use super::{
    convert::sender_handler,
    engine::{Engine, Job},
    status::{adapter_state, readiness_error, AdapterFacts, RadioPower},
};
use crate::{AdapterState, BleError};

/// The default adapter and its lookup.
#[derive(Default)]
pub(super) struct AdapterSlot {
    /// The adapter of the last lookup, when it found one. A new lookup
    /// replaces it.
    loaded: Option<LoadedAdapter>,
    /// The commands that wait for a lookup that runs.
    waiting: Option<Vec<Job>>,
}

struct LoadedAdapter {
    /// Tells the radio events of this adapter from those of one it replaced.
    generation: u64,
    adapter: BluetoothAdapter,
    facts: AdapterFacts,
    /// Without it the adapter counts as on.
    radio: Option<WatchedRadio>,
    /// A `GetRadioAsync` runs in the background.
    radio_lookup: bool,
}

impl LoadedAdapter {
    fn release(self) {
        if let Some(watched) = self.radio {
            let _ = watched.radio.RemoveStateChanged(watched.token);
        }
    }
}

/// A radio with its `StateChanged` handler.
struct WatchedRadio {
    radio: Radio,
    token: i64,
}

impl Engine {
    /// Runs `then` once the default adapter is known, while the operation is
    /// pending. With `need_on`, a command rejects first when the adapter is
    /// not on. While the adapter is not on, each command looks it up again
    /// first, so that a replaced adapter or a reinstalled driver counts. A
    /// missing radio is looked up again in the background.
    pub(super) fn when_adapter(
        &mut self,
        key: u64,
        need_on: bool,
        then: impl FnOnce(&mut Engine, u64) + Send + 'static,
    ) {
        let job: Job = Box::new(move |engine| {
            if !engine.pending(key) {
                return;
            }
            if need_on {
                if let Some(error) = engine.radio_error() {
                    engine.reject(key, error);
                    return;
                }
            }
            then(engine, key);
        });
        if let Some(waiters) = &mut self.adapter.waiting {
            waiters.push(job);
            return;
        }
        if self.adapter_state() == AdapterState::PoweredOn {
            self.find_radio();
            job(self);
            return;
        }
        self.adapter.waiting = Some(vec![job]);
        self.load_adapter();
    }

    /// The rejection for a command that needs the adapter on, or `None` when it is on.
    pub(super) fn radio_error(&self) -> Option<BleError> {
        readiness_error(self.adapter_state())
    }

    pub(super) fn adapter_state(&self) -> AdapterState {
        match &self.adapter.loaded {
            Some(loaded) => adapter_state(
                Some(&loaded.facts),
                loaded
                    .radio
                    .as_ref()
                    .map(|watched| radio_power(&watched.radio)),
            ),
            None => adapter_state(None, None),
        }
    }

    pub(super) fn adapter_facts(&self) -> Option<AdapterFacts> {
        self.adapter.loaded.as_ref().map(|loaded| loaded.facts)
    }

    fn load_adapter(&mut self) {
        let generation = self.token();
        match BluetoothAdapter::GetDefaultAsync() {
            Ok(operation) => self.spawn(operation.into_future(), move |engine, adapter| {
                engine.adapter_found(generation, adapter);
            }),
            Err(_) => self.adapter_loaded(None),
        }
    }

    /// `GetDefaultAsync` gives a null adapter when the computer has none.
    fn adapter_found(&mut self, generation: u64, adapter: windows::core::Result<BluetoothAdapter>) {
        let Ok(adapter) = adapter else {
            self.adapter_loaded(None);
            return;
        };
        let facts = AdapterFacts {
            low_energy: adapter.IsLowEnergySupported().unwrap_or(false),
            central: adapter.IsCentralRoleSupported().unwrap_or(false),
            peripheral: adapter.IsPeripheralRoleSupported().unwrap_or(false),
            max_advertisement_data_length: adapter.MaxAdvertisementDataLength().ok(),
        };
        let loaded = LoadedAdapter {
            generation,
            adapter,
            facts,
            radio: None,
            radio_lookup: false,
        };
        match loaded.adapter.GetRadioAsync() {
            Ok(operation) => self.spawn(operation.into_future(), move |engine, radio| {
                let radio = radio
                    .ok()
                    .and_then(|radio| engine.watch_radio(radio, generation));
                engine.adapter_loaded(Some(LoadedAdapter { radio, ..loaded }));
            }),
            Err(_) => self.adapter_loaded(Some(loaded)),
        }
    }

    /// Registers the `StateChanged` handler of a radio. The handler reads the
    /// state on the thread of the event, so that each change arrives in order
    /// even when the radio changes again before the engine runs. A radio whose
    /// handler cannot be registered is treated as missing.
    fn watch_radio(&self, radio: Radio, generation: u64) -> Option<WatchedRadio> {
        let poster = self.poster.clone();
        let token = radio
            .StateChanged(&sender_handler::<Radio, IInspectable>(move |sender| {
                if let Some(power) = sender.map(radio_power) {
                    poster.post(move |engine| engine.radio_changed(generation, power));
                }
            }))
            .ok()?;
        Some(WatchedRadio { radio, token })
    }

    /// Replaces the adapter of the last lookup, announces a changed state,
    /// then runs the commands that waited.
    fn adapter_loaded(&mut self, loaded: Option<LoadedAdapter>) {
        if let Some(previous) = std::mem::replace(&mut self.adapter.loaded, loaded) {
            previous.release();
        }
        let waiters = self.adapter.waiting.take().unwrap_or_default();
        self.announce(self.adapter_state());
        for waiter in waiters {
            waiter(self);
        }
    }

    /// Looks for the radio of the loaded adapter again while it has none.
    /// One lookup runs at a time.
    fn find_radio(&mut self) {
        let Some(loaded) = &mut self.adapter.loaded else {
            return;
        };
        if loaded.radio.is_some() || loaded.radio_lookup {
            return;
        }
        let Ok(operation) = loaded.adapter.GetRadioAsync() else {
            return;
        };
        loaded.radio_lookup = true;
        let generation = loaded.generation;
        self.spawn(operation.into_future(), move |engine, radio| {
            if engine.loaded_generation() != Some(generation) {
                return;
            }
            let radio = radio
                .ok()
                .and_then(|radio| engine.watch_radio(radio, generation));
            let Some(loaded) = &mut engine.adapter.loaded else {
                return;
            };
            loaded.radio_lookup = false;
            if radio.is_some() {
                loaded.radio = radio;
                engine.announce(engine.adapter_state());
            }
        });
    }

    fn loaded_generation(&self) -> Option<u64> {
        self.adapter.loaded.as_ref().map(|loaded| loaded.generation)
    }

    /// A state that the radio of adapter `generation` reported, in the order
    /// it reported them.
    fn radio_changed(&mut self, generation: u64, power: RadioPower) {
        let Some(loaded) = self
            .adapter
            .loaded
            .as_ref()
            .filter(|loaded| loaded.generation == generation)
        else {
            return;
        };
        let state = adapter_state(Some(&loaded.facts), Some(power));
        self.announce(state);
    }

    /// Reads the radio now and handles a change it missed. A stopped watcher
    /// calls this first, so that `adapterStateChanged` comes before the end
    /// of its scans.
    pub(super) fn recheck_radio(&mut self) {
        let reading = self.adapter.loaded.as_ref().and_then(|loaded| {
            let watched = loaded.radio.as_ref()?;
            Some((loaded.generation, radio_power(&watched.radio)))
        });
        if let Some((generation, power)) = reading {
            self.radio_changed(generation, power);
        }
    }

    fn announce(&mut self, state: AdapterState) {
        if let Some(state) = self.adapter_states.observe(state) {
            self.adapter_changed(state);
        }
    }
}

fn radio_power(radio: &Radio) -> RadioPower {
    match radio.State() {
        Ok(RadioState::On) => RadioPower::On,
        Ok(RadioState::Off) => RadioPower::Off,
        Ok(RadioState::Disabled) => RadioPower::Disabled,
        _ => RadioPower::Unknown,
    }
}
