//! AVLC (Aeronautical VHF Link Control) frame parsing.
//!
//! VDL Mode 2 bearer layer. Each AVLC frame carries either an ACARS payload
//! (identified by the 3-byte header `0xFF 0xFF 0x01`) or an X.25 packet.
//! Addresses are 28-bit values packed in HDLC-style LSB-first octets.
//!
//! ## Frame layout (after HDLC bit-destuffing, FCS still present)
//!
//! ```text
//! ┌─────────────┬─────────────┬───────┬─────────────────┬───────┐
//! │  DST  (4 B) │  SRC  (4 B) │ LCF   │  payload (n B)  │ FCS   │
//! │  28-bit addr│  28-bit addr│ (1 B) │                 │ (2 B) │
//! └─────────────┴─────────────┴───────┴─────────────────┴───────┘
//! ```
//!
//! The frame structure follows the VDL Mode 2 AVLC link-layer model defined by
//! ICAO VDL Mode 2 SARPs.

use deku::prelude::*;
use serde::{Deserialize, Serialize};

use crate::decode::acars::{AcarsMessage, MessageDirection};
use crate::decode::helpers::{
    deserialize_addr_hex, deserialize_bytes_hex, serialize_addr_hex, serialize_bytes_hex_variant,
};
use crate::decode::x25::{parse_x25_packet, X25Packet};
use crate::decode::xid::{parse_xid, XidMessage};
use crate::decode::{DecodeError, DecodeResult};

/// CRC-16/CCITT good-frame residual (`0xF0B8`).
const GOOD_FCS: u16 = 0xF0B8;

/// AVLC station address type, encoded as a 3-bit field in the 28-bit address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddrType {
    /// Aircraft station (type 1).
    Aircraft,
    /// Ground station (types 4 and 5 — administration and delivery are both represented here).
    GroundStation,
    /// All-stations broadcast (type 7).
    AllStations,
}

impl AddrType {
    fn from_bits(raw: u8) -> Self {
        match raw {
            1 => Self::Aircraft,
            4 | 5 => Self::GroundStation,
            7 => Self::AllStations,
            _ => Self::Aircraft, // unknown: treat as aircraft for best-effort
        }
    }

    /// Whether this is an aircraft address.
    pub fn is_aircraft(self) -> bool {
        matches!(self, Self::Aircraft)
    }

    /// Whether this is a ground-station address.
    pub fn is_ground(self) -> bool {
        matches!(self, Self::GroundStation)
    }
}

/// Frame role derived from the C/R bit in the source address.
///
/// In HDLC and AVLC, each station sets the C/R bit to indicate whether it is
/// transmitting a **command** frame (the originator expects a response) or a
/// **response** frame (answering a prior command).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameRole {
    /// Sending station is issuing a command; a response is expected.
    Command,
    /// Sending station is responding to a prior command.
    Response,
}

/// Aircraft/ground status derived from the A/G bit in the destination address.
///
/// For frames addressed to an aircraft, this bit reflects the aircraft's reported
/// operational state at the time the frame was generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AircraftGroundStatus {
    /// Aircraft is airborne.
    Airborne,
    /// Aircraft is on the ground.
    OnGround,
}

/// S-frame supervisory function code.
///
/// Encodes the 2-bit supervisory function field (bits 3:2 of the LCF byte).
/// Used in `AvlcLcf::S` frames to acknowledge I-frames and manage flow control
/// within the modulo-8 sliding-window ARQ scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SFunc {
    /// Receive Ready (RR) — all I-frames up to `recv_seq - 1` received OK;
    /// ready to accept more starting at `recv_seq`.
    ReceiveReady,
    /// Receive Not Ready (RNR) — acknowledges up to `recv_seq - 1` but
    /// requests the sender to stop transmitting I-frames until further notice.
    ReceiveNotReady,
    /// Reject (REJ) — acknowledges up to `recv_seq - 1` but requests
    /// go-back-N retransmission from `recv_seq` onwards.
    Reject,
    /// Selective Reject (SREJ) — requests retransmission of a single
    /// specific I-frame (`recv_seq` only), while others remain buffered.
    SelectiveReject,
}

