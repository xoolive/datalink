//! ACARS frame parsing.
//!
//! `AcarsMessage` implements `DekuReader<'_, MessageDirection>` to allow decoding
//! directly via the deku API.  The direction is passed as context because it may
//! not always be determinable from the frame alone.
//!
//! ## Entry points
//!
//! ```text
//! // With known direction (preferred)
//! let msg = AcarsMessage::from_bytes_with_direction(buf, MessageDirection::AirToGround)?;
//!
//! // With unknown direction (inferred from block_id)
//! let (_, msg) = AcarsMessage::from_bytes((buf, 0))?;
//!
//! // Via TryFrom (unknown direction, whole-slice)
//! let msg = AcarsMessage::try_from(buf)?;
//! ```

use deku::prelude::*;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::decode::{DecodeError, DecodeResult};

const DEL: u8 = 0x7f;
const STX: u8 = 0x02;
const ETX: u8 = 0x03;
const ETB: u8 = 0x17;
const ACK: u8 = 0x06;
const NAK: u8 = 0x15;

/// Fixed 11-byte ACARS preamble (after DEL is consumed).
#[derive(Debug, DekuRead)]
struct AcarsPreamble {
    mode: u8,
    reg: [u8; 7],
    ack: u8,
    label: [u8; 2],
    block_id: u8,
}

/// 10-byte downlink text header (present after STX for downlink blocks).
#[derive(Debug, DekuRead)]
struct DownlinkTextHeader {
    message_number: [u8; 3],
    message_sequence: u8,
    flight_id: [u8; 6],
}

/// Direction of an ACARS message relative to the aircraft.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MessageDirection {
    /// Direction could not be determined from the frame alone.
    Unknown,
    /// Ground-to-air (uplink): ground station transmitting to the aircraft.
    #[serde(rename = "UL")]
    GroundToAir,
    /// Air-to-ground (downlink): aircraft transmitting to the ground station.
    #[serde(rename = "DL")]
    AirToGround,
}

/// Acknowledgement status byte from the ACARS preamble.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub enum AckType {
    /// Positive acknowledgement (`ACK`, 0x06).
    Ack,
    /// Negative acknowledgement (`NAK`, 0x15).
    Nak,
    /// Any other value, preserved as a character for diagnostics.
    Other(char),
}

impl Serialize for AckType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Ack => serializer.serialize_str("ACK"),
            Self::Nak => serializer.serialize_str("NAK"),
            Self::Other(c) => serializer.serialize_str(&c.to_string()),
        }
    }
}

fn serialize_reassembly_block_end<S>(
    reassembly: &ReassemblyHint,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_bool(matches!(reassembly, ReassemblyHint::FinalBlock))
}

fn deserialize_reassembly_block_end<'de, D>(deserializer: D) -> Result<ReassemblyHint, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(if bool::deserialize(deserializer)? {
        ReassemblyHint::FinalBlock
    } else {
        ReassemblyHint::MoreBlocks
    })
}

fn normalize_tail(raw: &[u8; 7]) -> String {
    ascii_string(raw)
        .trim_matches(|c| c == '\0' || c == ' ')
        .trim_start_matches('.')
        .to_string()
}

/// ACARS block identifier, determining message direction and session sequence.
///
/// The block ID is a single ASCII byte:
/// - `'0'`–`'9'` — downlink (aircraft→ground); the digit is the session-level
///   block sequence number (cycles 0–9 per VDL link session).
/// - Any letter — uplink (ground→aircraft).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BlockId {
    /// Downlink block; the value is the session sequence number (0–9).
    #[serde(rename = "DL")]
    Downlink(u8),
    /// Uplink block; the character is the raw block identifier letter.
    #[serde(rename = "UL")]
    Uplink(char),
}

impl BlockId {
    pub fn from_byte(b: u8) -> Self {
        let c = b as char;
        if c.is_ascii_digit() {
            Self::Downlink(c as u8 - b'0')
        } else {
            Self::Uplink(if b == 0 { ' ' } else { c })
        }
    }

    /// Whether this is a downlink block (aircraft transmitting).
    pub fn is_downlink(self) -> bool {
        matches!(self, Self::Downlink(_))
    }
}

