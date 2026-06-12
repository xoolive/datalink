//! MIAM (Management of Integrated Avionics Maintenance) decoder.
//!
//! ACARS label `MA` carries MIAM frames. This module decodes:
//! - MIAM frame wrapper (Single Transfer, File Transfer Request/Accept/Segment/Abort, XON/XOFF)
//! - MIAM CORE v1/v2 inner PDU (header decoded with deku bitfields, body decompressed with flate2)
//!
//! The outer MIAM frame consists of a single-character frame-id byte followed by text.
//! The inner MIAM CORE PDU header + body are base85-encoded and separated by `|`.

use deku::prelude::*;
use serde::{Deserialize, Serialize};

use crate::decode::payload::PayloadError;
use crate::decode::{DecodeError, DecodeResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MiamFrameId {
    SingleTransfer,
    FileTransferRequest,
    FileTransferAccept,
    FileSegment,
    FileTransferAbort,
    XoffInd,
    XonInd,
}

impl MiamFrameId {
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            'T' => Some(Self::SingleTransfer),
            'F' => Some(Self::FileTransferRequest),
            'K' => Some(Self::FileTransferAccept),
            'S' => Some(Self::FileSegment),
            'A' => Some(Self::FileTransferAbort),
            'Y' => Some(Self::XoffInd),
            'X' => Some(Self::XonInd),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SingleTransfer => "SingleTransfer",
            Self::FileTransferRequest => "FileTransferRequest",
            Self::FileTransferAccept => "FileTransferAccept",
            Self::FileSegment => "FileSegment",
            Self::FileTransferAbort => "FileTransferAbort",
            Self::XoffInd => "XoffInd",
            Self::XonInd => "XonInd",
        }
    }
}

// MIAM CORE byte layout (big-endian bit ordering):
//
//  byte 0:  [pdu_type:4][version:4]
//  bytes 1-3: [pdu_len:24] (big-endian)
//  bytes 4-10: aircraft_id[7] (ASCII)
//  byte 11: [msg_num:7][ack_option:1]
//  bytes 12-13: [compression:10 split as {8 from byte12, 2 from byte13_hi}][encoding:4][app_type:4]
//               = effectively byte12 is [compression[9:2]], byte13 is [compression[1:0]|encoding|app_type]
//               compression = ((b[0]<<2) | ((b[1]>>6)&3)) & 7
//               This is a 3-bit field split across two bytes (bits [9:7] of the pair, then [6:5])
//               In practice only bits 2..0 of the first byte and bits 7..6 of the second are used.
//  bytes 14+: app_id[2|4|6], then crc32[4]

#[derive(Debug, DekuRead)]
#[deku(endian = "big")]
#[allow(dead_code)]
pub(crate) struct MiamCoreV1RawHdr {
    #[deku(bits = 4)]
    pub pdu_type: u8,
    #[deku(bits = 4)]
    pub version: u8,
    #[deku(bits = 24)]
    pub pdu_len: u32,
    pub aircraft_id: [u8; 7],
    #[deku(bits = 7)]
    pub msg_num: u8,
    #[deku(bits = 1)]
    pub ack_option: u8,
    // compression is 3 bits split: bits[9:7] from byte12, bits[6:5] from byte13
    // compression = ((b[0]<<2) | ((b[1]>>6)&3)) & 7
    // = (b12[2:0] << 2) | b13[7:6]  → but the outer <<2 means b12 contributes only 1 bit at bit2
    // More precisely: take full byte12 and byte13, then:
    //   compression = ((byte12 << 2) | (byte13 >> 6)) & 7
    // We read the two bytes raw here and compute in `parse`.
    pub flags_byte1: u8,
    pub flags_byte2: u8,
    // then app_id follows, variable length 2/4/6 bytes
    // We stop here, rest is parsed manually
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MiamAppId {
    Acars {
        label: String,
        sublabel: Option<String>,
        mfi: Option<String>,
    },
    NonAcars {
        app_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MiamCompression {
    None,
    Deflate,
    Unknown(u8),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MiamEncoding {
    Iso5,
    Binary,
    Unknown(u8),
}

/// Decoded MIAM CORE v1 data PDU header (without body).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MiamCoreV1DataHeader {
    pub pdu_len: u32,
    pub aircraft_id: String,
    pub msg_num: u8,
    pub ack_required: bool,
    pub compression: MiamCompression,
    pub encoding: MiamEncoding,
    pub app_id: MiamAppId,
    pub header_crc: u32,
}

/// Decoded MIAM CORE v1 ack PDU header.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MiamCoreV1AckHeader {
    pub pdu_len: u32,
    pub aircraft_id: String,
    pub msg_ack_num: u8,
    pub ack_xfer_result: u8,
}

/// Decoded body after decompression (if applicable).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MiamBody {
    Acms(Box<MiamBodyAcms>),
    Text(String),
    Binary(Vec<u8>),
    Compressed { error: String },
}

/// Full decoded MIAM CORE v1 PDU.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MiamCorePdu {
    Data {
        header: MiamCoreV1DataHeader,
        body: Option<MiamBody>,
    },
    Ack(MiamCoreV1AckHeader),
    Unknown {
        pdu_type: u8,
        version: u8,
    },
}

