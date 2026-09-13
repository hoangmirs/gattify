use std::future::IntoFuture;

use windows::{
    core::IInspectable,
    Devices::{
        Bluetooth::BluetoothAdapter,
        Radios::{Radio, RadioState},
    },
};

use super::{
    convert::handler,
    engine::{Engine, Job},
    status::{adapter_state, readiness_error, AdapterFacts, RadioPower},
};
use crate::{AdapterState, BleError};

pub(super) enum AdapterSlot {
    Unloaded,
    /// The commands that wait for the adapter.
    Loading(Vec<Job>),
    Loaded(LoadedAdapter),
}

pub(super) struct LoadedAdapter {
    facts: AdapterFacts,
    /// Kept so that its `StateChanged` handler stays registered.
    radio: Option<Radio>,
}

impl Engine {
    /// Runs `then` once the default adapter is known, while the operation is
    /// pending. With `need_on`, a command rejects first when the adapter is
    /// not on. A missing adapter is looked up again by the next command.
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
        match &mut self.adapter {
            AdapterSlot::Loaded(_) => {}
            AdapterSlot::Loading(waiters) => {
                waiters.push(job);
                return;
            }
            AdapterSlot::Unloaded => {
                self.adapter = AdapterSlot::Loading(vec![job]);
                self.load_adapter();
                return;
            }
        }
        job(self);
    }

    /// The rejection for a command that needs the adapter on, or `None` when it is on.
    pub(super) fn radio_error(&self) -> Option<BleError> {
        readiness_error(self.adapter_state())
    }

    pub(super) fn adapter_state(&self) -> AdapterState {
        match &self.adapter {
            AdapterSlot::Loaded(loaded) => {
                adapter_state(Some(&loaded.facts), loaded.radio.as_ref().map(radio_power))
            }
            AdapterSlot::Unloaded | AdapterSlot::Loading(_) => adapter_state(None, None),
        }
    }

    pub(super) fn adapter_facts(&self) -> Option<AdapterFacts> {
        match &self.adapter {
            AdapterSlot::Loaded(loaded) => Some(loaded.facts),
            AdapterSlot::Unloaded | AdapterSlot::Loading(_) => None,
        }
    }

    fn load_adapter(&mut self) {
        match BluetoothAdapter::GetDefaultAsync() {
            Ok(operation) => self.spawn(operation.into_future(), Engine::adapter_found),
            Err(_) => self.adapter_loaded(None),
        }
    }

    /// `GetDefaultAsync` gives a null adapter when the computer has none.
    fn adapter_found(&mut self, adapter: windows::core::Result<BluetoothAdapter>) {
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
        match adapter.GetRadioAsync() {
            Ok(operation) => self.spawn(operation.into_future(), move |engine, radio| {
                engine.radio_found(facts, radio.ok());
            }),
            Err(_) => self.radio_found(facts, None),
        }
    }

    fn radio_found(&mut self, facts: AdapterFacts, radio: Option<Radio>) {
        if let Some(radio) = &radio {
            let poster = self.poster.clone();
            let _ = radio.StateChanged(&handler::<Radio, IInspectable>(move |_| {
                poster.post(Engine::radio_changed);
            }));
        }
        self.adapter_loaded(Some(LoadedAdapter { facts, radio }));
    }

    fn adapter_loaded(&mut self, loaded: Option<LoadedAdapter>) {
        let slot = loaded.map_or(AdapterSlot::Unloaded, AdapterSlot::Loaded);
        let waiters = match std::mem::replace(&mut self.adapter, slot) {
            AdapterSlot::Loading(waiters) => waiters,
            AdapterSlot::Unloaded | AdapterSlot::Loaded(_) => Vec::new(),
        };
        if matches!(self.adapter, AdapterSlot::Loaded(_)) {
            self.last_state = Some(self.adapter_state());
        }
        for waiter in waiters {
            waiter(self);
        }
    }

    fn radio_changed(&mut self) {
        if !matches!(self.adapter, AdapterSlot::Loaded(_)) {
            return;
        }
        let state = self.adapter_state();
        if self.last_state == Some(state) {
            return;
        }
        self.last_state = Some(state);
        self.adapter_changed(state);
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
