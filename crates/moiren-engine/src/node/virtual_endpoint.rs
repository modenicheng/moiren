//! Persistent virtual resource identity, independent of graph and driver lifetimes.
use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// An opaque 128-bit identity assigned by the control side and retained on reload.
///
/// The bytes are stored and encoded in their given order, without native GUID
/// layout or host endianness. Serde always uses a string of 32 lowercase hex
/// digits; parsing also accepts uppercase hex. All 128-bit values are valid.
///
/// This identifies desired configuration, not a live driver PDO, handle, graph
/// node or prepared binding. Backends must resolve it afresh for each lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VirtualEndpointId([u8; 16]);

impl VirtualEndpointId {
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn to_bytes(self) -> [u8; 16] {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("a virtual endpoint ID must contain exactly 32 ASCII hexadecimal digits")]
pub struct ParseVirtualEndpointIdError;

impl FromStr for VirtualEndpointId {
    type Err = ParseVirtualEndpointIdError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        fn digit(byte: u8) -> Result<u8, ParseVirtualEndpointIdError> {
            match byte {
                b'0'..=b'9' => Ok(byte - b'0'),
                b'a'..=b'f' => Ok(byte - b'a' + 10),
                b'A'..=b'F' => Ok(byte - b'A' + 10),
                _ => Err(ParseVirtualEndpointIdError),
            }
        }

        if text.len() != 32 {
            return Err(ParseVirtualEndpointIdError);
        }
        let mut bytes = [0; 16];
        for (byte, pair) in bytes.iter_mut().zip(text.as_bytes().as_chunks::<2>().0) {
            *byte = (digit(pair[0])? << 4) | digit(pair[1])?;
        }
        Ok(Self::from_bytes(bytes))
    }
}

impl fmt::Display for VirtualEndpointId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl Serialize for VirtualEndpointId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for VirtualEndpointId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests;