/// Full parsed MIAM message (frame wrapper + inner CORE PDU).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MiamMessage {
    pub frame_id: String,
    pub body_pad: u8,
    pub header_pad: u8,
    pub core: MiamCorePdu,
}

// MIAM uses ASCII-85 (base85): each group of 5 chars in range [0x21..0x75] encodes 4 bytes.
// All-zero word uses 'z' shorthand.
// This is the same variant used in MIAM CORE PDUs.

fn miam_b85_decode(s: &str) -> DecodeResult<Vec<u8>> {
    let s = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 5 * 4 + 4);
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'z' {
            out.extend_from_slice(&[0u8; 4]);
            i += 1;
            continue;
        }
        if i + 5 > s.len() {
            break; // trailing incomplete group — ignore
        }
        let mut val: u32 = 0;
        for j in 0..5 {
            let c = s[i + j];
            if !(0x21..=0x75).contains(&c) {
                return Err(DecodeError::InvalidPayload(PayloadError::Miam(format!(
                    "invalid base85 character 0x{c:02x} at position {}",
                    i + j
                ))));
            }
            val = val * 85 + (c - 0x21) as u32;
        }
        out.extend_from_slice(&val.to_be_bytes());
        i += 5;
    }
    Ok(out)
}

fn parse_miam_core_v1_data_header(hdr: &[u8]) -> DecodeResult<MiamCoreV1DataHeader> {
    if hdr.len() < 16 {
        return Err(DecodeError::InvalidPayload(PayloadError::Miam(
            "header too short".into(),
        )));
    }
    // byte 0 already parsed (pdu_type / version) — but we re-read all here
    let pdu_type = hdr[0] >> 4;
    let _version = hdr[0] & 0xf;

    if pdu_type != 0 {
        return Err(DecodeError::InvalidPayload(PayloadError::Miam(format!(
            "expected data PDU (type 0), got {pdu_type}"
        ))));
    }

    let pdu_len = u32::from_be_bytes([0, hdr[1], hdr[2], hdr[3]]);
    let aircraft_id = ascii_trim(&hdr[4..11]);

    let byte11 = hdr[11];
    let msg_num = (byte11 >> 1) & 0x7f;
    let ack_option = byte11 & 0x1;

    let byte12 = hdr[12];
    let byte13 = hdr[13];
    let compression_raw = ((byte12 as u16) << 2 | (byte13 as u16) >> 6) as u8 & 0x7;
    let encoding_raw = (byte13 >> 4) & 0x3;
    let app_type = byte13 & 0xf;

    let compression = match compression_raw {
        0 => MiamCompression::None,
        1 => MiamCompression::Deflate,
        v => MiamCompression::Unknown(v),
    };
    let encoding = match encoding_raw {
        0 => MiamEncoding::Iso5,
        1 => MiamEncoding::Binary,
        v => MiamEncoding::Unknown(v),
    };

    let app_id_len: usize = match app_type {
        0 => 2,
        1 => 4,
        2 | 3 => 6,
        _ => {
            return Err(DecodeError::InvalidPayload(PayloadError::Miam(format!(
                "unknown app_type {app_type}"
            ))))
        }
    };

    if hdr.len() < 14 + app_id_len + 4 {
        return Err(DecodeError::InvalidPayload(PayloadError::Miam(
            "header too short for app_id + crc".into(),
        )));
    }

    let app_id_bytes = &hdr[14..14 + app_id_len];
    let app_id = match app_type {
        0 => MiamAppId::Acars {
            label: ascii_trim(&app_id_bytes[0..2]),
            sublabel: None,
            mfi: None,
        },
        1 => MiamAppId::Acars {
            label: ascii_trim(&app_id_bytes[0..2]),
            sublabel: Some(ascii_trim(&app_id_bytes[2..4])),
            mfi: None,
        },
        2 => MiamAppId::Acars {
            label: ascii_trim(&app_id_bytes[0..2]),
            sublabel: Some(ascii_trim(&app_id_bytes[2..4])),
            mfi: Some(ascii_trim(&app_id_bytes[4..6])),
        },
        3 => MiamAppId::NonAcars {
            app_id: ascii_trim(app_id_bytes),
        },
        _ => unreachable!(),
    };

    let crc_off = 14 + app_id_len;
    let header_crc = u32::from_be_bytes([
        hdr[crc_off],
        hdr[crc_off + 1],
        hdr[crc_off + 2],
        hdr[crc_off + 3],
    ]);

    Ok(MiamCoreV1DataHeader {
        pdu_len,
        aircraft_id,
        msg_num,
        ack_required: ack_option == 1,
        compression,
        encoding,
        app_id,
        header_crc,
    })
}