/// Decoded payload carried inside an AVLC I-frame or XID U-frame.
///
/// I-frames carry either ACARS (identified by the `0xFF 0xFF 0x01` header) or
/// X.25 packets. XID U-frames carry a Ground Station Information Frame (GSIF)
/// or link negotiation message. All other U-frame and S-frame payloads are not
/// carried here (they have no application-layer content).
///
/// ## Memory note
///
/// `Acars` boxes the `AcarsMessage` and `XidMessage` to avoid blowing up the
/// enum size, since they are substantially larger than the other variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvlcPayload {
    /// ACARS application message, decoded from I-frame payload starting `0xFF 0xFF 0x01`.
    Acars(Box<AcarsMessage>),
    /// X.25 packet (all other I-frame payloads).
    X25(X25Packet),
    /// XID / Ground Station Information Frame from a U-frame with `mfunc = 0x2B`.
    Xid(Box<XidMessage>),
    /// I-frame payload that could not be decoded; raw bytes preserved.
    #[serde(
        serialize_with = "serialize_bytes_hex_variant",
        deserialize_with = "deserialize_bytes_hex"
    )]
    Unknown(Vec<u8>),
}

/// A fully decoded AVLC frame.
///
/// AVLC is the link layer for VDL Mode 2. This struct captures the addressing,
/// frame type, FCS result, and — for I-frames and XID U-frames — the decoded
/// application payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvlcFrame {
    /// Destination station address.
    pub dst: AvlcAddr,
    /// Source station address.
    pub src: AvlcAddr,
    /// Frame role: `Command` or `Response`, derived from the C/R bit in `src`.
    pub role: FrameRole,
    /// Aircraft/ground status, derived from the A/G bit in `dst`.
    ///
    /// `Some` only when `dst` is an aircraft address (type 1). `None` for
    /// ground-station or broadcast destinations where the A/G bit has no meaning.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ag_status: Option<AircraftGroundStatus>,
    /// Link Control Field: I-frame, S-frame, or U-frame with sequence numbers.
    pub lcf: AvlcLcf,
    /// Whether the frame passed the CRC-16/CCITT FCS check.
    ///
    /// `false` means the frame was received with bit errors. Other fields are
    /// populated (best-effort) but should be treated as unreliable. The
    /// application payload (`payload`) is `None` for FCS-failed frames.
    #[serde(skip)]
    pub fcs_ok: bool,
    /// Decoded application payload.
    ///
    /// Present only for I-frames and XID U-frames with a passing FCS.
    /// `None` for S-frames, non-XID U-frames, or FCS failures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<AvlcPayload>,
}

/// Decoded AVLC address (28 bits: 24-bit station id + 3-bit type + 1 status bit).
///
/// The raw wire encoding is 4 HDLC-style octets with LSB-first bit packing and
/// per-octet EA (end-address) bits. `DekuRead` is implemented manually to handle
/// the bit-reversal that HDLC address encoding requires.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AvlcAddr {
    /// 24-bit station identifier (aircraft ICAO address or ground-station id).
    ///
    /// Serialised as a 6-digit lowercase hex string (e.g. `"2a3261"`).
    #[serde(serialize_with = "serialize_addr_hex")]
    #[serde(deserialize_with = "deserialize_addr_hex")]
    pub icao24: u32,
    /// Typed address category.
    pub addr_type: AddrType,
    /// Raw status bit.
    ///
    /// For aircraft addresses: `false` = airborne, `true` = on ground.
    /// For ground-station addresses: `false` = command, `true` = response
    /// (used to derive `AvlcFrame.role`).
    #[serde(skip)]
    pub status: bool,
}

impl<'a, Ctx> DekuReader<'a, Ctx> for AvlcAddr {
    fn from_reader_with_ctx<R: std::io::Read + std::io::Seek>(
        reader: &mut deku::reader::Reader<R>,
        _ctx: Ctx,
    ) -> Result<Self, DekuError> {
        let b0 = u8::from_reader_with_ctx(reader, ())?;
        let b1 = u8::from_reader_with_ctx(reader, ())?;
        let b2 = u8::from_reader_with_ctx(reader, ())?;
        let b3 = u8::from_reader_with_ctx(reader, ())?;
        Ok(Self::from_raw([b0, b1, b2, b3]))
    }
}

impl AvlcAddr {
    /// Parse from 4 raw HDLC address bytes (called by `DekuReader` and `parse_avlc_frame`).
    pub(crate) fn from_raw(buf: [u8; 4]) -> Self {
        // HDLC address: each octet has EA bit at bit-0, address bits at bits 7:1.
        // Assemble the raw 28-bit value (LSB-first within each octet), then bit-reverse.
        let raw = ((buf[0] as u32) >> 1)
            | ((buf[1] as u32) << 6)
            | ((buf[2] as u32) << 13)
            | (((buf[3] & 0xfe) as u32) << 20);
        let val = reverse_bits(raw, 28);
        let addr_type_raw = ((val >> 24) & 0x7) as u8;
        let addr_type = AddrType::from_bits(addr_type_raw);
        Self {
            icao24: val & 0x00FF_FFFF,
            addr_type,
            status: (val >> 27) & 0x1 == 1,
        }
    }

