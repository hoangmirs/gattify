use crate::{BleError, BleResult, ErrorCode};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_MAJOR: u8 = 1;
pub const HEADER_LEN: usize = 14;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[repr(u8)]
pub enum FrameKind {
    Hello = 1,
    HelloAck = 2,
    Data = 3,
    Ack = 4,
    Close = 5,
}

impl TryFrom<u8> for FrameKind {
    type Error = BleError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Hello),
            2 => Ok(Self::HelloAck),
            3 => Ok(Self::Data),
            4 => Ok(Self::Ack),
            5 => Ok(Self::Close),
            _ => Err(BleError::new(
                ErrorCode::ProtocolMismatch,
                "unknown v1 frame kind",
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub kind: FrameKind,
    pub message_id: u32,
    pub fragment_index: u16,
    pub fragment_count: u16,
    pub total_length: u32,
    pub payload: Vec<u8>,
}

impl Frame {
    /// Decodes and validates a protocol frame within the configured allocation limit.
    ///
    /// # Errors
    ///
    /// Returns a protocol or payload-size error for malformed, unsupported, or
    /// oversized input.
    pub fn decode(bytes: &[u8], max_logical_size: usize) -> BleResult<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(BleError::new(
                ErrorCode::ProtocolMismatch,
                "frame is shorter than the 14-byte header",
            ));
        }
        if bytes.len() - HEADER_LEN > max_logical_size {
            return Err(BleError::new(
                ErrorCode::PayloadTooLarge,
                "fragment payload exceeds the logical allocation ceiling",
            ));
        }
        if bytes[0] != PROTOCOL_MAJOR {
            return Err(BleError::new(
                ErrorCode::ProtocolMismatch,
                "unsupported protocol major",
            ));
        }
        let kind = FrameKind::try_from(bytes[1])?;
        let message_id = u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]);
        let fragment_index = u16::from_le_bytes([bytes[6], bytes[7]]);
        let fragment_count = u16::from_le_bytes([bytes[8], bytes[9]]);
        let total_length = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);

        let frame = Self {
            kind,
            message_id,
            fragment_index,
            fragment_count,
            total_length,
            payload: bytes[HEADER_LEN..].to_vec(),
        };
        frame.validate(max_logical_size)?;
        Ok(frame)
    }

    /// Validates and encodes this frame using the v1 wire representation.
    ///
    /// # Errors
    ///
    /// Returns a protocol or payload-size error when the frame fields violate
    /// the v1 contract or the configured allocation limit.
    pub fn encode(&self, max_logical_size: usize) -> BleResult<Vec<u8>> {
        self.validate(max_logical_size)?;
        let mut bytes = Vec::with_capacity(HEADER_LEN + self.payload.len());
        bytes.push(PROTOCOL_MAJOR);
        bytes.push(self.kind as u8);
        bytes.extend_from_slice(&self.message_id.to_le_bytes());
        bytes.extend_from_slice(&self.fragment_index.to_le_bytes());
        bytes.extend_from_slice(&self.fragment_count.to_le_bytes());
        bytes.extend_from_slice(&self.total_length.to_le_bytes());
        bytes.extend_from_slice(&self.payload);
        Ok(bytes)
    }

    fn validate(&self, max_logical_size: usize) -> BleResult<()> {
        if self.fragment_count == 0 || self.fragment_index >= self.fragment_count {
            return Err(BleError::new(
                ErrorCode::ProtocolMismatch,
                "invalid fragment index or count",
            ));
        }
        if self.total_length as usize > max_logical_size {
            return Err(BleError::new(
                ErrorCode::PayloadTooLarge,
                "logical payload exceeds configured maximum",
            ));
        }
        if self.payload.len() > self.total_length as usize {
            return Err(BleError::new(
                ErrorCode::ProtocolMismatch,
                "fragment payload exceeds the declared logical length",
            ));
        }
        if self.kind != FrameKind::Ack {
            if self.total_length == 0
                && (self.fragment_count != 1
                    || self.fragment_index != 0
                    || !self.payload.is_empty())
            {
                return Err(BleError::new(
                    ErrorCode::ProtocolMismatch,
                    "empty messages must use one header-only frame",
                ));
            }
            if self.total_length > 0
                && (self.payload.is_empty() || u32::from(self.fragment_count) > self.total_length)
            {
                return Err(BleError::new(
                    ErrorCode::ProtocolMismatch,
                    "fragment count or payload is impossible for the declared length",
                ));
            }
        }
        match self.kind {
            FrameKind::Data if self.message_id == 0 => Err(BleError::new(
                ErrorCode::ProtocolMismatch,
                "DATA message IDs must be nonzero",
            )),
            FrameKind::Ack
                if self.message_id == 0
                    || self.fragment_index != 0
                    || self.fragment_count != 1
                    || self.total_length != 0
                    || !self.payload.is_empty() =>
            {
                Err(BleError::new(
                    ErrorCode::ProtocolMismatch,
                    "ACK must contain a nonzero ID and no payload",
                ))
            }
            FrameKind::Hello | FrameKind::HelloAck | FrameKind::Close if self.message_id != 0 => {
                Err(BleError::new(
                    ErrorCode::ProtocolMismatch,
                    "control message IDs must be zero",
                ))
            }
            _ => Ok(()),
        }
    }

    #[must_use]
    pub fn ack(message_id: u32) -> Self {
        Self {
            kind: FrameKind::Ack,
            message_id,
            fragment_index: 0,
            fragment_count: 1,
            total_length: 0,
            payload: Vec::new(),
        }
    }
}