fn parse_miam_core_v1_ack_header(hdr: &[u8]) -> DecodeResult<MiamCoreV1AckHeader> {
    if hdr.len() < 16 {
        return Err(DecodeError::InvalidPayload(PayloadError::Miam(
            "ack header too short".into(),
        )));
    }
    let pdu_len = u32::from_be_bytes([0, hdr[1], hdr[2], hdr[3]]);
    let aircraft_id = ascii_trim(&hdr[4..11]);
    let msg_ack_num = (hdr[11] >> 1) & 0x7f;
    // byte 12 has [ack_xfer_result:4][_:4]
    let ack_xfer_result = (hdr[12] >> 4) & 0xf;
    Ok(MiamCoreV1AckHeader {
        pdu_len,
        aircraft_id,
        msg_ack_num,
        ack_xfer_result,
    })
}

// ─── Parse MIAM CORE PDU from header + body bytes ───────────────────────────

fn parse_miam_core_pdu(header: Vec<u8>, body: Option<Vec<u8>>) -> DecodeResult<MiamCorePdu> {
    if header.is_empty() {
        return Err(DecodeError::InvalidPayload(PayloadError::Miam(
            "empty CORE header".into(),
        )));
    }
    let pdu_type = header[0] >> 4;
    let version = header[0] & 0xf;

    match version {
        1 | 2 => {}
        v => {
            return Err(DecodeError::InvalidPayload(PayloadError::Miam(format!(
                "unsupported MIAM CORE version {v}"
            ))))
        }
    }

    match pdu_type {
        0 => {
            // Data PDU
            let hdr = parse_miam_core_v1_data_header(&header)?;
            let body = body.map(|b| decode_body(&hdr.compression, b, Some(&hdr.app_id)));
            Ok(MiamCorePdu::Data { header: hdr, body })
        }
        1 => {
            // Ack PDU
            let hdr = parse_miam_core_v1_ack_header(&header)?;
            Ok(MiamCorePdu::Ack(hdr))
        }
        _ => Ok(MiamCorePdu::Unknown { pdu_type, version }),
    }
}

fn decode_body(
    compression: &MiamCompression,
    body: Vec<u8>,
    app_id: Option<&MiamAppId>,
) -> MiamBody {
    let text = match try_decode_text(compression, body) {
        Ok(s) => s,
        Err(b) => return b,
    };
    // Try ACMS structured parse for H1/DF sublabel (Airbus ACMS/ARINC 665)
    let is_acms = matches!(app_id, Some(MiamAppId::Acars { sublabel: Some(s), .. }) if s == "DF");
    if is_acms {
        if let Some(acms) = parse_acms_body(&text) {
            return MiamBody::Acms(Box::new(acms));
        }
    }
    MiamBody::Text(text)
}

