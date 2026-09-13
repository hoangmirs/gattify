use crate::{AdvertisingReport, BleError, BleResult, ErrorCode};

/// The local name bytes that fit in the service data of a 128-bit service
/// UUID: a legacy packet holds 31 bytes, and the section header and the UUID
/// take 18. Windows cannot advertise a local name of its own choosing, so the
/// name travels as service data, as on Android.
pub(super) const LOCAL_NAME_BUDGET: usize = 13;

/// A local name cut to the budget at a UTF-8 character boundary.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct LocalNameCut {
    pub(super) bytes: Vec<u8>,
    pub(super) truncated: bool,
}

impl LocalNameCut {
    pub(super) fn included(&self) -> bool {
        !self.bytes.is_empty()
    }
}

pub(super) fn cut_local_name(name: Option<&str>, budget: usize) -> LocalNameCut {
    let name = name.unwrap_or_default();
    let mut end = budget.min(name.len());
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    LocalNameCut {
        bytes: name.as_bytes()[..end].to_vec(),
        truncated: end < name.len(),
    }
}

/// The reply of an advertisement that started. Windows leaves the service
/// data out when it does not fit (`all_data` false): the name then counts as
/// cut, and a required name rejects with `payloadTooLarge`.
pub(super) fn advertising_report(
    cut: &LocalNameCut,
    all_data: bool,
    name_optional: bool,
) -> BleResult<AdvertisingReport> {
    let dropped = cut.included() && !all_data;
    if (cut.truncated || dropped) && !name_optional {
        return Err(BleError::new(
            ErrorCode::PayloadTooLarge,
            format!(
                "the local name does not fit in the advertisement: at most {LOCAL_NAME_BUDGET} bytes, and only when Windows has room for them"
            ),
        ));
    }
    Ok(AdvertisingReport {
        local_name_included: cut.included() && all_data,
        local_name_truncated: cut.truncated || dropped,
    })
}

/// How a service provider publishes its service. Windows adds a service to
/// its GATT database only while its provider publishes: `Discoverable` lets
/// connected centrals find the service without advertising it, `Advertised`
/// also sends a connectable advertisement. Each advertisement has its own
/// number, so new options restart the provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Publication {
    Discoverable,
    Advertised(u64),
}

/// The `AdvertisementStatus` of a provider, from its event or read from it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProviderStatus {
    /// The provider never published.
    Created,
    Started {
        all_data: bool,
    },
    Stopped,
    Aborted,
    Other,
}

/// The provider call a publisher asks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PublishStep {
    Start(Publication),
    Stop,
}

/// How a start ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Published {
    Started {
        publication: Publication,
        all_data: bool,
    },
    Ended(Publication),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Phase {
    #[default]
    Idle,
    Starting(Publication),
    Started(Publication),
    Stopping,
}

/// Moves one service provider towards the publication it should have. A
/// provider takes a new publication only after the previous one ended, so a
/// change waits for the start, then stops, then starts.
///
/// It follows the status read from the provider, not the one an event
/// carries: an event can tell of a change that a read has already seen.
#[derive(Debug, Default)]
pub(super) struct Publisher {
    phase: Phase,
    desired: Option<Publication>,
}

impl Publisher {
    pub(super) fn want(&mut self, desired: Option<Publication>) -> Option<PublishStep> {
        self.desired = desired;
        self.step()
    }

    /// Takes the status read from the provider after a call or an event,
    /// or `later` by the watchdog. A start can take a moment to show, so
    /// right after a call a status without a start means nothing. Later, a
    /// publication without an advertisement counts as started, because
    /// Windows may never report it, and an advertisement that shows no start
    /// ends once it is no longer wanted.
    pub(super) fn check(
        &mut self,
        status: ProviderStatus,
        later: bool,
    ) -> (Option<Published>, Option<PublishStep>) {
        // The provider shows no start.
        let quiet = matches!(status, ProviderStatus::Created | ProviderStatus::Stopped);
        let status = match self.phase {
            Phase::Starting(_) if quiet && !later => return (None, None),
            Phase::Starting(Publication::Discoverable) if quiet => {
                ProviderStatus::Started { all_data: true }
            }
            Phase::Starting(publication) if quiet && self.desired == Some(publication) => {
                return (None, None);
            }
            // It may show no start all along.
            Phase::Started(Publication::Discoverable) if quiet => return (None, None),
            _ if quiet => ProviderStatus::Stopped,
            _ => status,
        };
        self.observe(status)
    }

