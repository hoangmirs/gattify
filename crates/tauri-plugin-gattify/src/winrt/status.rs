use std::time::Duration;

use crate::{
    AdapterState, BleError, Capabilities, Command, ErrorCode, PermissionOutcome, PermissionState,
    Support,
};

/// The legacy advertising data length, which the service provider uses.
const LEGACY_ADVERTISING_DATA_LENGTH: u32 = 31;

// Parts of the ATT error table.
const ATT_INVALID_ATTRIBUTE_VALUE_LENGTH: u8 = 0x0D;

/// The state of the Bluetooth radio, from `Radio.State`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RadioPower {
    On,
    Off,
    /// Switched off by the firmware or a hardware switch.
    Disabled,
    Unknown,
}

/// What the default adapter supports.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct AdapterFacts {
    pub(super) low_energy: bool,
    pub(super) central: bool,
    pub(super) peripheral: bool,
    /// `MaxAdvertisementDataLength`, on Windows 10 2004 and later.
    pub(super) max_advertisement_data_length: Option<u32>,
}

/// A missing adapter or one without LE is `unavailable`. A disabled radio is
/// off: the adapter is there, and switching the radio on brings it back. An
/// LE adapter without a radio object counts as on: Windows gives none to a
/// process whose architecture differs from the system's, and commands then
/// report the error of the platform call instead.
pub(super) fn adapter_state(
    facts: Option<&AdapterFacts>,
    radio: Option<RadioPower>,
) -> AdapterState {
    match (facts, radio) {
        (None, _) => AdapterState::Unavailable,
        (Some(facts), _) if !facts.low_energy => AdapterState::Unavailable,
        (Some(_), Some(RadioPower::On) | None) => AdapterState::PoweredOn,
        (Some(_), Some(RadioPower::Off | RadioPower::Disabled)) => AdapterState::PoweredOff,
        (Some(_), Some(RadioPower::Unknown)) => AdapterState::Unknown,
    }
}

/// The rejection for a command that needs the adapter on, or `None` when it is on.
pub(super) fn readiness_error(state: AdapterState) -> Option<BleError> {
    let (code, message) = match state {
        AdapterState::PoweredOn => return None,
        AdapterState::PoweredOff => (ErrorCode::BluetoothOff, "Bluetooth is off"),
        AdapterState::Unauthorized => {
            (ErrorCode::PermissionDenied, "the app may not use Bluetooth")
        }
        AdapterState::Unavailable => (
            ErrorCode::Unavailable,
            "this computer has no Bluetooth LE adapter",
        ),
        AdapterState::Resetting | AdapterState::Unknown => {
            (ErrorCode::Unavailable, "the Bluetooth adapter is not ready")
        }
    };
    Some(BleError::new(code, message))
}

pub(super) fn capabilities(facts: Option<&AdapterFacts>) -> Capabilities {
    let background = Support::unsupported("foregroundOnlyContract");
    let Some(facts) = facts.filter(|facts| facts.low_energy) else {
        let reason = if facts.is_none() {
            "noAdapter"
        } else {
            "noLowEnergy"
        };
        let unsupported = || Support::unsupported(reason);
        return Capabilities {
            central: unsupported(),
            peripheral: unsupported(),
            advertising: unsupported(),
            targeted_notify: unsupported(),
            simultaneous_roles: unsupported(),
            background,
            max_connections: None,
            max_advertising_data_length: None,
        };
    };
    let role = |supported: bool, reason: &str| {
        if supported {
            Support::supported()
        } else {
            Support::unsupported(reason)
        }
    };
    let peripheral = role(facts.peripheral, "noPeripheralRole");
    Capabilities {
        central: role(facts.central, "noCentralRole"),
        advertising: peripheral.clone(),
        targeted_notify: peripheral.clone(),
        simultaneous_roles: if facts.central {
            peripheral.clone()
        } else {
            Support::unsupported("noCentralRole")
        },
        peripheral,
        background,
        max_connections: None,
        max_advertising_data_length: Some(
            facts
                .max_advertisement_data_length
                .filter(|length| *length > 0)
                .map_or(LEGACY_ADVERTISING_DATA_LENGTH, |length| {
                    length.min(LEGACY_ADVERTISING_DATA_LENGTH)
                }),
        ),
    }
}

