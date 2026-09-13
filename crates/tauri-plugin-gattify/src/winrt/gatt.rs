use std::collections::BTreeMap;

use crate::{
    normalize_uuid, BleError, BleResult, CharacteristicProperties, ErrorCode, LinkLimits, WriteType,
};

/// The ATT MTU of a link before its MTU exchange.
pub(super) const DEFAULT_ATT_MTU: u16 = 23;
/// The longest value one attribute holds.
pub(super) const MAX_ATTRIBUTE_LENGTH: u32 = 512;

/// The ATT errors a local server answers with.
pub(super) mod att {
    pub(in crate::winrt) const READ_NOT_PERMITTED: u8 = 0x02;
    pub(in crate::winrt) const WRITE_NOT_PERMITTED: u8 = 0x03;
    pub(in crate::winrt) const INVALID_OFFSET: u8 = 0x07;
    pub(in crate::winrt) const ATTRIBUTE_NOT_FOUND: u8 = 0x0A;
    pub(in crate::winrt) const INVALID_ATTRIBUTE_VALUE_LENGTH: u8 = 0x0D;
    pub(in crate::winrt) const UNLIKELY_ERROR: u8 = 0x0E;
}

// GattCharacteristicProperties bits, the same as the ATT property bits.
const PROPERTY_READ: u32 = 0x02;
const PROPERTY_WRITE_WITHOUT_RESPONSE: u32 = 0x04;
const PROPERTY_WRITE: u32 = 0x08;
const PROPERTY_NOTIFY: u32 = 0x10;
const PROPERTY_INDICATE: u32 = 0x20;

/// A 128-bit UUID as a lowercase string with hyphens.
pub(super) fn uuid_from_u128(value: u128) -> String {
    let hex = format!("{value:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// The 128-bit value of a UUID string, short forms included.
pub(super) fn uuid_to_u128(uuid: &str) -> Option<u128> {
    u128::from_str_radix(&normalize_uuid(uuid)?.replace('-', ""), 16).ok()
}

/// The largest value one ATT write or notification carries at `mtu`: the MTU
/// minus 3, at most 512.
pub(super) fn value_length(mtu: u16) -> u32 {
    (u32::from(mtu.max(DEFAULT_ATT_MTU)) - 3).min(MAX_ATTRIBUTE_LENGTH)
}

pub(super) fn link_limits(mtu: u16) -> LinkLimits {
    let length = value_length(mtu);
    LinkLimits {
        write_with_response: Some(length),
        write_without_response: Some(length),
        notification: Some(length),
        att_mtu: Some(u32::from(mtu.max(DEFAULT_ATT_MTU))),
    }
}

/// The notification size a server reports for a subscribed client.
pub(super) fn notification_length(max_notification_size: u16) -> u32 {
    u32::from(max_notification_size).min(MAX_ATTRIBUTE_LENGTH)
}

pub(super) fn properties_from_bits(bits: u32) -> CharacteristicProperties {
    CharacteristicProperties {
        read: bits & PROPERTY_READ != 0,
        write: bits & PROPERTY_WRITE != 0,
        write_without_response: bits & PROPERTY_WRITE_WITHOUT_RESPONSE != 0,
        notify: bits & PROPERTY_NOTIFY != 0,
        indicate: bits & PROPERTY_INDICATE != 0,
    }
}

pub(super) fn properties_to_bits(properties: &CharacteristicProperties) -> u32 {
    [
        (properties.read, PROPERTY_READ),
        (properties.write, PROPERTY_WRITE),
        (
            properties.write_without_response,
            PROPERTY_WRITE_WITHOUT_RESPONSE,
        ),
        (properties.notify, PROPERTY_NOTIFY),
        (properties.indicate, PROPERTY_INDICATE),
    ]
    .into_iter()
    .filter(|(set, _)| *set)
    .fold(0, |bits, (_, bit)| bits | bit)
}

/// What a subscription enables in the client configuration descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Mode {
    Notify,
    Indicate,
}

/// What a client does with a remote characteristic.
#[derive(Clone, Copy, Debug)]
pub(super) enum Access {
    Read,
    Write(WriteType),
    Subscribe,
}

/// Rejects with `unsupported` when the characteristic lacks the property of
/// `access`. A subscription uses indications only when the characteristic has
/// no notify.
pub(super) fn check_access(
    properties: &CharacteristicProperties,
    access: Access,
) -> BleResult<Option<Mode>> {
    let (permitted, mode, lacking) = match access {
        Access::Read => (properties.read, None, "read"),
        Access::Write(WriteType::WithResponse) => (properties.write, None, "write"),
        Access::Write(WriteType::WithoutResponse) => (
            properties.write_without_response,
            None,
            "writeWithoutResponse",
        ),
        Access::Subscribe if properties.notify => (true, Some(Mode::Notify), ""),
        Access::Subscribe => (
            properties.indicate,
            Some(Mode::Indicate),
            "notify or indicate",
        ),
    };
    if permitted {
        Ok(mode)
    } else {
        Err(BleError::unsupported(format!(
            "the characteristic does not have {lacking}"
        )))
    }
}

/// Rejects a client write that one ATT request cannot carry. A write with
/// response longer than the link allows goes out as a long write.
pub(super) fn check_write_length(length: usize, write_type: WriteType, mtu: u16) -> BleResult<()> {
    let limit = match write_type {
        WriteType::WithResponse => MAX_ATTRIBUTE_LENGTH,
        WriteType::WithoutResponse => value_length(mtu),
    };
    if length > limit as usize {
        return Err(BleError::new(
            ErrorCode::PayloadTooLarge,
            format!("this write carries at most {limit} bytes"),
        ));
    }
    Ok(())
}

/// The part of a stored value that a read at `offset` returns, or the ATT error.
pub(super) fn read_answer(readable: bool, value: &[u8], offset: usize) -> Result<&[u8], u8> {
    if !readable {
        return Err(att::READ_NOT_PERMITTED);
    }
    value.get(offset..).ok_or(att::INVALID_OFFSET)
}

/// The ATT error of a server write, or `None` when it is valid. Windows
/// raises one request per write, so a write is one part: it starts at offset
/// 0 and fits `max_value_length`.
pub(super) fn write_error(
    properties: &CharacteristicProperties,
    max_value_length: u32,
    offset: usize,
    length: usize,
) -> Option<u8> {
    if !properties.write && !properties.write_without_response {
        Some(att::WRITE_NOT_PERMITTED)
    } else if offset != 0 {
        Some(att::INVALID_OFFSET)
    } else if length > max_value_length as usize {
        Some(att::INVALID_ATTRIBUTE_VALUE_LENGTH)
    } else {
        None
    }
}

/// A change in the subscribers of one local characteristic.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum SubscriberChange {
    Subscribed(String, u32),
    Resized(String, u32),
    Unsubscribed(String),
}

