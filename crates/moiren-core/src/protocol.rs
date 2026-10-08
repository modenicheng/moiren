//! Versioned, length-prefixed control frames for a NON-RT pipe/socket worker.
//! No pointer, Rust layout, buffer slot or callback-relative offset is a wire ID.
use std::io::{self, Read, Write};

pub const PROTOCOL_VERSION: u16 = 1;
pub const PARAM_BODY_BYTES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessorId(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParameterId(pub u32);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParameterKey {
    pub processor: ProcessorId,
    pub parameter: ParameterId,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamValue {
    Float(f64),
    Int(i64),
    Bool(bool),
    Enum(u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyAt {
    NextBlock,
    Frame(u64),
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParameterRequest {
    pub request_id: u64,
    pub plan_revision: u64,
    pub timeline_epoch: u64,
    pub target: ParameterKey,
    pub at: ApplyAt,
    pub value: ParamValue,
    pub ramp_frames: u32,
}
#[derive(Debug)]
pub enum ProtocolError {
    Io(io::Error),
    InvalidLength,
    UnsupportedVersion,
    UnsupportedOpcode,
    InvalidValue,
}
impl From<io::Error> for ProtocolError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl ParameterRequest {
    pub fn encode_body(self) -> Result<[u8; PARAM_BODY_BYTES], ProtocolError> {
        let (kind, value) = match self.value {
            ParamValue::Float(v) if v.is_finite() => (0u32, v.to_bits()),
            ParamValue::Float(_) => return Err(ProtocolError::InvalidValue),
            ParamValue::Int(v) => (1, v as u64),
            ParamValue::Bool(v) => (2, u64::from(v)),
            ParamValue::Enum(v) => (3, u64::from(v)),
        };
        let mut out = [0u8; PARAM_BODY_BYTES];
        out[0..2].copy_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        out[2..4].copy_from_slice(&1u16.to_le_bytes());
        out[4..12].copy_from_slice(&self.request_id.to_le_bytes());
        out[12..20].copy_from_slice(&self.plan_revision.to_le_bytes());
        out[20..28].copy_from_slice(&self.timeline_epoch.to_le_bytes());
        out[28..36].copy_from_slice(&self.target.processor.0.to_le_bytes());
        out[36..40].copy_from_slice(&self.target.parameter.0.to_le_bytes());
        let (timing, frame) = match self.at {
            ApplyAt::NextBlock => (0u32, 0),
            ApplyAt::Frame(f) => (1, f),
        };
        out[40..44].copy_from_slice(&(timing | (kind << 8)).to_le_bytes());
        out[44..52].copy_from_slice(&frame.to_le_bytes());
        out[52..60].copy_from_slice(&value.to_le_bytes());
        out[60..64].copy_from_slice(&self.ramp_frames.to_le_bytes());
        Ok(out)
    }

    pub fn decode_body(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() != PARAM_BODY_BYTES {
            return Err(ProtocolError::InvalidLength);
        }
        let u16_at = |i| u16::from_le_bytes(bytes[i..i + 2].try_into().unwrap());
        let u32_at = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        let u64_at = |i| u64::from_le_bytes(bytes[i..i + 8].try_into().unwrap());
        if u16_at(0) != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion);
        }
        if u16_at(2) != 1 {
            return Err(ProtocolError::UnsupportedOpcode);
        }
        let tag = u32_at(40);
        let at = match (tag & 255, u64_at(44)) {
            (0, 0) => ApplyAt::NextBlock,
            (1, frame) => ApplyAt::Frame(frame),
            _ => return Err(ProtocolError::InvalidValue),
        };
        let bits = u64_at(52);
        let value = match tag >> 8 {
            0 if f64::from_bits(bits).is_finite() => ParamValue::Float(f64::from_bits(bits)),
            1 => ParamValue::Int(bits as i64),
            2 if bits <= 1 => ParamValue::Bool(bits != 0),
            3 if bits <= u32::MAX as u64 => ParamValue::Enum(bits as u32),
            _ => return Err(ProtocolError::InvalidValue),
        };
        Ok(Self {
            request_id: u64_at(4),
            plan_revision: u64_at(12),
            timeline_epoch: u64_at(20),
            target: ParameterKey {
                processor: ProcessorId(u64_at(28)),
                parameter: ParameterId(u32_at(36)),
            },
            at,
            value,
            ramp_frames: u32_at(60),
        })
    }
}

/// Reject oversized frames BEFORE reading their bodies or allocating memory.
/// Close the connection on framing/version errors; do not guess a resync point.
pub fn read_parameter(reader: &mut impl Read) -> Result<ParameterRequest, ProtocolError> {
    let mut prefix = [0u8; 4];
    reader.read_exact(&mut prefix)?;
    if u32::from_le_bytes(prefix) as usize != PARAM_BODY_BYTES {
        return Err(ProtocolError::InvalidLength);
    }
    let mut body = [0u8; PARAM_BODY_BYTES];
    reader.read_exact(&mut body)?;
    ParameterRequest::decode_body(&body)
}
pub fn write_parameter(
    writer: &mut impl Write,
    request: ParameterRequest,
) -> Result<(), ProtocolError> {
    let body = request.encode_body()?;
    writer.write_all(&(PARAM_BODY_BYTES as u32).to_le_bytes())?;
    writer.write_all(&body)?;
    Ok(())
}

/// The worker may emit Accepted immediately, but only RT may report Applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ReplyCode {
    Accepted = 0,
    Applied = 1,
    AppliedLate = 2,
    StaleRevision = 3,
    StaleEpoch = 4,
    UnknownParameter = 5,
    InvalidValue = 6,
    OutOfOrder = 7,
    QueueFull = 8,
    InvalidTime = 9,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlReply {
    pub request_id: u64,
    pub plan_revision: u64,
    pub timeline_epoch: u64,
    pub code: ReplyCode,
    /// Zero for Accepted/rejected; meaningful only for Applied/AppliedLate.
    pub effective_frame: u64,
}
pub fn write_reply(writer: &mut impl Write, reply: ControlReply) -> Result<(), ProtocolError> {
    let mut body = [0u8; 40];
    body[0..2].copy_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    body[2..4].copy_from_slice(&2u16.to_le_bytes());
    body[4..8].copy_from_slice(&(reply.code as u32).to_le_bytes());
    body[8..16].copy_from_slice(&reply.request_id.to_le_bytes());
    body[16..24].copy_from_slice(&reply.plan_revision.to_le_bytes());
    body[24..32].copy_from_slice(&reply.timeline_epoch.to_le_bytes());
    body[32..40].copy_from_slice(&reply.effective_frame.to_le_bytes());
    writer.write_all(&40u32.to_le_bytes())?;
    writer.write_all(&body)?;
    Ok(())
}
pub fn read_reply(reader: &mut impl Read) -> Result<ControlReply, ProtocolError> {
    let mut prefix = [0u8; 4];
    reader.read_exact(&mut prefix)?;
    if u32::from_le_bytes(prefix) != 40 {
        return Err(ProtocolError::InvalidLength);
    }
    let mut b = [0u8; 40];
    reader.read_exact(&mut b)?;
    if u16::from_le_bytes(b[0..2].try_into().unwrap()) != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion);
    }
    if u16::from_le_bytes(b[2..4].try_into().unwrap()) != 2 {
        return Err(ProtocolError::UnsupportedOpcode);
    }
    let code = match u32::from_le_bytes(b[4..8].try_into().unwrap()) {
        0 => ReplyCode::Accepted,
        1 => ReplyCode::Applied,
        2 => ReplyCode::AppliedLate,
        3 => ReplyCode::StaleRevision,
        4 => ReplyCode::StaleEpoch,
        5 => ReplyCode::UnknownParameter,
        6 => ReplyCode::InvalidValue,
        7 => ReplyCode::OutOfOrder,
        8 => ReplyCode::QueueFull,
        9 => ReplyCode::InvalidTime,
        _ => return Err(ProtocolError::InvalidValue),
    };
    let number = |i| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
    Ok(ControlReply {
        request_id: number(8),
        plan_revision: number(16),
        timeline_epoch: number(24),
        code,
        effective_frame: number(32),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> ParameterRequest {
        ParameterRequest {
            request_id: 10,
            plan_revision: 3,
            timeline_epoch: 2,
            target: ParameterKey {
                processor: ProcessorId(8),
                parameter: ParameterId(1),
            },
            at: ApplyAt::Frame(4096),
            value: ParamValue::Float(0.5),
            ramp_frames: 32,
        }
    }
    #[test]
    fn typed_framed_roundtrip_and_concatenation() {
        let mut stream = Vec::new();
        let values = [
            ParamValue::Float(0.5),
            ParamValue::Int(i64::MIN),
            ParamValue::Bool(true),
            ParamValue::Enum(u32::MAX),
        ];
        for value in values {
            write_parameter(&mut stream, ParameterRequest { value, ..request() }).unwrap();
        }
        let mut reader = stream.as_slice();
        for value in values {
            assert_eq!(read_parameter(&mut reader).unwrap().value, value);
        }
        assert!(reader.is_empty());
        let reply = ControlReply {
            request_id: 10,
            plan_revision: 3,
            timeline_epoch: 2,
            code: ReplyCode::AppliedLate,
            effective_frame: 5000,
        };
        let mut bytes = Vec::new();
        write_reply(&mut bytes, reply).unwrap();
        assert_eq!(read_reply(&mut bytes.as_slice()).unwrap(), reply);
    }
    #[test]
    fn rejects_length_version_opcode_nan_and_truncation() {
        assert!(matches!(
            read_parameter(&mut u32::MAX.to_le_bytes().as_slice()),
            Err(ProtocolError::InvalidLength)
        ));
        let mut body = request().encode_body().unwrap();
        body[0] = 2;
        assert!(matches!(
            ParameterRequest::decode_body(&body),
            Err(ProtocolError::UnsupportedVersion)
        ));
        body[0] = 1;
        body[2] = 99;
        assert!(matches!(
            ParameterRequest::decode_body(&body),
            Err(ProtocolError::UnsupportedOpcode)
        ));
        body[2] = 1;
        body[52..60].copy_from_slice(&f64::NAN.to_le_bytes());
        assert!(matches!(
            ParameterRequest::decode_body(&body),
            Err(ProtocolError::InvalidValue)
        ));
        assert!(read_parameter(&mut &[64, 0, 0, 0, 1][..]).is_err());
    }
}