/// An unpackaged desktop app has no runtime Bluetooth permission, and a
/// packaged app declares the `bluetooth` capability in its manifest: neither
/// has anything to ask the user.
pub(super) fn permissions() -> PermissionState {
    PermissionState {
        scan: PermissionOutcome::NotRequired,
        connect: PermissionOutcome::NotRequired,
        advertise: PermissionOutcome::NotRequired,
    }
}

/// The deadline of a command: the requested one, else the contract default, else none.
pub(super) fn deadline_for(command: &Command, requested: Option<u64>) -> Option<Duration> {
    let millis = requested.or(match command {
        Command::Connect { options, .. } => Some(options.timeout_ms.unwrap_or(15_000)),
        Command::DiscoverServices { .. } => Some(10_000),
        Command::Read { .. }
        | Command::Write { .. }
        | Command::Subscribe { .. }
        | Command::Unsubscribe { .. }
        | Command::Notify { .. }
        | Command::CreateServer(_)
        | Command::StartAdvertising { .. } => Some(5_000),
        _ => None,
    })?;
    Some(Duration::from_millis(millis))
}

fn platform_error(code: ErrorCode, message: String, native_code: String) -> BleError {
    let mut error = BleError::new(code, message);
    error.native_code = Some(native_code);
    error
}

/// Maps a `BluetoothError` other than `Success`, keeping `bluetoothError<n>`.
pub(super) fn bluetooth_error(value: i32, action: &str) -> BleError {
    let (code, reason) = match value {
        1 => (
            ErrorCode::BluetoothOff,
            "the Bluetooth radio is not available",
        ),
        2 => (ErrorCode::Busy, "the resource is in use"),
        3 => (ErrorCode::Disconnected, "the device is not connected"),
        5 => (ErrorCode::Unsupported, "a policy disables it"),
        6 | 9 => (ErrorCode::Unsupported, "the adapter does not support it"),
        7 | 8 => (ErrorCode::PermissionDenied, "the user did not allow it"),
        _ => (ErrorCode::Internal, "Windows reported an error"),
    };
    platform_error(
        code,
        format!("{action} failed: {reason}"),
        format!("bluetoothError{value}"),
    )
}

/// Maps an ATT error, keeping `gattProtocolError<n>`.
fn protocol_error(value: u8, action: &str) -> BleError {
    let code = if value == ATT_INVALID_ATTRIBUTE_VALUE_LENGTH {
        ErrorCode::PayloadTooLarge
    } else {
        ErrorCode::Internal
    };
    platform_error(
        code,
        format!("{action} failed with ATT error 0x{value:02x}"),
        format!("gattProtocolError{value}"),
    )
}

/// Maps a `GattCommunicationStatus` other than `Success`, keeping
/// `gattCommunicationStatus<n>`, or the ATT error of a `ProtocolError`.
pub(super) fn communication_error(status: i32, att_error: Option<u8>, action: &str) -> BleError {
    let (code, reason) = match (status, att_error) {
        (2, Some(value)) => return protocol_error(value, action),
        (1, _) => (ErrorCode::Disconnected, "the device is unreachable"),
        (3, _) => (ErrorCode::PermissionDenied, "access is denied"),
        _ => (ErrorCode::Internal, "the GATT request failed"),
    };
    platform_error(
        code,
        format!("{action} failed: {reason}"),
        format!("gattCommunicationStatus{status}"),
    )
}