    /// Returns `true` if this is an aircraft address.
    pub fn is_aircraft(&self) -> bool {
        self.addr_type.is_aircraft()
    }

    /// Returns `true` if this is a ground-station address.
    pub fn is_ground(&self) -> bool {
        self.addr_type.is_ground()
    }
}

/// Link Control Field: encodes the AVLC frame type and sequence numbers.
///
/// The LCF is a single byte immediately following the two address fields.
/// Its two least-significant bits determine the frame class:
///
/// ```text
/// LCF bit 1:0   class   description
///     x x x 0   I       Information frame — carries application payload
///     x x 0 1   S       Supervisory frame — flow control, no payload
///     x x 1 1   U       Unnumbered frame — connection management
/// ```
///
/// ## Sequence numbers and the sliding window
///
/// AVLC uses a modulo-8 Go-Back-N ARQ. Both sides maintain:
///
/// - `send_seq` `N(S)`: the sequence number of the I-frame being sent.
/// - `recv_seq` `N(R)`: a cumulative ACK — all I-frames with `N(S) < N(R)` have been
///   received correctly. The sender may transmit up to 7 unacknowledged I-frames.
///
/// I-frames carry both; S-frames carry only `recv_seq` as a standalone ACK.
///
/// ## Poll/Final (P/F) bit
///
/// The same physical bit has different meanings depending on `role`:
///
/// | Frame role | Bit name | Meaning |
/// |---|---|---|
/// | `Command` | **Poll (P)** | Request immediate supervisory response from peer |
/// | `Response` | **Final (F)** | This is the last response in a checkpoint sequence |
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AvlcLcf {
    /// Information frame (I-frame).
    ///
    /// Carries application payload (ACARS or X.25). The sender may have up to
    /// 7 unacknowledged I-frames in flight simultaneously (modulo-8 window).
    I {
        /// Send sequence number `N(S)` of this I-frame (0–7).
        ///
        /// The receiver uses this to detect gaps and reorder out-of-sequence frames.
        send_seq: u8,
        /// Poll bit (P) when sent as a command; Final bit (F) as a response.
        ///
        /// `true` as a command requests the peer to respond immediately with its
        /// current `recv_seq`. Typically set on the last I-frame of a burst.
        poll: bool,
        /// Receive sequence number `N(R)` — piggybacked cumulative ACK.
        ///
        /// Acknowledges all I-frames with `send_seq` in `[0, recv_seq)` modulo 8.
        /// The remote sender may now slide its window forward accordingly.
        recv_seq: u8,
    },
    /// Supervisory frame (S-frame).
    ///
    /// No payload. Sent when there is no I-frame available to piggyback the ACK
    /// onto; also used for flow control (RNR) and retransmission requests (REJ/SREJ).
    S {
        /// Supervisory function — how the peer should react to `recv_seq`.
        sfunc: SFunc,
        /// Poll/Final bit — same semantics as in I-frames.
        pf: bool,
        /// Receive sequence number `N(R)` — see `AvlcLcf::I::recv_seq`.
        recv_seq: u8,
    },
    /// Unnumbered frame (U-frame).
    ///
    /// No sequence numbers. Used for link setup and teardown:
    ///
    /// | Name  | Meaning |
    /// |---|---|
    /// | `SABM` | Set Asynchronous Balanced Mode — opens a connection |
    /// | `UA`   | Unnumbered Acknowledgement — confirms SABM or DISC |
    /// | `DM`   | Disconnected Mode — rejects or closes a connection |
    /// | `DISC` | Disconnect — requests connection teardown |
    /// | `XID`  | Exchange Identification — GSIF station capability negotiation |
    /// | `FRMR` | Frame Reject — reports a framing error |
    /// | `UI`   | Unnumbered Information — data without sequencing (rarely used in VDL2) |
    U {
        /// Human-readable name of the U-frame type (e.g. `"XID"`, `"SABM"`, `"UA"`).
        name: String,
        /// Raw 6-bit M-function field (bits 7:2 of the LCF byte after masking with `0x3B`).
        ///
        /// Identifies the U-frame command/response type at the bit level.
        /// Prefer `name` for display; use `mfunc` for programmatic matching.
        mfunc: u8,
        /// Poll/Final bit — same semantics as in I- and S-frames.
        pf: bool,
    },
}