/// Whether this ACARS block is the last (or only) block of a multi-block message.
///
/// ACARS messages longer than ~220 bytes are split into blocks. Each block
/// carries an `ETB` (End of Transmission Block, 0x17) terminator except the
/// last, which carries `ETX` (End of Text, 0x03).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReassemblyHint {
    /// `ETX` terminator — last or only block; `txt` is the complete message text.
    #[serde(rename = "ETX")]
    FinalBlock,
    /// `ETB` terminator — intermediate block; more blocks will follow with the same
    /// `(reg, label, sublabel, msg_nb)` key.
    #[serde(rename = "ETB")]
    MoreBlocks,
}

/// A decoded ACARS message.
///
/// ACARS (Aircraft Communications Addressing and Reporting System) is the primary
/// VHF datalink standard for civil aviation. A message consists of a fixed-width
/// preamble, an optional downlink text header, and a free-form text body.
///
/// ## Frame layout (simplified)
///
/// ```text
/// [DEL] [mode] [reg×7] [ack] [label×2] [block_id]
///       [message_number×3] [message_sequence] [flight_id×6]  -- downlink only
///       [STX] <text> [ETX|ETB] [CRC×2] [DEL]
/// ```
///
/// ## DekuRead
///
/// `AcarsMessage` implements `DekuReader<'_, MessageDirection>` manually because:
/// - Parity bits must be stripped from every byte before field parsing.
/// - The text body is sentinel-terminated (`ETX`/`ETB`), not length-prefixed.
/// - The CRC check covers the whole preprocessed buffer.
/// - The downlink text header is conditional on `block_id`.
/// - App-layer dispatch is post-parse logic.
///
/// Use `from_bytes_with_direction` when direction is known, or `try_from` /
/// `from_bytes` when it should be inferred from `block_id`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AcarsMessage {
    /// ACARS mode character (byte 0 of the preamble).
    pub mode: char,
    /// Aircraft registration/tail number, normalized without ACARS leading dot or padding.
    #[serde(rename = "tail")]
    pub reg: String,
    /// Acknowledgement field from the preamble.
    pub ack: AckType,
    /// Two-character ACARS label identifying the application or message type.
    pub label: String,
    /// Block identifier: direction and session sequence number.
    pub block_id: BlockId,
    /// Uplink/downlink message number (downlink messages only), e.g. `"M25"`, `"S93"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg_nb: Option<String>,
    /// Block sequence character within a multi-block message (downlink only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<char>,
    /// Airline-assigned flight identifier (downlink messages only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flight_id: Option<String>,
    /// H1-label sublabel, extracted from the text body before app dispatch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sublabel: Option<String>,
    /// Decoded text body of the ACARS message.
    #[serde(rename = "text")]
    pub txt: String,
    /// Direction of the message.
    pub direction: MessageDirection,
    /// Whether this is the last block (`FinalBlock`) or intermediate (`MoreBlocks`).
    #[serde(
        rename = "block_end",
        serialize_with = "serialize_reassembly_block_end",
        deserialize_with = "deserialize_reassembly_block_end"
    )]
    pub reassembly: ReassemblyHint,
    /// Decoded application-layer payload — exactly one variant per message.
    #[serde(rename = "app")]
    pub app: crate::decode::payload::AcarsAppPayload,
}

// ─── DekuReader + DekuContainerRead + TryFrom ─────────────────────────────────

/// `AcarsMessage` implements `DekuReader<'_, MessageDirection>` manually.
/// The direction context is used to resolve ambiguous block_id cases and to
/// correctly parse the H1 sublabel/MFI fields.
///
/// When direction is unknown, use `DekuContainerRead::from_bytes((buf, 0))` or
/// `TryFrom<&[u8]>`, both of which pass `MessageDirection::Unknown`.
impl<'a> DekuReader<'a, MessageDirection> for AcarsMessage {
    fn from_reader_with_ctx<R: std::io::Read + std::io::Seek>(
        reader: &mut deku::reader::Reader<R>,
        direction: MessageDirection,
    ) -> Result<Self, DekuError> {
        // Drain the full frame from the reader
        let mut raw = Vec::<u8>::new();
        <deku::reader::Reader<R> as AsMut<R>>::as_mut(reader)
            .read_to_end(&mut raw)
            .map_err(|e| DekuError::Io(e.kind()))?;
        decode_acars_bytes(&raw, direction).map_err(|e| DekuError::Parse(e.to_string().into()))
    }
}

