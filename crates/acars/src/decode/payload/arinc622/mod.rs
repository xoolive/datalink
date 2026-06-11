pub mod adsc;
pub mod afn;
pub mod cpdlc;
pub mod oceanic;

use deku::ctx::Order;
use deku::no_std_io::{Read, Seek};
use deku::prelude::*;
use deku::reader::Reader;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

use crate::decode::acars::MessageDirection;
use crate::decode::payload::PayloadError;
use crate::decode::{DecodeError, DecodeResult};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Arinc622Error {
    #[error("could not find hex payload after registration")]
    MissingHexPayload,
}

/// ARINC 622 Interline Message Identifier (IMI) — 3-character application
/// routing code embedded in the envelope address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Imi {
    /// ADS-C report (`ADS`)
    Ads,
    /// CPDLC message (`AT1`)
    At1,
    /// CPDLC connect request (`CR1`)
    Cr1,
    /// CPDLC connect confirm (`CC1`)
    Cc1,
    /// CPDLC disconnect request (`DR1`)
    Dr1,
    /// AOC message (`AB1`)
    Ab1,
    /// ADS-C disconnect (`DIS`)
    Dis,
    /// Any other 3-character IMI not listed above.
    Unknown(String),
}

impl Serialize for Imi {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Imi {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(Self::parse(&value))
    }
}

impl Imi {
    pub fn parse(s: &str) -> Self {
        match s {
            "ADS" => Self::Ads,
            "AT1" => Self::At1,
            "CR1" => Self::Cr1,
            "CC1" => Self::Cc1,
            "DR1" => Self::Dr1,
            "AB1" => Self::Ab1,
            "DIS" => Self::Dis,
            other => Self::Unknown(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Ads => "ADS",
            Self::At1 => "AT1",
            Self::Cr1 => "CR1",
            Self::Cc1 => "CC1",
            Self::Dr1 => "DR1",
            Self::Ab1 => "AB1",
            Self::Dis => "DIS",
            Self::Unknown(s) => s.as_str(),
        }
    }
}

// ─── Standards-defined ARINC 622 structure ─────────────────────────────────

/// ARINC 622 message: standards-defined address/header plus IMI-dispatched
/// application payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    /// Ground station / ATSU address, e.g. `BDOCAYA`.
    pub atsu_address: String,
    /// Interline Message Identifier. This selects the payload decoder.
    pub imi: Imi,
    /// Aircraft registration extracted from the envelope.
    pub registration: String,
    /// Decoded or preserved application payload.
    pub payload: Payload,
}

/// ARINC 622 payload decoded according to `Message::imi`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "data")]
pub enum Payload {
    /// ADS-C message with fully decoded tag list.
    Adsc(adsc::AdscMessage),
    /// FANS-1/A CPDLC message — shallow decoded header/element plus raw hex.
    Cpdlc(Box<cpdlc::CpdlcMessage>),
    /// AOC message (`AB1`) — raw hex payload.
    Aoc { payload_hex: String },
    /// Unrecognised IMI — raw hex payload preserved for diagnostics.
    Unknown { payload_hex: String },
}

/// Raw envelope read with Deku. It mirrors the wire layout:
/// `/` ATSU `.` IMI `.` registration payload_hex+crc.
#[derive(Debug, DekuRead, PartialEq, Eq)]
struct RawMessage {
    #[deku(reader = "read_slash(deku::reader)")]
    _slash: (),
    #[deku(reader = "read_ascii_until(deku::reader, b'.')")]
    atsu_address: String,
    #[deku(reader = "read_imi(deku::reader)")]
    imi: Imi,
    #[deku(reader = "read_dot(deku::reader)")]
    _dot: (),
    #[deku(reader = "read_tail(deku::reader)")]
    tail: Tail,
}

#[derive(Debug, PartialEq, Eq)]
struct Tail {
    registration: String,
    payload_hex_full: String,
}

fn read_one<R: Read + Seek>(reader: &mut Reader<R>) -> Result<u8, DekuError> {
    let mut byte = [0u8; 1];
    reader.read_bytes(1, &mut byte, Order::Msb0)?;
    Ok(byte[0])
}

fn read_slash<R: Read + Seek>(reader: &mut Reader<R>) -> Result<(), DekuError> {
    match read_one(reader)? {
        b'/' => Ok(()),
        _ => Err(DekuError::Parse(
            "ARINC 622 envelope must start with '/'".into(),
        )),
    }
}

fn read_dot<R: Read + Seek>(reader: &mut Reader<R>) -> Result<(), DekuError> {
    match read_one(reader)? {
        b'.' => Ok(()),
        _ => Err(DekuError::Parse("expected '.' separator".into())),
    }
}

