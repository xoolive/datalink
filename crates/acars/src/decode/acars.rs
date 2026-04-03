use deku::prelude::*;
use serde::{Deserialize, Serialize};

use crate::decode::{DecodeError, DecodeResult};

const ACARS_PREAMBLE_LEN: usize = 16;
const DEL: u8 = 0x7f;
const STX: u8 = 0x02;
const ETX: u8 = 0x03;
const ETB: u8 = 0x17;
const ACK: u8 = 0x06;
const NAK: u8 = 0x15;

#[derive(Debug, DekuRead)]
struct AcarsPreamble {
    mode: u8,
    reg: [u8; 7],
    ack: u8,
    label: [u8; 2],
    block_id: u8,
}

#[derive(Debug, DekuRead)]
struct DownlinkTextHeader {
    message_number: [u8; 3],
    message_sequence: u8,
    flight_id: [u8; 6],
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MessageDirection {
    Unknown,
    GroundToAir,
    AirToGround,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AckType {
    Ack,
    Nak,
    Other(char),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReassemblyHint {
    FinalBlock,
    MoreBlocks,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcarsRawFrame {
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcarsMessage {
    pub crc_ok: bool,
    pub mode: char,
    pub reg: String,
    pub ack: AckType,
    pub label: String,
    pub block_id: char,
    pub message_number: Option<String>,
    pub message_sequence: Option<char>,
    pub flight_id: Option<String>,
    pub sublabel: Option<String>,
    pub mfi: Option<String>,
    pub txt: String,
    pub direction: MessageDirection,
    pub reassembly: ReassemblyHint,
}

impl AcarsRawFrame {
    pub fn parse(&self, direction: MessageDirection) -> DecodeResult<AcarsMessage> {
        parse_acars_frame(&self.bytes, direction)
    }
}

pub fn parse_acars_frame(buf: &[u8], direction: MessageDirection) -> DecodeResult<AcarsMessage> {
    if buf.len() < ACARS_PREAMBLE_LEN {
        return Err(DecodeError::FrameTooShort(buf.len()));
    }
    if buf.last().copied() != Some(DEL) {
        return Err(DecodeError::MissingDel);
    }

    let mut len = buf.len() - 1;
    let crc_ok = crc16_ccitt_zero(buf, len);
    if len < 2 {
        return Err(DecodeError::FrameTooShort(buf.len()));
    }
    len -= 2;

    let mut without_parity: Vec<u8> = buf[..len].iter().map(|b| b & 0x7f).collect();

    let reassembly = match without_parity.last().copied() {
        Some(ETX) => ReassemblyHint::FinalBlock,
        Some(ETB) => ReassemblyHint::MoreBlocks,
        _ => return Err(DecodeError::MissingTextTerminator),
    };
    without_parity.pop();

    let ((mut remaining, bit_offset), preamble) = AcarsPreamble::from_bytes((&without_parity, 0))
        .map_err(|e| DecodeError::Deku(e.to_string()))?;
    if bit_offset != 0 {
        return Err(DecodeError::Deku(
            "unexpected non-byte-aligned ACARS preamble".to_string(),
        ));
    }

    let mode = preamble.mode as char;
    let reg = ascii_string(&preamble.reg);
    let ack = map_ack(preamble.ack);

    let label = {
        let mut bytes = preamble.label;
        if bytes[1] == 0x7f {
            bytes[1] = b'd';
        }
        ascii_string(&bytes)
    };

    let mut block_id = preamble.block_id as char;
    if block_id == '\0' {
        block_id = ' ';
    }

    let parsed_direction = match direction {
        MessageDirection::Unknown => {
            if is_downlink_block(block_id) {
                MessageDirection::AirToGround
            } else {
                MessageDirection::GroundToAir
            }
        }
        known => known,
    };

    if remaining.is_empty() {
        if !is_downlink_block(block_id) {
            return Ok(AcarsMessage {
                crc_ok,
                mode,
                reg,
                ack,
                label,
                block_id,
                message_number: None,
                message_sequence: None,
                flight_id: None,
                sublabel: None,
                mfi: None,
                txt: String::new(),
                direction: parsed_direction,
                reassembly,
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

    let mut message_number = None;
    let mut message_sequence = None;
    let mut flight_id = None;
    let mut payload = text_bytes.as_slice();

    if is_downlink_block(block_id) {
        if payload.len() < 10 {
            return Err(DecodeError::MissingDownlinkFields);
        }

        let ((rest, bit_offset), downlink) = DownlinkTextHeader::from_bytes((payload, 0))
            .map_err(|e| DecodeError::Deku(e.to_string()))?;
        if bit_offset != 0 {
            return Err(DecodeError::Deku(
                "unexpected non-byte-aligned downlink header".to_string(),
            ));
        }

        message_number = Some(ascii_string(&downlink.message_number));
        message_sequence = Some(downlink.message_sequence as char);
        flight_id = Some(ascii_string(&downlink.flight_id));
        payload = rest;
    }

    let mut sublabel = None;
    let mut mfi = None;
    let txt = payload;
    let (offset, extracted_sublabel, extracted_mfi) =
        extract_sublabel_and_mfi(&label, parsed_direction, txt)?;
    if let Some(value) = extracted_sublabel {
        sublabel = Some(value);
    }
    if let Some(value) = extracted_mfi {
        mfi = Some(value);
    }
    let txt = ascii_string(&txt[offset..]);

    Ok(AcarsMessage {
        crc_ok,
        mode,
        reg,
        ack,
        label,
        block_id,
        message_number,
        message_sequence,
        flight_id,
        sublabel,
        mfi,
        txt,
        direction: parsed_direction,
        reassembly,
    })
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

fn is_downlink_block(block_id: char) -> bool {
    block_id.is_ascii_digit()
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

        let msg = parse_acars_frame(&bytes, MessageDirection::AirToGround).unwrap();
        assert!(msg.crc_ok);
        assert_eq!(msg.label, "23");
        assert_eq!(msg.block_id, '3');
        assert_eq!(msg.flight_id.as_deref(), Some("LO02DM"));
        assert_eq!(msg.message_number.as_deref(), Some("M09"));
        assert_eq!(msg.message_sequence, Some('A'));
        assert!(msg.txt.starts_with("ONN01LO02DM"));
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
