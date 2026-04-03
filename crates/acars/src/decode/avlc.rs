//! AVLC (Aeronautical VHF Link Control) frame parsing.
//!
//! VDL Mode 2 bearer layer: each AVLC frame carries either an ACARS payload
//! (prefixed 0xFF 0xFF 0x01) or an X.25 packet.  Addresses are 28-bit values
//! packed in HDLC-style LSB-first octets; they are assembled and then
//! bit-reversed before extracting the address, type, and status fields.
//!
//! Frame layout (after HDLC bit-destuffing, FCS still present):
//!
//! ```text
//! +-----------+-----------+-------+----------------+-------+
//! | DST  (4B) | SRC  (4B) | LCF   | payload (n B)  | FCS   |
//! +-----------+-----------+(1B)   +----------------+(2B)   +
//! ```
//!
//! References: dumpvdl2 `avlc.c`, VDL Mode 2 ICAO Annex 10.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::decode::acars::{parse_acars_frame, AcarsMessage, MessageDirection};
use crate::decode::xid::{parse_xid, XidMessage};
use crate::decode::x25::{parse_x25_packet, X25Packet};
use crate::decode::{DecodeError, DecodeResult};

/// Minimum valid AVLC frame length (4 dst + 4 src + 1 lcf + 2 fcs).
const MIN_AVLC_LEN: usize = 11;
/// CRC-16/CCITT residual for a correctly received frame.
const GOOD_FCS: u16 = 0xF0B8;

// ─── Address types ──────────────────────────────────────────────────────────

pub const ADDRTYPE_AIRCRAFT: u8 = 1;
pub const ADDRTYPE_GS_ADM: u8 = 4;
pub const ADDRTYPE_GS_DEL: u8 = 5;
pub const ADDRTYPE_ALL: u8 = 7;

// ─── Serde helpers for hex byte arrays ──────────────────────────────────────

/// Serialize `Vec<u8>` as a space-separated hex string.
pub fn serialize_bytes_hex<S: Serializer>(v: &Vec<u8>, s: S) -> Result<S::Ok, S::Error> {
    let hex: Vec<String> = v.iter().map(|b| format!("{:02x}", b)).collect();
    s.serialize_str(&hex.join(" "))
}

/// Serialize `Vec<u8>` as hex (for use in enum variants).
pub fn serialize_bytes_hex_variant<S: Serializer>(v: &Vec<u8>, s: S) -> Result<S::Ok, S::Error> {
    serialize_bytes_hex(v, s)
}

/// Serialize `Option<Vec<u8>>` as optional hex string.
pub fn serialize_opt_bytes_hex<S: Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(b) => serialize_bytes_hex(b, s),
        None => s.serialize_none(),
    }
}

/// Deserialize `Option<Vec<u8>>` — accepts null, a space-separated hex string, or a JSON byte array.
pub fn deserialize_opt_bytes_hex<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
    use serde::de::{self, SeqAccess, Visitor};

    struct OptBytesVisitor;

    impl<'de> Visitor<'de> for OptBytesVisitor {
        type Value = Option<Vec<u8>>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "null, a hex string, or a byte array")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Self::Value, D2::Error> {
            struct BytesVisitor;
            impl<'de> Visitor<'de> for BytesVisitor {
                type Value = Vec<u8>;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    write!(f, "a hex string or byte array")
                }
                fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                    v.split_whitespace()
                        .map(|h| u8::from_str_radix(h, 16).map_err(|e| E::custom(e.to_string())))
                        .collect()
                }
                fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                    let mut out = Vec::new();
                    while let Some(b) = seq.next_element::<u8>()? {
                        out.push(b);
                    }
                    Ok(out)
                }
            }
            d.deserialize_any(BytesVisitor).map(Some)
        }
    }

    d.deserialize_option(OptBytesVisitor)
}

// ─── AVLC address ───────────────────────────────────────────────────────────

fn addr_station_type(addr_type: u8) -> &'static str {
    match addr_type {
        1 => "Aircraft",
        4 | 5 => "Ground station",
        7 => "All stations",
        _ => "reserved",
    }
}

