use std::collections::VecDeque;

use ble_core::{BleError, BleResult, DeliveryOutcome, ErrorCode};

use crate::{fragment, FrameKind};

#[derive(Clone, Debug)]
pub struct SendLimits {
    pub max_logical_size: usize,
    pub max_outbound_queue_bytes: usize,
    pub value_limit: usize,
    pub ack_deadline_ms: u64,
    pub absolute_deadline_ms: u64,
    pub retransmissions: u8,
}

impl Default for SendLimits {
    fn default() -> Self {
        Self {
            max_logical_size: 16 * 1024,
            max_outbound_queue_bytes: 64 * 1024,
            value_limit: 20,
            ack_deadline_ms: 5_000,
            absolute_deadline_ms: 30_000,
            retransmissions: 2,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueuedMessage {
    pub message_id: u32,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SendAction {
    Idle,
    Submit {
        message_id: u32,
        frames: Vec<Vec<u8>>,
        retransmission: bool,
    },
    Acknowledged {
        message_id: u32,
    },
    Failed {
        message_id: u32,
        error: BleError,
    },
}

#[derive(Clone, Debug)]
struct Pending {
    message: QueuedMessage,
    frames: Vec<Vec<u8>>,
    first_submitted_ms: u64,
    last_submitted_ms: u64,
    retransmissions: u8,
}

pub struct Sender {
    limits: SendLimits,
    next_message_id: u32,
    queue: VecDeque<QueuedMessage>,
    queue_bytes: usize,
    pending: Option<Pending>,
}

impl Sender {
    #[must_use]
    pub fn new(limits: SendLimits) -> Self {
        Self {
            limits,
            next_message_id: 1,
            queue: VecDeque::new(),
            queue_bytes: 0,
            pending: None,
        }
    }

    /// Adds one logical payload to the bounded outbound queue.
    ///
    /// # Errors
    ///
    /// Returns a payload-size, queue-capacity, or exhausted-ID error when the
    /// payload cannot be admitted.
    pub fn enqueue(&mut self, payload: Vec<u8>) -> BleResult<u32> {
        if payload.len() > self.limits.max_logical_size {
            return Err(BleError::new(
                ErrorCode::PayloadTooLarge,
                "logical payload exceeds configured maximum",
            ));
        }
        if self.queued_bytes() + payload.len() > self.limits.max_outbound_queue_bytes {
            return Err(BleError::new(
                ErrorCode::QueueFull,
                "per-peer outbound queue is full",
            ));
        }
        if self.next_message_id == 0 {
            return Err(BleError::new(
                ErrorCode::Busy,
                "message ID space exhausted; renew the logical session",
            ));
        }
        let message_id = self.next_message_id;
        self.next_message_id = self.next_message_id.wrapping_add(1);
        self.queue_bytes += payload.len();
        self.queue.push_back(QueuedMessage {
            message_id,
            payload,
        });
        Ok(message_id)
    }

    /// Advances stop-and-wait delivery using the supplied monotonic time.
    ///
    /// # Errors
    ///
    /// Returns a framing error if the queued payload cannot be represented with
    /// the configured characteristic and logical-size limits.
    pub fn poll(&mut self, now_ms: u64) -> BleResult<SendAction> {
        if let Some(pending) = &mut self.pending {
            if now_ms.saturating_sub(pending.first_submitted_ms) >= self.limits.absolute_deadline_ms
            {
                let message_id = pending.message.message_id;
                self.pending = None;
                return Ok(SendAction::Failed {
                    message_id,
                    error: timeout_error(
                        "absolute send deadline elapsed; delivery may have occurred",
                    ),
                });
            }
            if now_ms.saturating_sub(pending.last_submitted_ms) >= self.limits.ack_deadline_ms {
                if pending.retransmissions >= self.limits.retransmissions {
                    let message_id = pending.message.message_id;
                    self.pending = None;
                    return Ok(SendAction::Failed {
                        message_id,
                        error: timeout_error("transport ACK deadline exhausted"),
                    });
                }
                pending.retransmissions += 1;
                pending.last_submitted_ms = now_ms;
                return Ok(SendAction::Submit {
                    message_id: pending.message.message_id,
                    frames: pending.frames.clone(),
                    retransmission: true,
                });
            }
            return Ok(SendAction::Idle);
        }

        let Some(message) = self.queue.pop_front() else {
            return Ok(SendAction::Idle);
        };
        self.queue_bytes -= message.payload.len();
        let frames = fragment(
            FrameKind::Data,
            message.message_id,
            &message.payload,
            self.limits.value_limit,
            self.limits.max_logical_size,
        )?;
        let message_id = message.message_id;
        self.pending = Some(Pending {
            message,
            frames: frames.clone(),
            first_submitted_ms: now_ms,
            last_submitted_ms: now_ms,
            retransmissions: 0,
        });
        Ok(SendAction::Submit {
            message_id,
            frames,
            retransmission: false,
        })
    }

    /// Applies an acknowledgement to the current in-flight message.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when the acknowledgement identifies a different
    /// in-flight message.
    pub fn acknowledge(&mut self, message_id: u32) -> BleResult<SendAction> {
        match &self.pending {
            Some(pending) if pending.message.message_id == message_id => {
                self.pending = None;
                Ok(SendAction::Acknowledged { message_id })
            }
            Some(_) => Err(BleError::new(
                ErrorCode::ProtocolMismatch,
                "ACK does not match the unacknowledged message",
            )),
            None => Ok(SendAction::Idle),
        }
    }

    pub fn disconnect(&mut self) -> Vec<QueuedMessage> {
        let mut failed = Vec::new();
        if let Some(pending) = self.pending.take() {
            failed.push(pending.message);
        }
        failed.extend(self.queue.drain(..));
        self.queue_bytes = 0;
        failed
    }

    #[must_use]
    pub fn queued_bytes(&self) -> usize {
        self.queue_bytes
            + self
                .pending
                .as_ref()
                .map_or(0, |pending| pending.message.payload.len())
    }
}

fn timeout_error(message: &str) -> BleError {
    BleError {
        code: ErrorCode::Timeout,
        message: message.into(),
        operation_id: None,
        resource_id: None,
        delivery: Some(DeliveryOutcome::Unknown),
        native_code: None,
    }
}

impl Default for Sender {
    fn default() -> Self {
        Self::new(SendLimits::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_and_wait_retries_twice_then_fails_honestly() {
        let mut sender = Sender::default();
        let message_id = sender.enqueue(b"hello".to_vec()).unwrap();
        assert!(matches!(
            sender.poll(0).unwrap(),
            SendAction::Submit {
                retransmission: false,
                ..
            }
        ));
        assert!(matches!(
            sender.poll(5_000).unwrap(),
            SendAction::Submit {
                retransmission: true,
                ..
            }
        ));
        assert!(matches!(
            sender.poll(10_000).unwrap(),
            SendAction::Submit {
                retransmission: true,
                ..
            }
        ));
        let SendAction::Failed {
            message_id: failed,
            error,
        } = sender.poll(15_000).unwrap()
        else {
            panic!("expected timeout")
        };
        assert_eq!(failed, message_id);
        assert_eq!(error.delivery, Some(DeliveryOutcome::Unknown));
    }

    #[test]
    fn ack_releases_next_message() {
        let mut sender = Sender::default();
        let first = sender.enqueue(vec![1]).unwrap();
        let second = sender.enqueue(vec![2]).unwrap();
        sender.poll(0).unwrap();
        assert_eq!(
            sender.acknowledge(first).unwrap(),
            SendAction::Acknowledged { message_id: first }
        );
        assert!(matches!(
            sender.poll(1).unwrap(),
            SendAction::Submit { message_id, .. } if message_id == second
        ));
    }
}