/// Splits a logical payload into validated frames for a characteristic value limit.
///
/// # Errors
///
/// Returns an unsupported, protocol, or payload-size error when the value limit
/// cannot carry v1 frames or the payload cannot be represented within the limits.
pub fn fragment(
    kind: FrameKind,
    message_id: u32,
    payload: &[u8],
    value_limit: usize,
    max_logical_size: usize,
) -> BleResult<Vec<Vec<u8>>> {
    if value_limit < HEADER_LEN {
        return Err(BleError::new(
            ErrorCode::Unsupported,
            "characteristic value limit is below the 14-byte protocol header",
        ));
    }
    if payload.len() > max_logical_size {
        return Err(BleError::new(
            ErrorCode::PayloadTooLarge,
            "logical payload exceeds configured maximum",
        ));
    }
    let total_length = u32::try_from(payload.len()).map_err(|_| {
        BleError::new(
            ErrorCode::PayloadTooLarge,
            "logical payload exceeds the v1 length field",
        )
    })?;
    if kind == FrameKind::Ack {
        return Ok(vec![Frame::ack(message_id).encode(max_logical_size)?]);
    }
    let fragment_payload = value_limit - HEADER_LEN;
    if !payload.is_empty() && fragment_payload == 0 {
        return Err(BleError::new(
            ErrorCode::Unsupported,
            "characteristic value limit leaves no room for payload",
        ));
    }
    let count = if payload.is_empty() {
        1
    } else {
        payload.len().div_ceil(fragment_payload)
    };
    let fragment_count = u16::try_from(count).map_err(|_| {
        BleError::new(
            ErrorCode::PayloadTooLarge,
            "payload requires more than u16::MAX fragments",
        )
    })?;
    let mut frames = Vec::with_capacity(count);
    for index in 0..count {
        let fragment_index = u16::try_from(index).map_err(|_| {
            BleError::new(
                ErrorCode::PayloadTooLarge,
                "payload requires an unrepresentable fragment index",
            )
        })?;
        let start = index * fragment_payload;
        let end = usize::min(start + fragment_payload, payload.len());
        frames.push(
            Frame {
                kind,
                message_id,
                fragment_index,
                fragment_count,
                total_length,
                payload: payload[start..end].to_vec(),
            }
            .encode(max_logical_size)?,
        );
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twenty_byte_limit_yields_six_byte_fragments() {
        let payload: Vec<u8> = (0..13).collect();
        let frames = fragment(FrameKind::Data, 7, &payload, 20, 16 * 1024).unwrap();
        assert_eq!(
            frames.iter().map(Vec::len).collect::<Vec<_>>(),
            [20, 20, 15]
        );
        assert_eq!(
            Frame::decode(&frames[2], 16 * 1024).unwrap().fragment_count,
            3
        );
    }

    #[test]
    fn golden_vectors_are_little_endian() {
        let data = Frame {
            kind: FrameKind::Data,
            message_id: 0x1234_5678,
            fragment_index: 1,
            fragment_count: 3,
            total_length: 7,
            payload: vec![0xaa, 0xbb],
        }
        .encode(16 * 1024)
        .unwrap();
        assert_eq!(
            data,
            [
                0x01, 0x03, 0x78, 0x56, 0x34, 0x12, 0x01, 0x00, 0x03, 0x00, 0x07, 0x00, 0x00, 0x00,
                0xaa, 0xbb
            ]
        );
        assert_eq!(
            Frame::ack(0x1234_5678).encode(16 * 1024).unwrap(),
            [0x01, 0x04, 0x78, 0x56, 0x34, 0x12, 0, 0, 1, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn empty_data_is_one_header_only_frame() {
        let frames = fragment(FrameKind::Data, 1, &[], 14, 16 * 1024).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].len(), HEADER_LEN);
    }

    #[test]
    fn malformed_ack_is_rejected() {
        let mut bytes = Frame::ack(9).encode(16 * 1024).unwrap();
        bytes.push(1);
        assert_eq!(
            Frame::decode(&bytes, 16 * 1024).unwrap_err().code,
            ErrorCode::ProtocolMismatch
        );
    }

    #[test]
    fn oversized_received_value_is_rejected_before_payload_copy() {
        let bytes = vec![0_u8; HEADER_LEN + 17];
        assert_eq!(
            Frame::decode(&bytes, 16).unwrap_err().code,
            ErrorCode::PayloadTooLarge
        );
    }

    #[test]
    fn fragment_count_is_bounded_by_the_declared_payload() {
        let frame = Frame {
            kind: FrameKind::Data,
            message_id: 1,
            fragment_index: 0,
            fragment_count: u16::MAX,
            total_length: 1,
            payload: vec![1],
        };
        assert_eq!(
            frame.encode(16 * 1024).unwrap_err().code,
            ErrorCode::ProtocolMismatch
        );
    }
}