fn try_decode_text(compression: &MiamCompression, body: Vec<u8>) -> Result<String, MiamBody> {
    match compression {
        MiamCompression::None => match String::from_utf8(body.clone()) {
            Ok(s) => Ok(s),
            Err(_) => Ok(body
                .iter()
                .map(|&b| {
                    if b.is_ascii_graphic() || b == b' ' {
                        b as char
                    } else {
                        '.'
                    }
                })
                .collect()),
        },
        MiamCompression::Deflate => {
            use std::io::Read;
            let mut decoder = flate2::read::DeflateDecoder::new(&body[..]);
            let mut out = Vec::new();
            match decoder.read_to_end(&mut out) {
                Ok(_) => match String::from_utf8(out.clone()) {
                    Ok(s) => Ok(s),
                    Err(_) => Err(MiamBody::Binary(out)),
                },
                Err(e) => Err(MiamBody::Compressed {
                    error: e.to_string(),
                }),
            }
        }
        MiamCompression::Unknown(n) => Err(MiamBody::Compressed {
            error: format!("unsupported compression {n}"),
        }),
    }
}

/// Parse a MIAM message from an ACARS label-MA text payload.
///
/// The input is the raw `txt` field from the ACARS message (after label extraction).
/// Format: `<frame_id><body_pad><header_pad><hdr_b85>|<body_b85>`
///
/// Returns `Err` only on structural failures that indicate this is definitely not a
/// valid MIAM frame. Partial/body decode errors are reported inside `MiamCorePdu`.
pub fn parse_miam(txt: &str) -> DecodeResult<MiamMessage> {
    if txt.len() < 3 {
        return Err(DecodeError::InvalidPayload(PayloadError::Miam(
            "too short".into(),
        )));
    }

    let mut chars = txt.chars();
    let frame_id_char = chars.next().unwrap();
    let frame_id = MiamFrameId::from_char(frame_id_char).ok_or_else(|| {
        DecodeError::InvalidPayload(PayloadError::Miam(format!(
            "unknown frame_id '{frame_id_char}'"
        )))
    })?;

    let body_pad_char = chars.next().unwrap();
    let header_pad_char = chars.next().unwrap();

    let body_pad = if body_pad_char == '-' || body_pad_char == '.' {
        0u8
    } else {
        body_pad_char.to_digit(10).unwrap_or(0) as u8
    };
    let header_pad = header_pad_char.to_digit(10).unwrap_or(0) as u8;

    let rest = &txt[3..];

    // Only Single Transfer has MIAM CORE inner PDU in a single ACARS message
    // (multi-frame File Transfer would need reassembly; for now parse what we can)
    if frame_id != MiamFrameId::SingleTransfer {
        // For non-single-transfer frames, return a structural description without CORE decode
        return Ok(MiamMessage {
            frame_id: frame_id.as_str().to_string(),
            body_pad,
            header_pad,
            core: MiamCorePdu::Unknown {
                pdu_type: 0xff,
                version: 0,
            },
        });
    }

    // Split on | into header_b85 and optional body_b85
    let (hdr_b85, body_b85) = match rest.split_once('|') {
        Some((h, b)) => (h, if b.is_empty() { None } else { Some(b) }),
        None => {
            return Err(DecodeError::InvalidPayload(PayloadError::Miam(
                "missing | separator between header and body".into(),
            )))
        }
    };

    // Decode base85
    let mut hdr_bytes = miam_b85_decode(hdr_b85)?;
    if (header_pad as usize) <= hdr_bytes.len() {
        let new_len = hdr_bytes.len() - header_pad as usize;
        hdr_bytes.truncate(new_len);
    }

    let body_bytes = if let Some(b85) = body_b85 {
        if body_pad_char == '-' {
            // Unencoded plain text body
            Some(b85.as_bytes().to_vec())
        } else {
            let mut decoded = miam_b85_decode(b85)?;
            if (body_pad as usize) <= decoded.len() {
                let new_len = decoded.len() - body_pad as usize;
                decoded.truncate(new_len);
            }
            Some(decoded)
        }
    } else {
        None
    };

    let core = parse_miam_core_pdu(hdr_bytes, body_bytes)?;

    Ok(MiamMessage {
        frame_id: frame_id.as_str().to_string(),
        body_pad,
        header_pad,
        core,
    })
}

fn ascii_trim(bytes: &[u8]) -> String {
    let s: String = bytes
        .iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '.'
            }
        })
        .collect();
    s.trim_end_matches('\0').trim().to_string()
}

