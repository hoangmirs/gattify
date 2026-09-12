/// Returns `value` as a lowercase 128-bit UUID with hyphens.
///
/// A 16-bit or 32-bit UUID is expanded with the Bluetooth base UUID. Hyphens in
/// the input are ignored. Returns `None` for anything else.
#[must_use]
pub fn normalize_uuid(value: &str) -> Option<String> {
    const BASE_TAIL: &str = "00001000800000805f9b34fb";

    let digits: String = value
        .chars()
        .filter(|character| *character != '-')
        .map(|character| character.to_ascii_lowercase())
        .collect();
    if !digits
        .chars()
        .all(|character| character.is_ascii_hexdigit())
    {
        return None;
    }
    let full = match digits.len() {
        4 => format!("0000{digits}{BASE_TAIL}"),
        8 => format!("{digits}{BASE_TAIL}"),
        32 => digits,
        _ => return None,
    };
    Some(format!(
        "{}-{}-{}-{}-{}",
        &full[..8],
        &full[8..12],
        &full[12..16],
        &full[16..20],
        &full[20..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_uuids_expand_with_the_bluetooth_base() {
        assert_eq!(
            normalize_uuid("180D").as_deref(),
            Some("0000180d-0000-1000-8000-00805f9b34fb")
        );
        assert_eq!(
            normalize_uuid("0000180d").as_deref(),
            Some("0000180d-0000-1000-8000-00805f9b34fb")
        );
    }

    #[test]
    fn long_uuids_are_lowercased_and_hyphenated() {
        assert_eq!(
            normalize_uuid("80FF87C38E844914AEDC0D6A3BA5534D").as_deref(),
            Some("80ff87c3-8e84-4914-aedc-0d6a3ba5534d")
        );
        assert_eq!(
            normalize_uuid("80ff87c3-8e84-4914-aedc-0d6a3ba5534d").as_deref(),
            Some("80ff87c3-8e84-4914-aedc-0d6a3ba5534d")
        );
    }

    #[test]
    fn other_values_are_rejected() {
        assert_eq!(normalize_uuid("xyz"), None);
        assert_eq!(normalize_uuid("180"), None);
        assert_eq!(normalize_uuid(""), None);
        assert_eq!(normalize_uuid("0000180g"), None);
    }
}