/// Diffs two subscriber lists, each a remote device ID with its notification length.
pub(super) fn subscriber_changes(
    before: &BTreeMap<String, u32>,
    after: &BTreeMap<String, u32>,
) -> Vec<SubscriberChange> {
    let gone = before
        .keys()
        .filter(|device| !after.contains_key(*device))
        .map(|device| SubscriberChange::Unsubscribed(device.clone()));
    let present = after
        .iter()
        .filter_map(|(device, length)| match before.get(device) {
            None => Some(SubscriberChange::Subscribed(device.clone(), *length)),
            Some(previous) if previous != length => {
                Some(SubscriberChange::Resized(device.clone(), *length))
            }
            Some(_) => None,
        });
    gone.chain(present).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Properties from letters: `r`ead, `w`rite, write `c`ommand, `n`otify, `i`ndicate.
    fn properties(flags: &str) -> CharacteristicProperties {
        CharacteristicProperties {
            read: flags.contains('r'),
            write: flags.contains('w'),
            write_without_response: flags.contains('c'),
            notify: flags.contains('n'),
            indicate: flags.contains('i'),
        }
    }

    #[test]
    fn uuids_round_trip_through_their_128_bit_value() {
        let value = uuid_to_u128("80FF87C3-8E84-4914-AEDC-0D6A3BA5534D").unwrap();
        assert_eq!(value, 0x80ff_87c3_8e84_4914_aedc_0d6a_3ba5_534d);
        assert_eq!(
            uuid_from_u128(value),
            "80ff87c3-8e84-4914-aedc-0d6a3ba5534d"
        );
        assert_eq!(
            uuid_from_u128(uuid_to_u128("180d").unwrap()),
            "0000180d-0000-1000-8000-00805f9b34fb"
        );
        assert_eq!(uuid_to_u128("not a uuid"), None);
    }

    #[test]
    fn link_limits_follow_the_att_mtu_and_stop_at_512() {
        assert_eq!(
            link_limits(185),
            LinkLimits {
                write_with_response: Some(182),
                write_without_response: Some(182),
                notification: Some(182),
                att_mtu: Some(185),
            }
        );
        assert_eq!(value_length(517), 512);
        assert_eq!(value_length(525), 512);
        assert_eq!(value_length(23), 20);
        assert_eq!(value_length(0), 20);
        assert_eq!(link_limits(525).att_mtu, Some(525));
        assert_eq!(notification_length(522), 512);
        assert_eq!(notification_length(20), 20);
    }

    #[test]
    fn property_bits_map_both_ways() {
        let all = properties("rwcni");
        assert_eq!(properties_to_bits(&all), 0x3E);
        assert_eq!(properties_from_bits(0x3E), all);
        // Broadcast and extended properties have no contract field.
        assert_eq!(properties_from_bits(0x01 | 0x80 | 0x02), properties("r"));
        assert_eq!(properties_to_bits(&CharacteristicProperties::default()), 0);
    }

    #[test]
    fn a_client_needs_the_matching_property() {
        let read_only = properties("r");
        assert!(check_access(&read_only, Access::Read).is_ok());
        let error = check_access(&read_only, Access::Write(WriteType::WithResponse)).unwrap_err();
        assert_eq!(error.code, ErrorCode::Unsupported);
        let commands = properties("c");
        assert!(check_access(&commands, Access::Write(WriteType::WithoutResponse)).is_ok());
        assert!(check_access(&commands, Access::Write(WriteType::WithResponse)).is_err());
        assert!(check_access(&read_only, Access::Subscribe).is_err());
    }

    #[test]
    fn a_subscription_indicates_only_without_notify() {
        let both = properties("ni");
        let indicate = properties("i");
        assert_eq!(
            check_access(&both, Access::Subscribe).unwrap(),
            Some(Mode::Notify)
        );
        assert_eq!(
            check_access(&indicate, Access::Subscribe).unwrap(),
            Some(Mode::Indicate)
        );
    }

    #[test]
    fn write_lengths_follow_the_write_type() {
        assert!(check_write_length(512, WriteType::WithResponse, 23).is_ok());
        assert_eq!(
            check_write_length(513, WriteType::WithResponse, 517)
                .unwrap_err()
                .code,
            ErrorCode::PayloadTooLarge
        );
        assert!(check_write_length(182, WriteType::WithoutResponse, 185).is_ok());
        assert!(check_write_length(183, WriteType::WithoutResponse, 185).is_err());
    }

    #[test]
    fn a_read_returns_the_value_from_its_offset() {
        assert_eq!(read_answer(true, b"hello", 0), Ok(&b"hello"[..]));
        assert_eq!(read_answer(true, b"hello", 3), Ok(&b"lo"[..]));
        assert_eq!(read_answer(true, b"hello", 5), Ok(&b""[..]));
        assert_eq!(read_answer(true, b"hello", 6), Err(att::INVALID_OFFSET));
        assert_eq!(
            read_answer(false, b"hello", 0),
            Err(att::READ_NOT_PERMITTED)
        );
    }

    #[test]
    fn a_server_write_must_be_permitted_start_at_zero_and_fit() {
        let writable = properties("w");
        let commands = properties("c");
        let read_only = properties("r");
        assert_eq!(write_error(&writable, 4, 0, 4), None);
        assert_eq!(write_error(&commands, 4, 0, 1), None);
        assert_eq!(
            write_error(&read_only, 4, 0, 1),
            Some(att::WRITE_NOT_PERMITTED)
        );
        assert_eq!(write_error(&writable, 4, 2, 1), Some(att::INVALID_OFFSET));
        assert_eq!(
            write_error(&writable, 4, 0, 5),
            Some(att::INVALID_ATTRIBUTE_VALUE_LENGTH)
        );
        assert_eq!(att::ATTRIBUTE_NOT_FOUND, 0x0A);
        assert_eq!(att::UNLIKELY_ERROR, 0x0E);
    }

    #[test]
    fn subscriber_lists_diff_into_subscribes_resizes_and_unsubscribes() {
        let before: BTreeMap<String, u32> = [("a".to_owned(), 20), ("b".to_owned(), 20)]
            .into_iter()
            .collect();
        let after: BTreeMap<String, u32> = [("b".to_owned(), 182), ("c".to_owned(), 20)]
            .into_iter()
            .collect();
        assert_eq!(
            subscriber_changes(&before, &after),
            vec![
                SubscriberChange::Unsubscribed("a".into()),
                SubscriberChange::Resized("b".into(), 182),
                SubscriberChange::Subscribed("c".into(), 20),
            ]
        );
        assert!(subscriber_changes(&after, &after).is_empty());
    }
}