/// Decoded AVLC address (28 bits packed as 24-bit address + 3-bit type + status).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AvlcAddr {
    /// 24-bit ICAO aircraft address or ground-station id (hex).
    pub addr: u32,
    /// Address type: 1=aircraft, 4=GS_ADM, 5=GS_DEL, 7=all.
    pub addr_type: u8,
    /// Human-readable station type.
    pub station_type: String,
    /// Status bit: for aircraft = airborne(false)/on-ground(true).
    /// For C/R interpretation, see `AvlcFrame.cr`.
    pub status: bool,
}

impl AvlcAddr {
    fn parse(buf: &[u8; 4]) -> Self {
        // HDLC address: each octet has EA bit at bit-0, address bits at bits 7:1.
        // Assemble the raw 28-bit value (LSB-first within each octet), then bit-reverse.
        let raw = ((buf[0] as u32) >> 1)
            | ((buf[1] as u32) << 6)
            | ((buf[2] as u32) << 13)
            | (((buf[3] & 0xfe) as u32) << 20);
        let val = reverse_bits(raw, 28);
        let addr_type = ((val >> 24) & 0x7) as u8;
        Self {
            addr: val & 0x00FF_FFFF,
            addr_type,
            station_type: addr_station_type(addr_type).to_string(),
            status: (val >> 27) & 0x1 == 1,
        }
    }

    pub fn is_aircraft(&self) -> bool {
        self.addr_type == ADDRTYPE_AIRCRAFT
    }

    pub fn is_ground(&self) -> bool {
        self.addr_type == ADDRTYPE_GS_ADM || self.addr_type == ADDRTYPE_GS_DEL
    }
}

/// Reverse the `numbits` least-significant bits of `v`.
fn reverse_bits(mut v: u32, numbits: u32) -> u32 {
    let mut r = v;
    let mut s: i32 = 31;
    v >>= 1;
    while v != 0 {
        r <<= 1;
        r |= v & 1;
        v >>= 1;
        s -= 1;
    }
    r <<= s;
    r >> (32 - numbits)
}

// ─── Link Control Field ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SFunc {
    ReceiveReady,
    ReceiveNotReady,
    Reject,
    SelectiveReject,
}

/// U-frame name lookup.
fn u_frame_name(mfunc: u8) -> &'static str {
    match mfunc {
        0x00 => "UI",
        0x03 => "DM",
        0x10 => "DISC",
        0x18 => "UA",
        0x21 => "FRMR",
        0x2B => "XID",
        0x2C => "SABM",
        0x38 => "TEST",
        _ => "U",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AvlcLcf {
    /// Information frame: carries payload.
    I {
        send_seq: u8,
        poll: bool,
        recv_seq: u8,
    },
    /// Supervisory frame: flow control, no payload.
    S {
        sfunc: SFunc,
        pf: bool,
        recv_seq: u8,
    },
    /// Unnumbered frame: connection management (XID, UA, DM, …).
    U { name: String, mfunc: u8, pf: bool },
}

fn parse_lcf(byte: u8) -> AvlcLcf {
    if byte & 0x01 == 0 {
        // I-frame: [type:1][send_seq:3][poll:1][recv_seq:3]
        AvlcLcf::I {
            send_seq: (byte >> 1) & 0x7,
            poll: (byte >> 4) & 0x1 == 1,
            recv_seq: (byte >> 5) & 0x7,
        }
    } else if byte & 0x03 == 0x01 {
        // S-frame: [type:2][sfunc:2][pf:1][recv_seq:3]
        let sfunc = match (byte >> 2) & 0x3 {
            0 => SFunc::ReceiveReady,
            1 => SFunc::ReceiveNotReady,
            2 => SFunc::Reject,
            _ => SFunc::SelectiveReject,
        };
        AvlcLcf::S {
            sfunc,
            pf: (byte >> 4) & 0x1 == 1,
            recv_seq: (byte >> 5) & 0x7,
        }
    } else {
        // U-frame: [type:2][mfunc:6]
        let mfunc_raw = byte >> 2;
        let mfunc = mfunc_raw & 0x3b;
        let pf = (mfunc_raw >> 2) & 0x1 == 1;
        AvlcLcf::U {
            name: u_frame_name(mfunc).to_string(),
            mfunc,
            pf,
        }
    }
}

// ─── Payload ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AvlcPayload {
    Acars(AcarsMessage),
    X25(X25Packet),
    Xid(XidMessage),
    Unknown(Vec<u8>),
}