impl<'a> DekuReader<'a, ()> for AcarsMessage {
    fn from_reader_with_ctx<R: std::io::Read + std::io::Seek>(
        reader: &mut deku::reader::Reader<R>,
        _ctx: (),
    ) -> Result<Self, DekuError> {
        <Self as DekuReader<'a, MessageDirection>>::from_reader_with_ctx(
            reader,
            MessageDirection::Unknown,
        )
    }
}

impl<'a> DekuContainerRead<'a> for AcarsMessage {
    fn from_reader<R: std::io::Read + std::io::Seek>(
        input: (&'a mut R, usize),
    ) -> Result<(usize, Self), DekuError>
    where
        Self: Sized,
    {
        let mut reader = deku::reader::Reader::new(input.0);
        let val = <Self as DekuReader<'_, ()>>::from_reader_with_ctx(&mut reader, ())?;
        Ok((reader.bits_read, val))
    }

    fn from_bytes(input: (&'a [u8], usize)) -> Result<((&'a [u8], usize), Self), DekuError>
    where
        Self: Sized,
    {
        let buf = input.0;
        let mut cursor = std::io::Cursor::new(buf);
        let mut reader = deku::reader::Reader::new(&mut cursor);
        let val = <Self as DekuReader<'_, ()>>::from_reader_with_ctx(&mut reader, ())?;
        let bytes_read = reader.bits_read / 8;
        Ok(((buf.get(bytes_read..).unwrap_or(&[]), 0), val))
    }
}

impl TryFrom<&[u8]> for AcarsMessage {
    type Error = DekuError;
    fn try_from(buf: &[u8]) -> Result<Self, DekuError> {
        <Self as DekuContainerRead>::from_bytes((buf, 0)).map(|(_, v)| v)
    }
}

impl AcarsMessage {
    /// Parse from raw bytes with an explicit direction.
    ///
    /// This is the preferred entry point when the bearer layer already knows the
    /// message direction (e.g. from the AVLC source address C/R bit).
    pub fn from_bytes_with_direction(
        buf: &[u8],
        direction: MessageDirection,
    ) -> DecodeResult<Self> {
        decode_acars_bytes(buf, direction)
    }
}

fn decode_acars_bytes(buf: &[u8], direction: MessageDirection) -> DecodeResult<AcarsMessage> {
    if buf.len() < 13 {
        return Err(DecodeError::FrameTooShort(buf.len()));
    }
    if buf.last().copied() != Some(DEL) {
        return Err(DecodeError::MissingDel);
    }

    let mut len = buf.len() - 1; // strip trailing DEL
                                 // CRC check before any decoding
    if !crc16_ccitt_zero(buf, len) {
        return Err(DecodeError::CrcFail);
    }
    if len < 2 {
        return Err(DecodeError::FrameTooShort(buf.len()));
    }
    len -= 2; // strip 2-byte CRC

    // Strip parity bits (bit 7) from every byte
    let mut without_parity: Vec<u8> = buf[..len].iter().map(|b| b & 0x7f).collect();

    let reassembly = match without_parity.last().copied() {
        Some(ETX) => ReassemblyHint::FinalBlock,
        Some(ETB) => ReassemblyHint::MoreBlocks,
        _ => return Err(DecodeError::MissingTextTerminator),
    };
    without_parity.pop();

    // Parse fixed preamble with deku
    let ((mut remaining, _), preamble) = AcarsPreamble::from_bytes((&without_parity, 0))
        .map_err(|e| DecodeError::Deku(e.to_string()))?;

    let mode = preamble.mode as char;
    let reg = normalize_tail(&preamble.reg);
    let ack = map_ack(preamble.ack);

    let label = {
        let mut bytes = preamble.label;
        if bytes[1] == 0x7f {
            bytes[1] = b'd';
        }
        ascii_string(&bytes)
    };

    let block_id = BlockId::from_byte(preamble.block_id);

    let direction = match direction {
        MessageDirection::Unknown => {
            if block_id.is_downlink() {
                MessageDirection::AirToGround
            } else {
                MessageDirection::GroundToAir
            }
        }
        known => known,
    };

    // Empty body: valid only for uplink ack frames
    if remaining.is_empty() {
        if !block_id.is_downlink() {
            return Ok(AcarsMessage {
                mode,
                reg,
                ack,
                label,
                block_id,
                msg_nb: None,
                sequence: None,
                flight_id: None,
                sublabel: None,
                txt: String::new(),
                direction,
                reassembly,
                app: crate::decode::payload::AcarsAppPayload::None,
            });
        }
        return Err(DecodeError::MissingDownlinkFields);
    }

    if remaining[0] != STX {
        return Err(DecodeError::MissingStx);
    }
    remaining = &remaining[1..];

    let mut text_bytes = remaining.to_vec();
    for byte in &mut text_bytes {
        if *byte == 0 {
            *byte = b'.';
        }
    }

    let mut msg_nb = None;
    let mut sequence = None;
    let mut flight_id = None;
    let mut payload = text_bytes.as_slice();

    // Downlink text header (conditional on block_id)
    if block_id.is_downlink() {
        if payload.len() < 10 {
            return Err(DecodeError::MissingDownlinkFields);
        }
        let ((rest, _), downlink) = DownlinkTextHeader::from_bytes((payload, 0))
            .map_err(|e| DecodeError::Deku(e.to_string()))?;
        msg_nb = Some(ascii_string(&downlink.message_number));
        sequence = Some(downlink.message_sequence as char);
        flight_id = Some(ascii_string(&downlink.flight_id));
        payload = rest;
    }

    // Sublabel/MFI extraction (mfi extracted but not stored in AcarsMessage)
    let (offset, sublabel, _mfi) = extract_sublabel_and_mfi(&label, direction, payload)?;
    let txt_after_sublabel = &payload[offset..];

    // App-layer dispatch
    use crate::decode::payload::AcarsAppPayload;

    let txt;

    let app = if !txt_after_sublabel.is_empty() && txt_after_sublabel[0] == b'/' {
        match crate::decode::payload::arinc622::parse_with_direction(
            &ascii_string(txt_after_sublabel),
            direction,
        ) {
            Ok(message) => {
                txt = String::new();
                AcarsAppPayload::Arinc622(message)
            }
            Err(_) => {
                txt = ascii_string(txt_after_sublabel);
                dispatch_by_label(&label, sublabel.as_deref(), &txt)
            }
        }
    } else {
        txt = ascii_string(txt_after_sublabel);
        dispatch_by_label(&label, sublabel.as_deref(), &txt)
    };

    Ok(AcarsMessage {
        mode,
        reg,
        ack,
        label,
        block_id,
        msg_nb,
        sequence,
        flight_id,
        sublabel,
        txt,
        direction,
        reassembly,
        app,
    })
}

/// Decode an ACARS text payload when frame-level parsing was performed elsewhere.
///
/// This is useful for feeds such as Airframes that provide ACARS label/text fields
/// but not the original ACARS frame bytes. It handles ARINC 622 slash/dot payloads
/// first, then falls back to label/sublabel-specific dispatch.
pub fn decode_acars_text_payload(
    label: &str,
    sublabel: Option<&str>,
    txt: &str,
    direction: MessageDirection,
) -> crate::decode::payload::AcarsAppPayload {
    if !txt.is_empty() && txt.as_bytes()[0] == b'/' {
        if let Ok(message) = crate::decode::payload::arinc622::parse_with_direction(txt, direction)
        {
            return crate::decode::payload::AcarsAppPayload::Arinc622(message);
        }
    }
    dispatch_by_label(label, sublabel, txt)
}

fn dispatch_by_label(
    label: &str,
    sublabel: Option<&str>,
    txt: &str,
) -> crate::decode::payload::AcarsAppPayload {
    use crate::decode::payload::AcarsAppPayload;
    if txt.is_empty() {
        return AcarsAppPayload::None;
    }
    // Q0 = ACARS link test / keepalive; payload is always empty by spec.
    if label == "Q0" {
        return AcarsAppPayload::LinkTest;
    }
    match label {
        "MA" => crate::decode::payload::miam::parse_miam(txt)
            .map(AcarsAppPayload::Miam)
            .unwrap_or_else(|_| AcarsAppPayload::Text(txt.to_string())),
        "SA" => crate::decode::payload::arinc620::media_advisory::parse_media_advisory(txt)
            .map(AcarsAppPayload::MediaAdvisory)
            .unwrap_or_else(|_| AcarsAppPayload::Text(txt.to_string())),
        "SQ" => crate::decode::payload::arinc620::squitter::parse_squitter(txt)
            .map(AcarsAppPayload::Squitter)
            .unwrap_or_else(|_| AcarsAppPayload::Text(txt.to_string())),
        "80" => crate::decode::payload::aoc::label80::parse_label80(txt)
            .map(AcarsAppPayload::AocReport)
            .unwrap_or(AcarsAppPayload::Text(txt.to_string())),
        // Q0 = ACARS link test / keepalive — payload is always empty by spec.
        // If text is somehow present we still label it correctly.
        "Q0" => AcarsAppPayload::LinkTest,
        "QF" => crate::decode::payload::aoc::oooi::parse_qf(txt)
            .map(AcarsAppPayload::OooiOffDestination)
            .unwrap_or_else(|| AcarsAppPayload::Text(txt.to_string())),
        "QQ" => crate::decode::payload::aoc::oooi::parse_qq(txt)
            .map(AcarsAppPayload::OooiOffReport)
            .unwrap_or_else(|| AcarsAppPayload::Text(txt.to_string())),
        "A9" => crate::decode::payload::arinc623::atis::parse_a9(txt)
            .map(AcarsAppPayload::AtisDelivery)
            .unwrap_or_else(|| AcarsAppPayload::Text(txt.to_string())),
        "A0" | "B0" => crate::decode::payload::arinc622::afn::parse_afn(txt)
            .map(AcarsAppPayload::Afn)
            .unwrap_or_else(|| AcarsAppPayload::Text(txt.to_string())),
        "B1" => crate::decode::payload::arinc622::oceanic::parse_oceanic(txt)
            .map(AcarsAppPayload::OceanicClearance)
            .unwrap_or_else(|| AcarsAppPayload::Text(txt.to_string())),
        "RA" | "C1" => crate::decode::payload::aoc::weather::parse_weather_bundle(txt)
            .map(AcarsAppPayload::Weather)
            .unwrap_or_else(|| AcarsAppPayload::Text(txt.to_string())),
        // B9 = ATIS request from aircraft (ARINC 623 TI2 protocol).
        "B9" => crate::decode::payload::arinc623::atis::parse_b9(txt)
            .map(AcarsAppPayload::AtisRequest)
            .unwrap_or_else(|| AcarsAppPayload::Text(txt.to_string())),
        "5Z" => crate::decode::payload::aoc::label5z::parse_label5z(txt)
            .map(AcarsAppPayload::Label5z)
            .unwrap_or_else(|| AcarsAppPayload::Text(txt.to_string())),
        "21" | "22" | "31" | "36" | "44" | "83" => {
            crate::decode::payload::aoc::position::parse_aoc_position(label, txt)
                .map(AcarsAppPayload::AocPosition)
                .unwrap_or_else(|| AcarsAppPayload::Text(txt.to_string()))
        }
        "32" => crate::decode::payload::aoc::label32::parse_label32(txt)
            .map(AcarsAppPayload::Label32)
            .unwrap_or_else(|| AcarsAppPayload::Text(txt.to_string())),
        "16" => crate::decode::payload::aoc::label16::parse_label16(txt)
            .map(AcarsAppPayload::Label16)
            .unwrap_or_else(|| AcarsAppPayload::Text(txt.to_string())),
        "37" => crate::decode::payload::aoc::label37::parse_label37(txt)
            .map(AcarsAppPayload::Label37)
            .unwrap_or_else(|| AcarsAppPayload::Text(txt.to_string())),
        "H1" if sublabel == Some("T1") => {
            if crate::decode::payload::boeing::ohma::is_ohma(txt) {
                crate::decode::payload::boeing::ohma::parse_ohma(txt)
                    .map(AcarsAppPayload::Ohma)
                    .unwrap_or_else(|_| AcarsAppPayload::Text(txt.to_string()))
            } else {
                AcarsAppPayload::Text(txt.to_string())
            }
        }
        _ => AcarsAppPayload::Text(txt.to_string()),
    }
}

/// Parse an ACARS frame with an explicit direction.
///
/// Thin wrapper around `AcarsMessage::from_bytes_with_direction`.
/// Prefer the deku API directly in new code.
pub fn parse_acars_frame(buf: &[u8], direction: MessageDirection) -> DecodeResult<AcarsMessage> {
    AcarsMessage::from_bytes_with_direction(buf, direction)
}

pub fn extract_sublabel_and_mfi(
    label: &str,
    direction: MessageDirection,
    txt: &[u8],
) -> DecodeResult<(usize, Option<String>, Option<String>)> {
    if label.len() < 2 {
        return Ok((0, None, None));
    }
    if direction == MessageDirection::Unknown {
        return Err(DecodeError::InvalidDirection);
    }
    if &label[..2] != "H1" {
        return Ok((0, None, None));
    }

    let mut consumed = 0usize;
    let mut ptr = txt;
    let mut sublabel = None;
    let mut mfi = None;

    match direction {
        MessageDirection::GroundToAir => {
            if ptr.len() >= 5 && &ptr[..3] == b"- #" {
                sublabel = Some(ascii_string(&ptr[3..5]));
                ptr = &ptr[5..];
                consumed += 5;
            }
        }
        MessageDirection::AirToGround => {
            if ptr.len() >= 4 && ptr[0] == b'#' && ptr[3] == b'B' {
                sublabel = Some(ascii_string(&ptr[1..3]));
                ptr = &ptr[4..];
                consumed += 4;
            }
        }
        MessageDirection::Unknown => return Err(DecodeError::InvalidDirection),
    }

    if sublabel.is_some() && ptr.len() >= 4 && ptr[0] == b'/' && ptr[3] == b' ' {
        mfi = Some(ascii_string(&ptr[1..3]));
        consumed += 4;
    }

    Ok((consumed, sublabel, mfi))
}

fn map_ack(byte: u8) -> AckType {
    match byte {
        ACK => AckType::Ack,
        NAK => AckType::Nak,
        other => AckType::Other(other as char),
    }
}

fn ascii_string(bytes: &[u8]) -> String {
    bytes.iter().map(|b| *b as char).collect()
}

fn crc16_ccitt_zero(buf: &[u8], len: usize) -> bool {
    let mut crc: u16 = 0;
    for byte in &buf[..len] {
        crc ^= *byte as u16;
        for _ in 0..8 {
            if (crc & 0x0001) != 0 {
                crc = (crc >> 1) ^ 0x8408;
            } else {
                crc >>= 1;
            }
        }
    }
    crc == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_basic_acars_example() {
        let bytes = vec![
            0x32, 0xae, 0xd3, 0xd0, 0xad, 0x4c, 0xc4, 0x45, 0x15, 0x32, 0xb3, 0xb3, 0x02, 0xcd,
            0xb0, 0xb9, 0xc1, 0x4c, 0x4f, 0xb0, 0x32, 0xc4, 0xcd, 0x4f, 0xce, 0xce, 0xb0, 0x31,
            0x4c, 0x4f, 0xb0, 0x32, 0xc4, 0xcd, 0x2f, 0x2a, 0x2a, 0x32, 0xb9, 0x32, 0xb0, 0x34,
            0x31, 0x45, 0x4c, 0x4c, 0x58, 0x45, 0xd0, 0x57, 0xc1, 0x32, 0xb0, 0x34, 0x31, 0xb0,
            0xb0, 0x32, 0x38, 0x83, 0xdf, 0xcb, 0x7f,
        ];

        // Test all three entry points give the same result
        let msg1 = AcarsMessage::from_bytes_with_direction(&bytes, MessageDirection::AirToGround)
            .expect("from_bytes_with_direction");
        let msg2 = AcarsMessage::try_from(bytes.as_slice()).expect("try_from (unknown dir)");
        let (_, msg3) = AcarsMessage::from_bytes((&bytes, 0)).expect("from_bytes");

        assert_eq!(msg1.label, "23");
        assert_eq!(msg1.block_id, BlockId::Downlink(3));
        assert_eq!(msg1.flight_id.as_deref(), Some("LO02DM"));
        assert_eq!(msg1.msg_nb.as_deref(), Some("M09"));
        assert_eq!(msg1.sequence, Some('A'));
        assert!(msg1.txt.starts_with("ONN01LO02DM"));
        assert!(matches!(
            msg1.app,
            crate::decode::payload::AcarsAppPayload::Text(_)
        ));

        // All entry points agree on the decoded fields
        assert_eq!(msg1.label, msg2.label);
        assert_eq!(msg1.label, msg3.label);
        assert_eq!(msg1.flight_id, msg2.flight_id);
    }

    #[test]
    fn extract_uplink_h1_fields() {
        let txt = b"- #MD/AA ATLTWXA.CR1.N856DN203A3AA8E5C1A9323EDD";
        let (offset, sublabel, mfi) =
            extract_sublabel_and_mfi("H1", MessageDirection::GroundToAir, txt).unwrap();
        assert_eq!(offset, 9);
        assert_eq!(sublabel.as_deref(), Some("MD"));
        assert_eq!(mfi.as_deref(), Some("AA"));
    }
}