/// Maps a failed `WinRT` call, keeping `hresult0x<8 hex digits>`.
pub(super) fn hresult_error(value: i32, message: &str, action: &str) -> BleError {
    let bits = value.cast_unsigned();
    let code = match bits {
        // E_ACCESSDENIED
        0x8007_0005 => ErrorCode::PermissionDenied,
        // ERROR_DEVICE_NOT_CONNECTED
        0x8007_048F => ErrorCode::Disconnected,
        // ERROR_DEVICE_NOT_AVAILABLE
        0x8007_10DF => ErrorCode::Unavailable,
        // E_NOTIMPL, E_NOINTERFACE (an API this Windows build lacks), ERROR_NOT_SUPPORTED
        0x8000_4001 | 0x8000_4002 | 0x8007_0032 => ErrorCode::Unsupported,
        // E_BLUETOOTH_ATT_INVALID_ATTRIBUTE_VALUE_LENGTH
        0x8065_000D => ErrorCode::PayloadTooLarge,
        _ => ErrorCode::Internal,
    };
    let message = message.trim();
    let message = if message.is_empty() {
        format!("{action} failed")
    } else {
        format!("{action} failed: {message}")
    };
    platform_error(code, message, format!("hresult0x{bits:08x}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConnectOptions, ConnectionId, DeviceId, SupportLevel};

    fn facts(central: bool, peripheral: bool) -> AdapterFacts {
        AdapterFacts {
            low_energy: true,
            central,
            peripheral,
            max_advertisement_data_length: None,
        }
    }

    #[test]
    fn the_adapter_state_follows_the_adapter_then_the_radio() {
        let le = facts(true, true);
        assert_eq!(adapter_state(None, None), AdapterState::Unavailable);
        let classic_only = AdapterFacts {
            low_energy: false,
            ..le
        };
        assert_eq!(
            adapter_state(Some(&classic_only), Some(RadioPower::On)),
            AdapterState::Unavailable
        );
        assert_eq!(
            adapter_state(Some(&le), Some(RadioPower::On)),
            AdapterState::PoweredOn
        );
        assert_eq!(
            adapter_state(Some(&le), Some(RadioPower::Off)),
            AdapterState::PoweredOff
        );
        assert_eq!(
            adapter_state(Some(&le), Some(RadioPower::Disabled)),
            AdapterState::PoweredOff
        );
        assert_eq!(
            adapter_state(Some(&le), Some(RadioPower::Unknown)),
            AdapterState::Unknown
        );
    }

    #[test]
    fn an_le_adapter_without_a_radio_counts_as_on() {
        let le = facts(true, true);
        assert_eq!(adapter_state(Some(&le), None), AdapterState::PoweredOn);
        assert_eq!(readiness_error(adapter_state(Some(&le), None)), None);
        let classic_only = AdapterFacts {
            low_energy: false,
            ..le
        };
        assert_eq!(
            adapter_state(Some(&classic_only), None),
            AdapterState::Unavailable
        );
    }

    #[test]
    fn commands_that_need_the_radio_reject_with_the_code_of_the_state() {
        let code = |state| readiness_error(state).map(|error| error.code);
        assert_eq!(code(AdapterState::PoweredOn), None);
        assert_eq!(
            code(AdapterState::PoweredOff),
            Some(ErrorCode::BluetoothOff)
        );
        assert_eq!(
            code(AdapterState::Unavailable),
            Some(ErrorCode::Unavailable)
        );
        assert_eq!(code(AdapterState::Unknown), Some(ErrorCode::Unavailable));
        assert_eq!(code(AdapterState::Resetting), Some(ErrorCode::Unavailable));
        assert_eq!(
            code(AdapterState::Unauthorized),
            Some(ErrorCode::PermissionDenied)
        );
    }

    #[test]
    fn capabilities_follow_the_roles_of_the_adapter() {
        let both = capabilities(Some(&facts(true, true)));
        assert_eq!(both.central, Support::supported());
        assert_eq!(both.peripheral, Support::supported());
        assert_eq!(both.advertising, Support::supported());
        assert_eq!(both.targeted_notify, Support::supported());
        assert_eq!(both.simultaneous_roles, Support::supported());
        assert_eq!(
            both.background,
            Support::unsupported("foregroundOnlyContract")
        );
        assert_eq!(both.max_connections, None);
        assert_eq!(both.max_advertising_data_length, Some(31));

        let central_only = capabilities(Some(&facts(true, false)));
        assert_eq!(central_only.central.level, SupportLevel::Supported);
        for support in [
            &central_only.peripheral,
            &central_only.advertising,
            &central_only.targeted_notify,
            &central_only.simultaneous_roles,
        ] {
            assert_eq!(support, &Support::unsupported("noPeripheralRole"));
        }
        let peripheral_only = capabilities(Some(&facts(false, true)));
        assert_eq!(
            peripheral_only.simultaneous_roles,
            Support::unsupported("noCentralRole")
        );
    }

    #[test]
    fn the_advertising_data_length_is_the_legacy_length_at_most() {
        let with_length = |length| {
            capabilities(Some(&AdapterFacts {
                max_advertisement_data_length: Some(length),
                ..facts(true, true)
            }))
            .max_advertising_data_length
        };
        assert_eq!(with_length(1650), Some(31));
        assert_eq!(with_length(24), Some(24));
        assert_eq!(with_length(0), Some(31));
    }

    #[test]
    fn without_an_le_adapter_every_role_is_unsupported() {
        for (adapter, reason) in [
            (None, "noAdapter"),
            (
                Some(AdapterFacts {
                    low_energy: false,
                    ..facts(true, true)
                }),
                "noLowEnergy",
            ),
        ] {
            let capabilities = capabilities(adapter.as_ref());
            assert_eq!(capabilities.central, Support::unsupported(reason));
            assert_eq!(
                capabilities.simultaneous_roles,
                Support::unsupported(reason)
            );
            assert_eq!(
                capabilities.background,
                Support::unsupported("foregroundOnlyContract")
            );
            assert_eq!(capabilities.max_advertising_data_length, None);
        }
    }

    #[test]
    fn no_role_needs_a_runtime_permission() {
        assert_eq!(
            permissions(),
            PermissionState {
                scan: PermissionOutcome::NotRequired,
                connect: PermissionOutcome::NotRequired,
                advertise: PermissionOutcome::NotRequired,
            }
        );
    }

    #[test]
    fn deadlines_default_per_command() {
        let connect = |timeout_ms| Command::Connect {
            device_id: DeviceId::new("device-1"),
            options: ConnectOptions { timeout_ms },
        };
        let discover = Command::DiscoverServices {
            connection_id: ConnectionId::new("connection-1"),
        };
        assert_eq!(
            deadline_for(&connect(None), None),
            Some(Duration::from_secs(15))
        );
        assert_eq!(
            deadline_for(&connect(Some(3_000)), None),
            Some(Duration::from_secs(3))
        );
        assert_eq!(
            deadline_for(&connect(Some(3_000)), Some(700)),
            Some(Duration::from_millis(700))
        );
        assert_eq!(deadline_for(&discover, None), Some(Duration::from_secs(10)));
        assert_eq!(deadline_for(&Command::CloseOwner, None), None);
        assert_eq!(
            deadline_for(&Command::GetState, Some(50)),
            Some(Duration::from_millis(50))
        );
    }

    #[test]
    fn platform_errors_keep_their_native_code() {
        let busy = bluetooth_error(2, "creating the service");
        assert_eq!(busy.code, ErrorCode::Busy);
        assert_eq!(busy.native_code.as_deref(), Some("bluetoothError2"));
        assert_eq!(bluetooth_error(1, "x").code, ErrorCode::BluetoothOff);
        assert_eq!(bluetooth_error(7, "x").code, ErrorCode::PermissionDenied);
        assert_eq!(bluetooth_error(4, "x").code, ErrorCode::Internal);

        let unreachable = communication_error(1, None, "the read");
        assert_eq!(unreachable.code, ErrorCode::Disconnected);
        assert_eq!(
            unreachable.native_code.as_deref(),
            Some("gattCommunicationStatus1")
        );
        assert_eq!(
            communication_error(3, None, "x").code,
            ErrorCode::PermissionDenied
        );
        assert_eq!(
            communication_error(2, None, "x").native_code.as_deref(),
            Some("gattCommunicationStatus2")
        );

        let too_long = communication_error(2, Some(0x0D), "the write");
        assert_eq!(too_long.code, ErrorCode::PayloadTooLarge);
        assert_eq!(too_long.native_code.as_deref(), Some("gattProtocolError13"));
        let not_permitted = protocol_error(0x03, "the write");
        assert_eq!(not_permitted.code, ErrorCode::Internal);
        assert_eq!(
            not_permitted.native_code.as_deref(),
            Some("gattProtocolError3")
        );
    }

    #[test]
    fn hresults_map_to_contract_codes_and_keep_their_hex_form() {
        let denied = hresult_error(
            0x8007_0005_u32.cast_signed(),
            "Access is denied.\r\n",
            "the connect",
        );
        assert_eq!(denied.code, ErrorCode::PermissionDenied);
        assert_eq!(denied.native_code.as_deref(), Some("hresult0x80070005"));
        assert_eq!(denied.message, "the connect failed: Access is denied.");
        let missing = hresult_error(
            0x8000_4002_u32.cast_signed(),
            "",
            "setting the service data",
        );
        assert_eq!(missing.code, ErrorCode::Unsupported);
        assert_eq!(missing.message, "setting the service data failed");
        assert_eq!(
            hresult_error(0x8065_000D_u32.cast_signed(), "", "x").code,
            ErrorCode::PayloadTooLarge
        );
        assert_eq!(
            hresult_error(0x8007_048F_u32.cast_signed(), "", "x").code,
            ErrorCode::Disconnected
        );
        let other = hresult_error(0x8000_000E_u32.cast_signed(), "", "x");
        assert_eq!(other.code, ErrorCode::Internal);
        assert_eq!(other.native_code.as_deref(), Some("hresult0x8000000e"));
    }
}