    fn observe(&mut self, status: ProviderStatus) -> (Option<Published>, Option<PublishStep>) {
        let published = match (self.phase, status) {
            (Phase::Starting(publication), ProviderStatus::Started { all_data }) => {
                self.phase = Phase::Started(publication);
                Some(Published::Started {
                    publication,
                    all_data,
                })
            }
            // A start that Windows refused, or a publication it ended on its
            // own, is not retried.
            (
                Phase::Starting(publication) | Phase::Started(publication),
                ProviderStatus::Stopped | ProviderStatus::Aborted,
            ) => {
                self.phase = Phase::Idle;
                if self.desired == Some(publication) {
                    self.desired = None;
                }
                Some(Published::Ended(publication))
            }
            (Phase::Stopping, ProviderStatus::Stopped | ProviderStatus::Aborted) => {
                self.phase = Phase::Idle;
                None
            }
            _ => None,
        };
        (published, self.step())
    }

    /// Whether a call waits for the provider to report its outcome.
    pub(super) fn is_settling(&self) -> bool {
        matches!(self.phase, Phase::Starting(_) | Phase::Stopping)
    }

    /// The provider refused a start at once.
    pub(super) fn start_failed(&mut self) {
        self.phase = Phase::Idle;
        self.desired = None;
    }

    /// The provider refused a stop at once, so it publishes nothing.
    pub(super) fn stop_failed(&mut self) -> Option<PublishStep> {
        self.phase = Phase::Idle;
        self.step()
    }