// ─── Frame ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvlcFrame {
    pub dst: AvlcAddr,
    pub src: AvlcAddr,
    /// "Command" or "Response" — from src address C/R status bit.
    pub cr: String,
    /// "Airborne" or "On ground" — from dst address A/G status bit.
    pub ag_status: String,
    pub lcf: AvlcLcf,
    pub fcs_ok: bool,
    /// Payload is `Some` only for I-frames (and XID U-frames) with a good FCS.
    pub payload: Option<AvlcPayload>,
}

/// Parse an AVLC frame from raw bytes (including the 2-byte FCS at the end).
///
/// The bytes are the product of HDLC bit-destuffing and should include the FCS.
/// The parser checks the FCS and sets `fcs_ok` accordingly.  For I-frames with
/// a good FCS, the ACARS or X.25 payload is decoded.
pub fn parse_avlc_frame(buf: &[u8]) -> DecodeResult<AvlcFrame> {
    if buf.len() < MIN_AVLC_LEN {
        return Err(DecodeError::FrameTooShort(buf.len()));
    }

    let fcs_ok = crc16_ccitt(buf, 0xFFFF) == GOOD_FCS;

    // Strip FCS bytes when present so that payload extraction is correct.
    let content = if fcs_ok {
        &buf[..buf.len() - 2]
    } else {
        buf
    };

    if content.len() < 9 {
        return Err(DecodeError::FrameTooShort(buf.len()));
    }

    let dst = AvlcAddr::parse(content[0..4].try_into().unwrap());
    let src = AvlcAddr::parse(content[4..8].try_into().unwrap());
    let lcf = parse_lcf(content[8]);
    let payload_bytes = &content[9..];

    // C/R bit: from src.status (false = Command, true = Response).
    let cr = if src.status { "Response" } else { "Command" }.to_string();
    // A/G status: from dst.status (false = Airborne, true = On ground).
    let ag_status = if dst.status { "On ground" } else { "Airborne" }.to_string();

    let payload = if fcs_ok {
        match &lcf {
            AvlcLcf::I { .. } => {
                Some(decode_i_payload(payload_bytes, &src))
            }
            AvlcLcf::U { mfunc, pf, .. } if *mfunc == 0x2B => {
                // XID frame
                parse_xid(src.status, *pf, payload_bytes)
                    .map(AvlcPayload::Xid)
            }
            _ => None,
        }
    } else {
        None
    };

    Ok(AvlcFrame { dst, src, cr, ag_status, lcf, fcs_ok, payload })
}

fn decode_i_payload(bytes: &[u8], src: &AvlcAddr) -> AvlcPayload {
    // ACARS payload is identified by the 3-byte header 0xFF 0xFF 0x01.
    if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xFF && bytes[2] == 0x01 {
        let direction = if src.is_aircraft() {
            MessageDirection::AirToGround
        } else {
            MessageDirection::GroundToAir
        };
        match parse_acars_frame(&bytes[3..], direction) {
            Ok(msg) => return AvlcPayload::Acars(msg),
            Err(_) => return AvlcPayload::Unknown(bytes.to_vec()),
        }
    }
    AvlcPayload::X25(parse_x25_packet(bytes))
}

// ─── CRC-16/CCITT ───────────────────────────────────────────────────────────