/// `AvlcFrame` implements `DekuReader`, `DekuContainerRead`, and `TryFrom<&[u8]>`
/// manually because the frame layout requires FCS verification over the entire
/// buffer and variable-length payload dispatch based on the LCF byte.
///
/// Use the standard deku entry point:
/// ```text
/// let (rest, frame) = AvlcFrame::from_bytes((buf, 0))?;  // returns remaining bytes
/// let frame = AvlcFrame::try_from(buf)?;                  // consumes whole slice
/// ```
impl<'a, Ctx> DekuReader<'a, Ctx> for AvlcFrame {
    fn from_reader_with_ctx<R: std::io::Read + std::io::Seek>(
        reader: &mut deku::reader::Reader<R>,
        _ctx: Ctx,
    ) -> Result<Self, DekuError> {
        // Read the 9 header bytes raw first — we need the original bytes for the
        // FCS check since the HDLC address packing has overlapping bit assignments
        // that make re-encoding from parsed fields unreliable.
        let [d0, d1, d2, d3, s0, s1, s2, s3, lcf_byte] =
            <[u8; 9]>::from_reader_with_ctx(reader, ())?;

        let dst = AvlcAddr::from_raw([d0, d1, d2, d3]);
        let src = AvlcAddr::from_raw([s0, s1, s2, s3]);

        // Drain remaining bytes (payload + optional FCS) via the underlying reader.
        let mut tail = Vec::<u8>::new();
        <deku::reader::Reader<R> as AsMut<R>>::as_mut(reader)
            .read_to_end(&mut tail)
            .map_err(|e| DekuError::Io(e.kind()))?;

        // FCS check: CRC over the full wire frame (original header bytes + tail).
        let has_fcs = tail.len() >= 2;
        let fcs_ok = has_fcs && {
            let header = [d0, d1, d2, d3, s0, s1, s2, s3, lcf_byte];
            let mut full = header.to_vec();
            full.extend_from_slice(&tail);
            crc16_ccitt(&full, 0xFFFF) == GOOD_FCS
        };

        let payload_bytes = if fcs_ok && has_fcs {
            &tail[..tail.len() - 2]
        } else {
            &tail[..]
        };

        let lcf = parse_lcf(lcf_byte);
        let role = if src.status {
            FrameRole::Response
        } else {
            FrameRole::Command
        };
        let ag_status = if dst.addr_type.is_aircraft() {
            Some(if dst.status {
                AircraftGroundStatus::OnGround
            } else {
                AircraftGroundStatus::Airborne
            })
        } else {
            None
        };
        let payload = if fcs_ok || !has_fcs {
            match &lcf {
                AvlcLcf::I { .. } => Some(decode_i_payload(payload_bytes, &src)),
                AvlcLcf::U { mfunc, pf, .. } if *mfunc == 0x2B => {
                    parse_xid(src.status, *pf, payload_bytes).map(|x| AvlcPayload::Xid(Box::new(x)))
                }
                _ => None,
            }
        } else {
            None
        };

        Ok(AvlcFrame {
            dst,
            src,
            role,
            ag_status,
            lcf,
            fcs_ok,
            payload,
        })
    }
}

impl<'a> DekuContainerRead<'a> for AvlcFrame {
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

impl TryFrom<&[u8]> for AvlcFrame {
    type Error = DekuError;
    fn try_from(buf: &[u8]) -> Result<Self, DekuError> {
        <Self as DekuContainerRead>::from_bytes((buf, 0)).map(|(_, v)| v)
    }
}

/// Parse an AVLC frame from raw bytes including the 2-byte FCS.
///
/// Thin wrapper around `AvlcFrame::try_from(buf)`. Prefer the deku idiom
/// `AvlcFrame::try_from(buf)` or `AvlcFrame::from_bytes((buf, 0))` in new code.
pub fn parse_avlc_frame(buf: &[u8]) -> DecodeResult<AvlcFrame> {
    AvlcFrame::try_from(buf).map_err(|e| DecodeError::Deku(e.to_string()))
}

fn decode_i_payload(bytes: &[u8], src: &AvlcAddr) -> AvlcPayload {
    if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xFF && bytes[2] == 0x01 {
        let direction = if src.is_aircraft() {
            MessageDirection::AirToGround
        } else {
            MessageDirection::GroundToAir
        };
        match AcarsMessage::from_bytes_with_direction(&bytes[3..], direction) {
            Ok(msg) => return AvlcPayload::Acars(Box::new(msg)),
            Err(_) => return AvlcPayload::Unknown(bytes.to_vec()),
        }
    }
    AvlcPayload::X25(parse_x25_packet(bytes))
}

fn parse_lcf(byte: u8) -> AvlcLcf {
    if byte & 0x01 == 0 {
        AvlcLcf::I {
            send_seq: (byte >> 1) & 0x7,
            poll: (byte >> 4) & 0x1 == 1,
            recv_seq: (byte >> 5) & 0x7,
        }
    } else if byte & 0x03 == 0x01 {
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

/// CRC-16/CCITT (polynomial 0x1021), table-based, big-endian bit order.
///
/// Used by AVLC: init = 0xFFFF, good residual = `GOOD_FCS` (0xF0B8).
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