fn read_ascii_until<R: Read + Seek>(
    reader: &mut Reader<R>,
    delimiter: u8,
) -> Result<String, DekuError> {
    let mut bytes = Vec::new();
    loop {
        let b = read_one(reader)?;
        if b == delimiter {
            break;
        }
        bytes.push(b);
    }
    String::from_utf8(bytes).map_err(|_| DekuError::Parse("non-UTF8 ARINC 622 field".into()))
}

fn read_imi<R: Read + Seek>(reader: &mut Reader<R>) -> Result<Imi, DekuError> {
    let mut bytes = [0u8; 3];
    for byte in &mut bytes {
        *byte = read_one(reader)?;
    }
    let s = std::str::from_utf8(&bytes)
        .map_err(|_| DekuError::Parse("non-UTF8 ARINC 622 IMI".into()))?;
    Ok(Imi::parse(s))
}

fn read_tail<R: Read + Seek>(reader: &mut Reader<R>) -> Result<Tail, DekuError> {
    let mut bytes = Vec::new();
    loop {
        match read_one(reader) {
            Ok(b) => bytes.push(b),
            Err(DekuError::Incomplete(_)) => break,
            Err(e) => return Err(e),
        }
    }
    let tail =
        String::from_utf8(bytes).map_err(|_| DekuError::Parse("non-UTF8 ARINC 622 tail".into()))?;
    split_registration_and_payload(&tail).map_err(|e| DekuError::Parse(e.to_string().into()))
}

fn split_registration_and_payload(tail: &str) -> Result<Tail, Arinc622Error> {
    // Disconnect/control messages may carry only a CRC after registration,
    // while short CPDLC messages can be 3 payload bytes plus 2 CRC bytes. Keep
    // preferring six-character registrations so hex-looking tails like JA797A
    // still split correctly.
    const MIN_HEX_BYTES: usize = 2;
    const MIN_REG_LEN: usize = 2;
    const MAX_REG_LEN: usize = 8;

    // Registrations often end in hex-looking characters (e.g. JA797A, N29968),
    // but most aircraft registrations in ARINC 622 samples are six characters.
    // Prefer a six-character split when it leaves a valid hex payload, then fall
    // back to the first valid split to avoid swallowing payload bytes.
    let mut first_valid = None;
    for i in MIN_REG_LEN..=tail.len().min(MAX_REG_LEN) {
        let remaining = &tail[i..];
        if remaining.len() >= MIN_HEX_BYTES * 2
            && remaining.len().is_multiple_of(2)
            && remaining.chars().all(|c| c.is_ascii_hexdigit())
        {
            if i == 6 {
                first_valid = Some(i);
                break;
            }
            first_valid.get_or_insert(i);
        }
    }
    if let Some(i) = first_valid {
        return Ok(Tail {
            registration: tail[..i].to_string(),
            payload_hex_full: tail[i..].to_string(),
        });
    }
    Err(Arinc622Error::MissingHexPayload)
}

impl TryFrom<&str> for Message {
    type Error = DecodeError;

    fn try_from(text: &str) -> Result<Self, Self::Error> {
        parse(text)
    }
}

/// Parse and dispatch an ARINC 622 envelope.
pub fn parse(text: &str) -> DecodeResult<Message> {
    parse_with_direction(text, MessageDirection::Unknown)
}

/// Parse and dispatch an ARINC 622 envelope with known ACARS direction.
pub fn parse_with_direction(text: &str, direction: MessageDirection) -> DecodeResult<Message> {
    let text = text.trim();
    let (_, raw) = RawMessage::from_bytes((text.as_bytes(), 0))
        .map_err(|e| DecodeError::InvalidPayload(PayloadError::Arinc622(e.to_string())))?;

    if raw.atsu_address.is_empty() {
        return Err(DecodeError::InvalidPayload(PayloadError::Arinc622(
            "missing ATSU address".to_string(),
        )));
    }
    if raw.tail.registration.is_empty() {
        return Err(DecodeError::InvalidPayload(PayloadError::Arinc622(
            "missing registration".to_string(),
        )));
    }

    let payload_hex_full = raw.tail.payload_hex_full;
    let min_payload_hex_len = match raw.imi {
        // DR1 disconnect may have only CRC (2 bytes hex), ADS uplink contracts
        // can be as short as 4 hex chars (2 payload bytes + 2 CRC).
        Imi::Dr1 => 4,
        Imi::Ads => 4,
        _ => 8,
    };
    if payload_hex_full.len() < min_payload_hex_len || !payload_hex_full.len().is_multiple_of(2) {
        return Err(DecodeError::InvalidPayload(PayloadError::Arinc622(
            format!(
                "payload must be even length and >= {min_payload_hex_len}, got {}",
                payload_hex_full.len()
            ),
        )));
    }
    if !payload_hex_full.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(DecodeError::InvalidPayload(PayloadError::Arinc622(
            "payload contains non-hex characters".to_string(),
        )));
    }

    // Strip 4-char CRC suffix to get the actual payload hex.
    let payload_no_crc = &payload_hex_full[..payload_hex_full.len() - 4];
    let payload = match &raw.imi {
        Imi::Ads => Payload::Adsc(adsc::AdscMessage {
            atsu_address: raw.atsu_address.clone(),
            registration: raw.tail.registration.clone(),
            tags: adsc::parse_adsc_payload_hex_with_direction(payload_no_crc, direction)?,
        }),
        Imi::At1 => Payload::Cpdlc(Box::new(cpdlc::parse_cpdlc_payload_hex_with_direction(
            payload_no_crc,
            direction,
        )?)),
        Imi::Cr1 => Payload::Cpdlc(Box::new(cpdlc::parse_cpdlc_control_payload_hex(
            payload_no_crc,
            cpdlc::CpdlcControlKind::ConnectRequest,
        )?)),
        Imi::Cc1 => Payload::Cpdlc(Box::new(cpdlc::parse_cpdlc_control_payload_hex(
            payload_no_crc,
            cpdlc::CpdlcControlKind::ConnectConfirm,
        )?)),
        Imi::Dr1 => Payload::Cpdlc(Box::new(cpdlc::parse_cpdlc_control_payload_hex(
            payload_no_crc,
            cpdlc::CpdlcControlKind::DisconnectRequest,
        )?)),
        Imi::Ab1 => Payload::Aoc {
            payload_hex: payload_no_crc.to_string(),
        },
        _ => Payload::Unknown {
            payload_hex: payload_no_crc.to_string(),
        },
    };

    Ok(Message {
        atsu_address: raw.atsu_address,
        imi: raw.imi,
        registration: raw.tail.registration,
        payload,
    })
}