/// Parsed MIAM body text — Airbus ACMS/ARINC 665 format.
///
/// The H1/DF sublabel body uses the ARINC 665 ACMS message format:
///
/// ```text
/// <ac_type>,<seq>,<ver>,<count>,<acms_id>/REP<id>,<blk>,<pg>;<section>/<section>/...
/// ```
///
/// Sections: H01 (primary header), H02 (flight ids), H03 (optional), A0x (data).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MiamBodyAcms {
    pub ac_type: String,
    pub msg_seq: String,
    pub version: String,
    pub acms_id: String,
    pub report_id: Option<String>,
    pub block: Option<String>,
    pub page: Option<String>,
    pub h01: Option<MiamAcmsH01>,
    pub h02: Option<MiamAcmsH02>,
    pub h03: Option<Vec<String>>,
    pub data_sections: Vec<MiamAcmsData>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MiamAcmsH01 {
    pub report_id: String,
    pub block: String,
    pub page: String,
    pub ata: String,
    pub msg_num: String,
    pub registration: String,
    pub system: String,
    pub source: String,
    pub day: String,
    pub month: String,
    pub year: String,
    pub hour: String,
    pub minute: String,
    pub second: String,
    pub ms: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MiamAcmsH02 {
    pub prev_flight_id: Option<String>,
    pub curr_flight_id: Option<String>,
    pub leg_id_1: Option<String>,
    pub leg_id_2: Option<String>,
    pub carrier_flight_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MiamAcmsData {
    pub tag: String,
    pub param_id: Option<String>,
    pub count: Option<u32>,
    pub values: Vec<String>,
}

pub fn parse_acms_body(text: &str) -> Option<MiamBodyAcms> {
    let (outer, rest) = text.split_once('/')?;
    let outer_fields: Vec<&str> = outer.split(',').collect();
    if outer_fields.len() < 5 {
        return None;
    }
    let ac_type = outer_fields[0].to_string();
    let msg_seq = outer_fields[1].to_string();
    let version = outer_fields[2].to_string();
    // index 3 is count, index 4 is acms_id
    let acms_id = outer_fields[4].to_string();

    let mut report_id = None;
    let mut block = None;
    let mut page = None;
    let mut h01 = None;
    let mut h02 = None;
    let mut h03 = None;
    let mut data_sections = Vec::new();

    let mut current = rest;
    loop {
        let (sec, remainder) = match current.split_once('/') {
            Some((s, r)) => (s, Some(r)),
            None => (current, None),
        };
        if !sec.is_empty() && sec != ":" {
            // routing prefix before ;
            let (_routing, body) = if let Some((r, b)) = sec.split_once(';') {
                let rfields: Vec<&str> = r.split(',').collect();
                report_id = rfields.first().map(|s| s.to_string());
                block = rfields.get(1).map(|s| s.to_string());
                page = rfields.get(2).map(|s| s.to_string());
                (Some(r), b)
            } else {
                (None, sec)
            };
            let fields: Vec<&str> = body.split(',').collect();
            let tag = fields[0];
            match tag {
                "H01" => {
                    h01 = Some(MiamAcmsH01 {
                        report_id: fields.get(1).unwrap_or(&"").to_string(),
                        block: fields.get(2).unwrap_or(&"").to_string(),
                        page: fields.get(3).unwrap_or(&"").to_string(),
                        ata: fields.get(4).unwrap_or(&"").to_string(),
                        msg_num: fields.get(5).unwrap_or(&"").to_string(),
                        registration: fields.get(6).unwrap_or(&"").to_string(),
                        system: fields.get(7).unwrap_or(&"").to_string(),
                        source: fields.get(8).unwrap_or(&"").to_string(),
                        day: fields.get(9).unwrap_or(&"").to_string(),
                        month: fields.get(10).unwrap_or(&"").to_string(),
                        year: fields.get(11).unwrap_or(&"").to_string(),
                        hour: fields.get(12).unwrap_or(&"").to_string(),
                        minute: fields.get(13).unwrap_or(&"").to_string(),
                        second: fields.get(14).unwrap_or(&"").to_string(),
                        ms: fields.get(15).map(|s| s.to_string()),
                    });
                }
                "H02" => {
                    h02 = Some(MiamAcmsH02 {
                        prev_flight_id: fields.get(1).map(|s| s.to_string()),
                        curr_flight_id: fields.get(2).map(|s| s.to_string()),
                        leg_id_1: fields.get(3).map(|s| s.to_string()),
                        leg_id_2: fields.get(4).map(|s| s.to_string()),
                        carrier_flight_id: fields.get(5).map(|s| s.to_string()),
                    });
                }
                "H03" => {
                    h03 = Some(fields[1..].iter().map(|s| s.to_string()).collect());
                }
                t if !t.is_empty() => {
                    let count = fields.get(2).and_then(|s| s.parse::<u32>().ok());
                    data_sections.push(MiamAcmsData {
                        tag: t.to_string(),
                        param_id: fields.get(1).map(|s| s.to_string()),
                        count,
                        values: fields
                            .get(3..)
                            .unwrap_or(&[])
                            .iter()
                            .map(|s| s.to_string())
                            .collect(),
                    });
                }
                _ => {}
            }
        }
        match remainder {
            Some(r) => current = r,
            None => break,
        }
    }

    Some(MiamBodyAcms {
        ac_type,
        msg_seq,
        version,
        acms_id,
        report_id,
        block,
        page,
        h01,
        h02,
        h03,
        data_sections,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Observed fixture from vdl136.jsonl ACARS MA frame, aircraft F-WWCW
    const FIXTURE_MA: &str = "T22!<<+U/k.Eo=$p%@!'s.16q1BMX8W)!|Kh]`#_CuDq+[b:#.2AAmG`9h!##b?u?UC+_43j-9>I!;H>Ta&)0_diTJi$i,=VP.hoe[$GGG*<s*EVTG]39uIn6gf,1kS2/n,f&S0ad<'<AhG-8re6HNo;`$V`U0+2a!tea5R_iWOBcfqu?]s";

    #[test]
    fn test_parse_miam_fixture() {
        let msg = parse_miam(FIXTURE_MA).expect("should parse");
        assert_eq!(msg.frame_id, "SingleTransfer");
        assert_eq!(msg.body_pad, 2);
        assert_eq!(msg.header_pad, 2);

        match &msg.core {
            MiamCorePdu::Data { header, body } => {
                assert_eq!(header.aircraft_id, ".F-WWCW");
                assert_eq!(header.msg_num, 22);
                assert!(header.ack_required);
                assert_eq!(header.compression, MiamCompression::Deflate);
                assert_eq!(header.encoding, MiamEncoding::Iso5);
                assert_eq!(header.pdu_len, 136);
                assert_eq!(
                    header.app_id,
                    MiamAppId::Acars {
                        label: "H1".to_string(),
                        sublabel: Some("DF".to_string()),
                        mfi: None,
                    }
                );
                let body = body.as_ref().expect("body should be present");
                match body {
                    MiamBody::Acms(acms) => {
                        assert_eq!(acms.ac_type, "A350", "expected A350 aircraft type");
                        let h01 = acms.h01.as_ref().expect("H01 section should be present");
                        assert!(
                            h01.registration.contains("F-WWCW"),
                            "reg should contain F-WWCW"
                        );
                        assert_eq!(h01.day, "18");
                        assert_eq!(h01.month, "05");
                        assert_eq!(h01.hour, "11");
                    }
                    other => panic!("expected Acms body, got {other:?}"),
                }
            }
            other => panic!("expected Data PDU, got {other:?}"),
        }
    }

    #[test]
    fn test_b85_decode_basic() {
        // 'hello' in MIAM base85: 'hello' -> 5-char group
        // ascii85 of 0x00000000 = '!!!!!' but z shorthand gives [0,0,0,0]
        let decoded = miam_b85_decode("z").unwrap();
        assert_eq!(decoded, vec![0, 0, 0, 0]);
    }

    #[test]
    fn test_miam_json_serializes() {
        let msg = parse_miam(FIXTURE_MA).unwrap();
        let json = serde_json::to_string(&msg).expect("should serialize");
        assert!(json.contains("A350"));
        assert!(json.contains("SingleTransfer"));
        assert!(json.contains("Deflate"));
        assert!(json.contains("F-WWCW"));
        assert!(
            json.contains("Acms"),
            "body should be tagged as Acms variant"
        );
        assert!(
            json.contains("CDR01NG121"),
            "carrier flight id should be present"
        );
    }
}