    fn step(&mut self) -> Option<PublishStep> {
        match self.phase {
            Phase::Idle => {
                let publication = self.desired?;
                self.phase = Phase::Starting(publication);
                Some(PublishStep::Start(publication))
            }
            Phase::Started(publication) if self.desired != Some(publication) => {
                self.phase = Phase::Stopping;
                Some(PublishStep::Stop)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_cut_at_a_character_boundary() {
        assert_eq!(
            cut_local_name(Some("Host A"), LOCAL_NAME_BUDGET),
            LocalNameCut {
                bytes: b"Host A".to_vec(),
                truncated: false
            }
        );
        let cut = cut_local_name(Some("Thirteen byte"), LOCAL_NAME_BUDGET);
        assert!(!cut.truncated);
        let cut = cut_local_name(Some("Fourteen bytes"), LOCAL_NAME_BUDGET);
        assert_eq!(cut.bytes, b"Fourteen byte");
        assert!(cut.truncated);
        // "é" takes two bytes and cannot be split.
        let cut = cut_local_name(Some("ééééééé"), LOCAL_NAME_BUDGET);
        assert_eq!(cut.bytes, "éééééé".as_bytes());
        assert!(cut.truncated);
        let cut = cut_local_name(Some("😀"), 3);
        assert!(!cut.included());
        assert!(cut.truncated);
    }

    #[test]
    fn a_missing_or_empty_name_is_neither_included_nor_cut() {
        for name in [None, Some("")] {
            let cut = cut_local_name(name, LOCAL_NAME_BUDGET);
            assert!(!cut.included());
            assert!(!cut.truncated);
            assert_eq!(
                advertising_report(&cut, false, false),
                Ok(AdvertisingReport {
                    local_name_included: false,
                    local_name_truncated: false
                })
            );
        }
    }

    #[test]
    fn a_name_left_out_by_windows_counts_as_cut() {
        let cut = cut_local_name(Some("Host A"), LOCAL_NAME_BUDGET);
        assert_eq!(
            advertising_report(&cut, true, false),
            Ok(AdvertisingReport {
                local_name_included: true,
                local_name_truncated: false
            })
        );
        assert_eq!(
            advertising_report(&cut, false, true),
            Ok(AdvertisingReport {
                local_name_included: false,
                local_name_truncated: true
            })
        );
        assert_eq!(
            advertising_report(&cut, false, false).unwrap_err().code,
            ErrorCode::PayloadTooLarge
        );
    }

    #[test]
    fn a_cut_name_rejects_unless_it_is_optional() {
        let cut = cut_local_name(Some("A much longer host name"), LOCAL_NAME_BUDGET);
        assert_eq!(
            advertising_report(&cut, true, false).unwrap_err().code,
            ErrorCode::PayloadTooLarge
        );
        assert_eq!(
            advertising_report(&cut, true, true),
            Ok(AdvertisingReport {
                local_name_included: true,
                local_name_truncated: true
            })
        );
    }

    #[test]
    fn a_fresh_provider_starts_at_once_and_reports_the_start() {
        let mut publisher = Publisher::default();
        let advertised = Publication::Advertised(1);
        assert_eq!(
            publisher.want(Some(advertised)),
            Some(PublishStep::Start(advertised))
        );
        assert_eq!(publisher.want(Some(advertised)), None);
        assert_eq!(
            publisher.observe(ProviderStatus::Started { all_data: false }),
            (
                Some(Published::Started {
                    publication: advertised,
                    all_data: false
                }),
                None
            )
        );
    }

    #[test]
    fn a_change_waits_for_the_start_then_stops_then_starts() {
        let mut publisher = Publisher::default();
        publisher.want(Some(Publication::Advertised(1)));
        assert_eq!(publisher.want(Some(Publication::Advertised(2))), None);
        let (outcome, step) = publisher.observe(ProviderStatus::Started { all_data: true });
        assert!(matches!(
            outcome,
            Some(Published::Started {
                publication: Publication::Advertised(1),
                ..
            })
        ));
        assert_eq!(step, Some(PublishStep::Stop));
        assert_eq!(publisher.observe(ProviderStatus::Other), (None, None));
        assert_eq!(
            publisher.observe(ProviderStatus::Stopped),
            (None, Some(PublishStep::Start(Publication::Advertised(2))))
        );
    }

    #[test]
    fn a_stopped_advertisement_stays_discoverable() {
        let mut publisher = Publisher::default();
        publisher.want(Some(Publication::Advertised(1)));
        publisher.observe(ProviderStatus::Started { all_data: true });
        assert_eq!(
            publisher.want(Some(Publication::Discoverable)),
            Some(PublishStep::Stop)
        );
        assert_eq!(
            publisher.observe(ProviderStatus::Stopped),
            (None, Some(PublishStep::Start(Publication::Discoverable)))
        );
        assert_eq!(publisher.want(None), None);
        let (_, step) = publisher.observe(ProviderStatus::Started { all_data: true });
        assert_eq!(step, Some(PublishStep::Stop));
    }

    #[test]
    fn an_aborted_start_ends_the_wish_without_a_retry() {
        let mut publisher = Publisher::default();
        publisher.want(Some(Publication::Advertised(3)));
        assert_eq!(
            publisher.observe(ProviderStatus::Aborted),
            (Some(Published::Ended(Publication::Advertised(3))), None)
        );
        assert_eq!(
            publisher.want(Some(Publication::Advertised(4))),
            Some(PublishStep::Start(Publication::Advertised(4)))
        );
    }

    #[test]
    fn a_publication_that_windows_ends_is_reported_and_not_restarted() {
        let mut publisher = Publisher::default();
        publisher.want(Some(Publication::Discoverable));
        publisher.observe(ProviderStatus::Started { all_data: true });
        assert_eq!(
            publisher.observe(ProviderStatus::Aborted),
            (Some(Published::Ended(Publication::Discoverable)), None)
        );
    }

    #[test]
    fn right_after_a_call_only_its_outcome_counts() {
        let mut publisher = Publisher::default();
        publisher.want(Some(Publication::Advertised(1)));
        assert!(publisher.is_settling());
        // The start can take a moment to show.
        assert_eq!(
            publisher.check(ProviderStatus::Created, false),
            (None, None)
        );
        assert_eq!(
            publisher.check(ProviderStatus::Started { all_data: true }, false),
            (
                Some(Published::Started {
                    publication: Publication::Advertised(1),
                    all_data: true
                }),
                None
            )
        );
        assert!(!publisher.is_settling());
        assert_eq!(
            publisher.want(Some(Publication::Discoverable)),
            Some(PublishStep::Stop)
        );
        assert_eq!(
            publisher.check(ProviderStatus::Started { all_data: true }, false),
            (None, None)
        );
        assert_eq!(
            publisher.check(ProviderStatus::Stopped, false),
            (None, Some(PublishStep::Start(Publication::Discoverable)))
        );
    }

    #[test]
    fn the_late_event_of_a_stop_read_already_does_not_end_the_next_start() {
        let mut publisher = Publisher::default();
        publisher.want(Some(Publication::Discoverable));
        publisher.check(ProviderStatus::Started { all_data: true }, false);
        publisher.want(Some(Publication::Advertised(2)));
        let (_, step) = publisher.check(ProviderStatus::Stopped, false);
        assert_eq!(step, Some(PublishStep::Start(Publication::Advertised(2))));
        // The event of the stop arrives, and the start does not show yet.
        assert_eq!(
            publisher.check(ProviderStatus::Stopped, false),
            (None, None)
        );
        assert!(publisher.is_settling());
        let (outcome, _) = publisher.check(ProviderStatus::Started { all_data: true }, false);
        assert!(matches!(outcome, Some(Published::Started { .. })));
    }

    #[test]
    fn a_silent_publication_without_an_advertisement_counts_as_started() {
        let mut publisher = Publisher::default();
        publisher.want(Some(Publication::Discoverable));
        assert_eq!(
            publisher.check(ProviderStatus::Stopped, true),
            (
                Some(Published::Started {
                    publication: Publication::Discoverable,
                    all_data: true
                }),
                None
            )
        );
        assert!(!publisher.is_settling());
        // A status without a start does not end it, an abort does.
        assert_eq!(
            publisher.check(ProviderStatus::Stopped, false),
            (None, None)
        );
        assert_eq!(
            publisher.check(ProviderStatus::Aborted, false),
            (Some(Published::Ended(Publication::Discoverable)), None)
        );
    }

    #[test]
    fn the_next_advertisement_is_not_stuck_behind_a_silent_publication() {
        let mut publisher = Publisher::default();
        publisher.want(Some(Publication::Discoverable));
        assert_eq!(publisher.want(Some(Publication::Advertised(2))), None);
        assert_eq!(
            publisher.check(ProviderStatus::Created, true).1,
            Some(PublishStep::Stop)
        );
        assert_eq!(
            publisher.check(ProviderStatus::Created, false),
            (None, Some(PublishStep::Start(Publication::Advertised(2))))
        );
    }

    #[test]
    fn a_silent_advertisement_waits_while_wanted_and_ends_when_not() {
        let mut publisher = Publisher::default();
        publisher.want(Some(Publication::Advertised(1)));
        assert_eq!(publisher.check(ProviderStatus::Stopped, true), (None, None));
        assert!(publisher.is_settling());
        assert_eq!(publisher.want(Some(Publication::Discoverable)), None);
        assert_eq!(
            publisher.check(ProviderStatus::Stopped, true),
            (
                Some(Published::Ended(Publication::Advertised(1))),
                Some(PublishStep::Start(Publication::Discoverable))
            )
        );
    }

    #[test]
    fn an_advertisement_that_shows_no_start_any_more_has_ended() {
        let mut publisher = Publisher::default();
        publisher.want(Some(Publication::Advertised(1)));
        publisher.check(ProviderStatus::Started { all_data: true }, false);
        assert_eq!(
            publisher.check(ProviderStatus::Stopped, false),
            (Some(Published::Ended(Publication::Advertised(1))), None)
        );
    }

    #[test]
    fn a_stop_that_still_shows_the_start_keeps_waiting() {
        let mut publisher = Publisher::default();
        publisher.want(Some(Publication::Discoverable));
        publisher.check(ProviderStatus::Started { all_data: true }, false);
        assert_eq!(publisher.want(None), Some(PublishStep::Stop));
        assert_eq!(
            publisher.check(ProviderStatus::Started { all_data: true }, true),
            (None, None)
        );
        assert!(publisher.is_settling());
        assert_eq!(publisher.check(ProviderStatus::Created, true), (None, None));
        assert!(!publisher.is_settling());
        assert_eq!(publisher.check(ProviderStatus::Stopped, true), (None, None));
    }

    #[test]
    fn a_refused_call_leaves_the_provider_idle() {
        let mut publisher = Publisher::default();
        publisher.want(Some(Publication::Advertised(1)));
        publisher.start_failed();
        assert_eq!(
            publisher.observe(ProviderStatus::Started { all_data: true }),
            (None, None)
        );

        publisher.want(Some(Publication::Discoverable));
        publisher.observe(ProviderStatus::Started { all_data: true });
        assert_eq!(
            publisher.want(Some(Publication::Advertised(2))),
            Some(PublishStep::Stop)
        );
        assert_eq!(
            publisher.stop_failed(),
            Some(PublishStep::Start(Publication::Advertised(2)))
        );
    }
}