/// Back-compatible name for the ARINC 622 parser.
pub fn parse_and_dispatch(text: &str) -> DecodeResult<Message> {
    parse(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_adsc_envelope() {
        let text =
            "/BDOCAYA.ADS.A7-ANR073759D0C997088B86BC1F0D377770C71C488B805B38E698AB9AC88B80A626";
        let msg = parse(text).expect("should parse");
        assert_eq!(msg.atsu_address, "BDOCAYA");
        assert_eq!(msg.imi, Imi::Ads);
        assert_eq!(msg.registration, "A7-ANR");
        assert!(matches!(msg.payload, Payload::Adsc(_)));
    }

    #[test]
    fn test_parse_cpdlc_cr1_envelope() {
        let text = "/ATLTWXA.CR1.N856DN203A3AA8E5C1A9323EDD";
        let msg = parse(text).expect("should parse");
        assert_eq!(msg.imi, Imi::Cr1);
        assert_eq!(msg.registration, "N856DN");
        assert!(matches!(msg.payload, Payload::Cpdlc(_)));
    }

    #[test]
    fn test_parse_adsc_h1_after_sublabel() {
        let text = "/LHWE1YA.ADS.N572UP07263B5872A048C9F21C1F0E5B88D700000239";
        let msg = parse(text).expect("should parse");
        assert_eq!(msg.imi, Imi::Ads);
        assert_eq!(msg.registration, "N572UP");
        assert_eq!(msg.atsu_address, "LHWE1YA");
    }

    #[test]
    fn test_missing_leading_slash() {
        let err = parse("BDOCAYA.ADS.A7-ANR0737...").expect_err("should fail");
        assert!(matches!(
            err,
            DecodeError::InvalidPayload(PayloadError::Arinc622(_))
        ));
    }

    #[test]
    fn test_missing_first_dot() {
        let err = parse("/BDOCAYAADSA7-ANR...").expect_err("should fail");
        assert!(matches!(
            err,
            DecodeError::InvalidPayload(PayloadError::Arinc622(_))
        ));
    }

    #[test]
    fn test_missing_second_dot() {
        let err = parse("/BDOCAYA.ADSA7-ANR...").expect_err("should fail");
        assert!(matches!(
            err,
            DecodeError::InvalidPayload(PayloadError::Arinc622(_))
        ));
    }

    #[test]
    fn test_unknown_imi() {
        let text =
            "/BDOCAYA.XYZ.A7-ANR073759D0C997088B86BC1F0D377770C71C488B805B38E698AB9AC88B80A626";
        let msg = parse(text).expect("should parse unknown IMI");
        assert_eq!(msg.imi, Imi::Unknown("XYZ".to_string()));
        assert!(matches!(msg.payload, Payload::Unknown { .. }));
    }

    #[test]
    fn test_cpdlc_payload_hex() {
        let text = "/ATLTWXA.CR1.N856DN203A3AA8E5C1A9323EDD";
        let msg = parse(text).expect("should parse");
        match msg.payload {
            Payload::Cpdlc(cpdlc) => assert_eq!(cpdlc.payload_hex, "203A3AA8E5C1A932"),
            other => panic!("expected Cpdlc, got {other:?}"),
        }
    }
}