/// CRC-16/CCITT (polynomial 0x1021), table-based, big-endian bit order.
/// This is the variant used by AVLC (init=0xFFFF, good residual=0xF0B8).
fn crc16_ccitt(data: &[u8], init: u16) -> u16 {
    #[rustfmt::skip]
    static TABLE: [u16; 256] = [
        0x0000, 0x1189, 0x2312, 0x329B, 0x4624, 0x57AD, 0x6536, 0x74BF,
        0x8C48, 0x9DC1, 0xAF5A, 0xBED3, 0xCA6C, 0xDBE5, 0xE97E, 0xF8F7,
        0x1081, 0x0108, 0x3393, 0x221A, 0x56A5, 0x472C, 0x75B7, 0x643E,
        0x9CC9, 0x8D40, 0xBFDB, 0xAE52, 0xDAED, 0xCB64, 0xF9FF, 0xE876,
        0x2102, 0x308B, 0x0210, 0x1399, 0x6726, 0x76AF, 0x4434, 0x55BD,
        0xAD4A, 0xBCC3, 0x8E58, 0x9FD1, 0xEB6E, 0xFAE7, 0xC87C, 0xD9F5,
        0x3183, 0x200A, 0x1291, 0x0318, 0x77A7, 0x662E, 0x54B5, 0x453C,
        0xBDCB, 0xAC42, 0x9ED9, 0x8F50, 0xFBEF, 0xEA66, 0xD8FD, 0xC974,
        0x4204, 0x538D, 0x6116, 0x709F, 0x0420, 0x15A9, 0x2732, 0x36BB,
        0xCE4C, 0xDFC5, 0xED5E, 0xFCD7, 0x8868, 0x99E1, 0xAB7A, 0xBAF3,
        0x5285, 0x430C, 0x7197, 0x601E, 0x14A1, 0x0528, 0x37B3, 0x263A,
        0xDECD, 0xCF44, 0xFDDF, 0xEC56, 0x98E9, 0x8960, 0xBBFB, 0xAA72,
        0x6306, 0x728F, 0x4014, 0x519D, 0x2522, 0x34AB, 0x0630, 0x17B9,
        0xEF4E, 0xFEC7, 0xCC5C, 0xDDD5, 0xA96A, 0xB8E3, 0x8A78, 0x9BF1,
        0x7387, 0x620E, 0x5095, 0x411C, 0x35A3, 0x242A, 0x16B1, 0x0738,
        0xFFCF, 0xEE46, 0xDCDD, 0xCD54, 0xB9EB, 0xA862, 0x9AF9, 0x8B70,
        0x8408, 0x9581, 0xA71A, 0xB693, 0xC22C, 0xD3A5, 0xE13E, 0xF0B7,
        0x0840, 0x19C9, 0x2B52, 0x3ADB, 0x4E64, 0x5FED, 0x6D76, 0x7CFF,
        0x9489, 0x8500, 0xB79B, 0xA612, 0xD2AD, 0xC324, 0xF1BF, 0xE036,
        0x18C1, 0x0948, 0x3BD3, 0x2A5A, 0x5EE5, 0x4F6C, 0x7DF7, 0x6C7E,
        0xA50A, 0xB483, 0x8618, 0x9791, 0xE32E, 0xF2A7, 0xC03C, 0xD1B5,
        0x2942, 0x38CB, 0x0A50, 0x1BD9, 0x6F66, 0x7EEF, 0x4C74, 0x5DFD,
        0xB58B, 0xA402, 0x9699, 0x8710, 0xF3AF, 0xE226, 0xD0BD, 0xC134,
        0x39C3, 0x284A, 0x1AD1, 0x0B58, 0x7FE7, 0x6E6E, 0x5CF5, 0x4D7C,
        0xC60C, 0xD785, 0xE51E, 0xF497, 0x8028, 0x91A1, 0xA33A, 0xB2B3,
        0x4A44, 0x5BCD, 0x6956, 0x78DF, 0x0C60, 0x1DE9, 0x2F72, 0x3EFB,
        0xD68D, 0xC704, 0xF59F, 0xE416, 0x90A9, 0x8120, 0xB3BB, 0xA232,
        0x5AC5, 0x4B4C, 0x79D7, 0x685E, 0x1CE1, 0x0D68, 0x3FF3, 0x2E7A,
        0xE70E, 0xF687, 0xC41C, 0xD595, 0xA12A, 0xB0A3, 0x8238, 0x93B1,
        0x6B46, 0x7ACF, 0x4854, 0x59DD, 0x2D62, 0x3CEB, 0x0E70, 0x1FF9,
        0xF78F, 0xE606, 0xD49D, 0xC514, 0xB1AB, 0xA022, 0x92B9, 0x8330,
        0x7BC7, 0x6A4E, 0x58D5, 0x495C, 0x3DE3, 0x2C6A, 0x1EF1, 0x0F78,
    ];
    let mut crc = init;
    for &byte in data {
        crc = (crc >> 8) ^ TABLE[((crc ^ byte as u16) & 0xFF) as usize];
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc16_zero_data() {
        assert_eq!(crc16_ccitt(&[], 0xFFFF), 0xFFFF);
    }

    #[test]
    fn reverse_bits_basic() {
        assert_eq!(reverse_bits(1 << 27, 28), 0b0001);
        assert_eq!(reverse_bits(0b0001, 28), 1 << 27);
    }
}
