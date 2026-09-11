use std::{
    collections::{HashMap, VecDeque},
    mem::size_of,
    time::Instant,
};

use crate::{BleError, BleResult, ErrorCode};

use crate::peer::{Frame, FrameKind};

#[derive(Clone, Debug)]
pub struct ReceiverLimits {
    pub max_logical_size: usize,
    pub max_buffered_bytes: usize,
    pub max_partial_messages: usize,
    pub max_complete_messages: usize,
    pub max_complete_queue_bytes: usize,
    pub recent_message_ids: usize,
    pub reassembly_deadline_ms: u64,
}

impl Default for ReceiverLimits {
    fn default() -> Self {
        Self {
            max_logical_size: 16 * 1024,
            max_buffered_bytes: 1024 * 1024,
            max_partial_messages: 64,
            max_complete_messages: 64,
            max_complete_queue_bytes: 64 * 1024,
            recent_message_ids: 32,
            reassembly_deadline_ms: 30_000,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReceiveAction {
    None,
    Control {
        kind: FrameKind,
        payload: Vec<u8>,
    },
    Message {
        message_id: u32,
        payload: Vec<u8>,
        ack: Vec<u8>,
    },
    DuplicateAck {
        message_id: u32,
        ack: Vec<u8>,
    },
    Ack {
        message_id: u32,
    },
}

#[derive(Debug)]
struct Partial {
    total_length: usize,
    fragments: Vec<Option<Vec<u8>>>,
    received_bytes: usize,
    reserved_bytes: usize,
    started_ms: u64,
}

pub struct Receiver {
    limits: ReceiverLimits,
    partials: HashMap<(FrameKind, u32), Partial>,
    buffered_bytes: usize,
    completed: VecDeque<(u32, Vec<u8>)>,
    complete_queue_bytes: usize,
    recent: VecDeque<(u32, Vec<u8>)>,
    recent_bytes: usize,
    started_at: Instant,
}

impl Receiver {
    #[must_use]
    pub fn new(limits: ReceiverLimits) -> Self {
        Self {
            limits,
            partials: HashMap::new(),
            buffered_bytes: 0,
            completed: VecDeque::new(),
            complete_queue_bytes: 0,
            recent: VecDeque::new(),
            recent_bytes: 0,
            started_at: Instant::now(),
        }
    }

    /// Validates and admits one frame into the bounded receive state.
    ///
    /// # Errors
    ///
    /// Returns a structured protocol, payload-size, or queue-capacity error when
    /// the frame cannot be safely accepted.
    pub fn receive(&mut self, bytes: &[u8]) -> BleResult<ReceiveAction> {
        let elapsed = self.started_at.elapsed().as_millis();
        let now_ms = u64::try_from(elapsed).unwrap_or(u64::MAX);
        self.receive_at(bytes, now_ms)
    }

    /// Receives one frame using a caller-provided monotonic timestamp.
    ///
    /// This entry point makes expiry behavior deterministic in native adapters
    /// and tests.
    ///
    /// # Errors
    ///
    /// Returns a structured protocol, payload-size, or queue-capacity error when
    /// the frame cannot be safely accepted.
    // Keeping admission and ACK decisions in one transition prevents partial
    // state updates from escaping between reassembly and queue admission.
    #[allow(clippy::too_many_lines)]
    pub fn receive_at(&mut self, bytes: &[u8], now_ms: u64) -> BleResult<ReceiveAction> {
        self.expire_partials(now_ms);
        let frame = Frame::decode(bytes, self.limits.max_logical_size)?;
        if frame.kind == FrameKind::Ack {
            return Ok(ReceiveAction::Ack {
                message_id: frame.message_id,
            });
        }

        let key = (frame.kind, frame.message_id);
        let declared_total = frame.total_length as usize;
        if !self.partials.contains_key(&key) {
            if self.partials.len() >= self.limits.max_partial_messages {
                return Err(BleError::new(
                    ErrorCode::QueueFull,
                    "partial-message limit reached",
                ));
            }
            let reserved_bytes = usize::from(frame.fragment_count)
                .checked_mul(size_of::<Option<Vec<u8>>>())
                .and_then(|slots| {
                    slots
                        .checked_add(size_of::<Partial>())
                        .and_then(|bytes| bytes.checked_add(size_of::<(FrameKind, u32)>()))
                })
                .ok_or_else(|| {
                    BleError::new(
                        ErrorCode::PayloadTooLarge,
                        "fragment metadata size overflow",
                    )
                })?;
            if self
                .total_buffered_bytes()
                .checked_add(reserved_bytes)
                .is_none_or(|bytes| bytes > self.limits.max_buffered_bytes)
            {
                return Err(BleError::new(
                    ErrorCode::QueueFull,
                    "fragment metadata exceeds the protocol buffer budget",
                ));
            }
            self.buffered_bytes += reserved_bytes;
            self.partials.insert(
                key,
                Partial {
                    total_length: declared_total,
                    fragments: vec![None; usize::from(frame.fragment_count)],
                    received_bytes: 0,
                    reserved_bytes,
                    started_ms: now_ms,
                },
            );
        }
        let buffered_before_payload = self.total_buffered_bytes();
        let complete = {
            let Some(partial) = self.partials.get_mut(&key) else {
                return Err(BleError::new(
                    ErrorCode::Internal,
                    "partial reassembly state is missing",
                ));
            };
            if partial.total_length != declared_total
                || partial.fragments.len() != usize::from(frame.fragment_count)
            {
                return Err(BleError::new(
                    ErrorCode::ProtocolMismatch,
                    "fragment metadata changed during reassembly",
                ));
            }
            let slot = &mut partial.fragments[usize::from(frame.fragment_index)];
            if let Some(existing) = slot {
                if existing == &frame.payload {
                    return Ok(ReceiveAction::None);
                }
                return Err(BleError::new(
                    ErrorCode::ProtocolMismatch,
                    "duplicate fragment has different content",
                ));
            }
            if buffered_before_payload
                .checked_add(frame.payload.len())
                .is_none_or(|bytes| bytes > self.limits.max_buffered_bytes)
            {
                return Err(BleError::new(
                    ErrorCode::QueueFull,
                    "adapter protocol buffer budget exhausted",
                ));
            }
            partial.received_bytes += frame.payload.len();
            self.buffered_bytes += frame.payload.len();
            *slot = Some(frame.payload);
            !partial.fragments.iter().any(Option::is_none)
        };

        if !complete {
            return Ok(ReceiveAction::None);
        }
        let Some(partial) = self.partials.remove(&key) else {
            return Err(BleError::new(
                ErrorCode::ProtocolMismatch,
                "completed reassembly state is missing",
            ));
        };
        self.buffered_bytes = self
            .buffered_bytes
            .saturating_sub(partial.reserved_bytes + partial.received_bytes);
        if partial.received_bytes != partial.total_length {
            return Err(BleError::new(
                ErrorCode::ProtocolMismatch,
                "reassembled length does not match declared total",
            ));
        }
        let mut payload = Vec::with_capacity(partial.total_length);
        for fragment in &partial.fragments {
            let Some(fragment) = fragment else {
                return Err(BleError::new(
                    ErrorCode::ProtocolMismatch,
                    "completed reassembly contains a missing fragment",
                ));
            };
            payload.extend_from_slice(fragment);
        }
        if frame.kind != FrameKind::Data {
            return Ok(ReceiveAction::Control {
                kind: frame.kind,
                payload,
            });
        }

        if let Some((_, existing)) = self
            .recent
            .iter()
            .find(|(recent_id, _)| *recent_id == frame.message_id)
        {
            if existing != &payload {
                return Err(BleError::new(
                    ErrorCode::ProtocolMismatch,
                    "message ID was reused with different content",
                ));
            }
            return Ok(ReceiveAction::DuplicateAck {
                message_id: frame.message_id,
                ack: Frame::ack(frame.message_id).encode(self.limits.max_logical_size)?,
            });
        }
        if self.completed.len() >= self.limits.max_complete_messages
            || self.complete_queue_bytes + payload.len() > self.limits.max_complete_queue_bytes
        {
            return Err(BleError::new(
                ErrorCode::QueueFull,
                "complete-message queue cannot accept the message",
            ));
        }

        let evicted_recent_bytes = if self.recent.len() >= self.limits.recent_message_ids {
            self.recent.front().map_or(0, |(_, value)| value.len())
        } else {
            0
        };
        // An admitted payload is held twice: in the complete queue and in the
        // duplicate-detection cache.
        let admission_bytes = payload
            .len()
            .checked_mul(2)
            .and_then(|bytes| self.total_buffered_bytes().checked_add(bytes))
            .map(|bytes| bytes.saturating_sub(evicted_recent_bytes));
        if admission_bytes.is_none_or(|bytes| bytes > self.limits.max_buffered_bytes) {
            return Err(BleError::new(
                ErrorCode::QueueFull,
                "complete message exceeds the total protocol buffer budget",
            ));
        }

        self.complete_queue_bytes += payload.len();
        self.completed
            .push_back((frame.message_id, payload.clone()));
        self.recent_bytes += payload.len();
        self.recent.push_back((frame.message_id, payload.clone()));
        while self.recent.len() > self.limits.recent_message_ids {
            if let Some((_, evicted)) = self.recent.pop_front() {
                self.recent_bytes -= evicted.len();
            }
        }
        Ok(ReceiveAction::Message {
            message_id: frame.message_id,
            payload,
            ack: Frame::ack(frame.message_id).encode(self.limits.max_logical_size)?,
        })
    }

    pub fn pop_message(&mut self) -> Option<(u32, Vec<u8>)> {
        let message = self.completed.pop_front()?;
        self.complete_queue_bytes -= message.1.len();
        Some(message)
    }

    pub fn disconnect(&mut self) {
        self.partials.clear();
        self.buffered_bytes = 0;
    }

    fn expire_partials(&mut self, now_ms: u64) {
        let expired: Vec<_> = self
            .partials
            .iter()
            .filter_map(|(key, partial)| {
                (now_ms.saturating_sub(partial.started_ms) >= self.limits.reassembly_deadline_ms)
                    .then_some(*key)
            })
            .collect();
        for key in expired {
            if let Some(partial) = self.partials.remove(&key) {
                self.buffered_bytes = self
                    .buffered_bytes
                    .saturating_sub(partial.reserved_bytes + partial.received_bytes);
            }
        }
    }

    fn total_buffered_bytes(&self) -> usize {
        self.buffered_bytes
            .saturating_add(self.complete_queue_bytes)
            .saturating_add(self.recent_bytes)
    }
}

impl Default for Receiver {
    fn default() -> Self {
        Self::new(ReceiverLimits::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer::fragment;

    #[test]
    fn accepts_only_after_complete_reassembly() {
        let payload: Vec<u8> = (0..13).collect();
        let frames = fragment(FrameKind::Data, 41, &payload, 20, 16 * 1024).unwrap();
        let mut receiver = Receiver::default();
        assert_eq!(receiver.receive(&frames[1]).unwrap(), ReceiveAction::None);
        assert_eq!(receiver.receive(&frames[0]).unwrap(), ReceiveAction::None);
        let ReceiveAction::Message {
            message_id,
            payload: actual,
            ..
        } = receiver.receive(&frames[2]).unwrap()
        else {
            panic!("expected complete message")
        };
        assert_eq!(message_id, 41);
        assert_eq!(actual, payload);
        assert_eq!(receiver.pop_message(), Some((41, payload)));
    }

    #[test]
    fn queue_overflow_happens_before_ack() {
        let mut receiver = Receiver::new(ReceiverLimits {
            max_complete_queue_bytes: 2,
            ..ReceiverLimits::default()
        });
        let frame = fragment(FrameKind::Data, 1, b"abc", 20, 16 * 1024).unwrap();
        assert_eq!(
            receiver.receive(&frame[0]).unwrap_err().code,
            ErrorCode::QueueFull
        );
    }

    #[test]
    fn valid_duplicate_is_reacked_and_not_queued_twice() {
        let frame = fragment(FrameKind::Data, 7, b"ok", 20, 16 * 1024).unwrap();
        let mut receiver = Receiver::default();
        assert!(matches!(
            receiver.receive(&frame[0]).unwrap(),
            ReceiveAction::Message { .. }
        ));
        assert!(matches!(
            receiver.receive(&frame[0]).unwrap(),
            ReceiveAction::DuplicateAck { .. }
        ));
        assert_eq!(receiver.pop_message().unwrap().1, b"ok");
        assert!(receiver.pop_message().is_none());
    }

    #[test]
    fn control_reassembly_is_not_blocked_by_partial_data() {
        let data = fragment(FrameKind::Data, 9, b"split-data", 20, 16 * 1024).unwrap();
        let control = fragment(FrameKind::Hello, 0, b"hello", 20, 16 * 1024).unwrap();
        let mut receiver = Receiver::default();
        assert_eq!(receiver.receive(&data[0]).unwrap(), ReceiveAction::None);
        assert_eq!(
            receiver.receive(&control[0]).unwrap(),
            ReceiveAction::Control {
                kind: FrameKind::Hello,
                payload: b"hello".to_vec(),
            }
        );
    }

    #[test]
    fn metadata_allocation_respects_the_buffer_budget() {
        let frame = fragment(FrameKind::Data, 1, b"ab", 15, 16 * 1024).unwrap();
        let mut receiver = Receiver::new(ReceiverLimits {
            max_buffered_bytes: 0,
            ..ReceiverLimits::default()
        });
        assert_eq!(
            receiver.receive(&frame[0]).unwrap_err().code,
            ErrorCode::QueueFull
        );
    }

    #[test]
    fn expired_partial_releases_its_budget() {
        let frames = fragment(FrameKind::Data, 1, b"ab", 15, 16 * 1024).unwrap();
        let metadata_budget = 512;
        let mut receiver = Receiver::new(ReceiverLimits {
            max_buffered_bytes: metadata_budget,
            max_partial_messages: 1,
            reassembly_deadline_ms: 10,
            ..ReceiverLimits::default()
        });
        assert_eq!(
            receiver.receive_at(&frames[0], 0).unwrap(),
            ReceiveAction::None
        );

        let replacement = fragment(FrameKind::Data, 2, b"cd", 15, 16 * 1024).unwrap();
        assert_eq!(
            receiver.receive_at(&replacement[0], 10).unwrap(),
            ReceiveAction::None
        );
    }
}
