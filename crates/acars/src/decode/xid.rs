//! XID (Exchange Identification) frame decoder for VDL Mode 2.
//!
//! XID frames carry ground-station information (GSIF) or link management
//! messages (LE, LCR, HO, LPM).  The payload format is:
//!
//! ```text
//! 0x82  (XID format ID)
//! [GID=0x80  grouplen(2B BE)  TLV…]   ← public parameters (optional)
//! [GID=0xF0  grouplen(2B BE)  TLV…]   ← VDL private parameters
//! ```
//!
//! Each TLV within a group is:  typecode(1B) + length(1B) + data.

use serde::{Deserialize, Serialize};

/// XID type names, indexed by `(cr<<3)|(pf<<2)|(h<<1)|r`.
static XID_NAMES: [(&str, &str); 16] = [
    ("", ""),
    ("XID_CMD_LCR", "Link Connection Refused"),
    ("XID_CMD_HO", "Handoff Request / Broadcast Handoff"),
    ("GSIF", "Ground Station Information Frame"),
    ("XID_CMD_LE", "Link Establishment"),
    ("", ""),
    ("XID_CMD_HO", "Handoff Initiation"),
    ("XID_CMD_LPM", "Link Parameter Modification"),
    ("", ""),
    ("", ""),
    ("", ""),
    ("", ""),
    ("XID_RSP_LE", "Link Establishment Response"),
    ("XID_RSP_LCR", "Link Connection Refused Response"),
    ("XID_RSP_HO", "Handoff Response"),
    ("XID_RSP_LPM", "Link Parameter Modification Response"),
];

/// One entry in the frequency support list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreqSupport {
    /// Ground station ICAO address.
    pub gs_addr: String,
    /// Channel frequency in MHz.
    pub freq_mhz: f32,
    /// Modulation description (e.g. "VDL-M2, D8PSK, 31500 bps").
    pub modulation: String,
}

/// Ground-station geodetic location.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GsLocation {
    pub lat: f32,
    pub lon: f32,
}

/// An unparsed TLV parameter (typecode + raw data).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawTlv {
    pub typecode: u8,
    #[serde(
        serialize_with = "crate::decode::helpers::serialize_bytes_hex",
        deserialize_with = "crate::decode::helpers::deserialize_bytes_hex"
    )]
    pub data: Vec<u8>,
}

/// Public (HDLC) XID parameters.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct XidPubParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param_set_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "crate::decode::helpers::serialize_opt_bytes_hex")]
    #[serde(deserialize_with = "crate::decode::helpers::deserialize_opt_bytes_hex")]
    pub procedure_classes: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "crate::decode::helpers::serialize_opt_bytes_hex")]
    #[serde(deserialize_with = "crate::decode::helpers::deserialize_opt_bytes_hex")]
    pub hdlc_options: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other: Vec<RawTlv>,
}

/// VDL Mode 2–specific XID parameters.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct XidVdlParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param_set_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "crate::decode::helpers::serialize_opt_bytes_hex")]
    #[serde(deserialize_with = "crate::decode::helpers::deserialize_opt_bytes_hex")]
    pub avlc_specific_options: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub freq_support_list: Vec<FreqSupport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "crate::decode::helpers::serialize_opt_bytes_hex")]
    #[serde(deserialize_with = "crate::decode::helpers::deserialize_opt_bytes_hex")]
    pub timer_t4: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub airport_coverage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "crate::decode::helpers::serialize_opt_bytes_hex")]
    #[serde(deserialize_with = "crate::decode::helpers::deserialize_opt_bytes_hex")]
    pub atn_router_nets: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub system_mask: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gs_location: Option<GsLocation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other: Vec<RawTlv>,
}

/// Decoded XID message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XidMessage {
    pub xid_type: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pub_params: Option<XidPubParams>,
    pub vdl_params: XidVdlParams,
}

/// Parse an XID payload.
///
/// `cr`  — source address C/R bit (false = Command, true = Response).
/// `pf`  — P/F bit from the U-frame LCF.
/// `buf` — bytes after the LCF byte.
pub fn parse_xid(cr: bool, pf: bool, buf: &[u8]) -> Option<XidMessage> {
    const XID_FMT_ID: u8 = 0x82;
    const GID_PUBLIC: u8 = 0x80;
    const GID_PRIVATE: u8 = 0xF0;
    const MIN_GROUPLEN: usize = 3; // GID + len(2)

    if buf.len() < 1 + 2 * MIN_GROUPLEN {
        return None;
    }
    if buf[0] != XID_FMT_ID {
        return None;
    }
    let mut pos = 1usize;
    let mut pub_params: Option<XidPubParams> = None;
    let mut vdl_params: Option<XidVdlParams> = None;

    while pos + MIN_GROUPLEN <= buf.len() {
        let gid = buf[pos];
        let grouplen = u16::from_be_bytes([buf[pos + 1], buf[pos + 2]]) as usize;
        pos += 3;
        if pos + grouplen > buf.len() {
            break;
        }
        let group = &buf[pos..pos + grouplen];
        match gid {
            GID_PUBLIC => {
                pub_params = Some(parse_pub_params(group));
            }
            GID_PRIVATE => {
                vdl_params = Some(parse_vdl_params(group));
            }
            _ => {}
        }
        pos += grouplen;
    }

    let vdl = vdl_params?;

    // Determine XID type from C/R, P/F, and connection-management parameter.
    // If no conn-mgmt param, h and r are forced to 1 (GSIF / LPM case).
    let (h, r) = find_conn_mgmt(&vdl);
    let type_idx = ((cr as u8) << 3) | ((pf as u8) << 2) | (h << 1) | r;
    let (name, description) = XID_NAMES[type_idx as usize];

    Some(XidMessage {
        xid_type: name.to_string(),
        description: description.to_string(),
        pub_params,
        vdl_params: vdl,
    })
}

fn parse_pub_params(buf: &[u8]) -> XidPubParams {
    let mut p = XidPubParams::default();
    for_each_tlv(buf, |typecode, data| match typecode {
        0x01 => p.param_set_id = bytes_to_ascii(data),
        0x02 => p.procedure_classes = Some(data.to_vec()),
        0x03 => p.hdlc_options = Some(data.to_vec()),
        _ => p.other.push(RawTlv {
            typecode,
            data: data.to_vec(),
        }),
    });
    p
}

fn parse_vdl_params(buf: &[u8]) -> XidVdlParams {
    let mut p = XidVdlParams::default();
    for_each_tlv(buf, |typecode, data| match typecode {
        0x00 => p.param_set_id = bytes_to_ascii(data),
        0x04 => p.avlc_specific_options = Some(data.to_vec()),
        0x42 => p.timer_t4 = Some(data.to_vec()),
        0xC0 => p.freq_support_list = parse_freq_support_list(data),
        0xC1 => p.airport_coverage = bytes_to_ascii(data),
        0xC4 => p.atn_router_nets = Some(data.to_vec()),
        0xC5 => p.system_mask = parse_system_mask(data),
        0xC8 => p.gs_location = parse_gs_location(data),
        _ => p.other.push(RawTlv {
            typecode,
            data: data.to_vec(),
        }),
    });
    p
}

/// Iterate over 1-byte-tag, 1-byte-length, N-byte-data TLVs.
fn for_each_tlv<F: FnMut(u8, &[u8])>(buf: &[u8], mut f: F) {
    let mut pos = 0;
    while pos + 2 <= buf.len() {
        let typecode = buf[pos];
        let len = buf[pos + 1] as usize;
        pos += 2;
        if pos + len > buf.len() {
            break;
        }
        f(typecode, &buf[pos..pos + len]);
        pos += len;
    }
}

fn bytes_to_ascii(b: &[u8]) -> Option<String> {
    let s: String = b
        .iter()
        .map(|&c| {
            if c.is_ascii_graphic() || c == b' ' {
                c as char
            } else {
                '.'
            }
        })
        .collect();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Parse the frequency support list (6 bytes per entry).
fn parse_freq_support_list(buf: &[u8]) -> Vec<FreqSupport> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 6 <= buf.len() {
        let freq_bytes = &buf[pos..pos + 2];
        let addr_bytes = &buf[pos + 2..pos + 6];
        let (freq_mhz, modulation) = parse_vdl2_frequency(freq_bytes);
        let addr = parse_dlc_addr_24(addr_bytes);
        out.push(FreqSupport {
            gs_addr: format!("{:06X}", addr),
            freq_mhz,
            modulation: modulation.to_string(),
        });
        pos += 6;
    }
    out
}

/// Parse a VDL2 frequency from 2 bytes.
/// Returns (MHz, modulation_description).
fn parse_vdl2_frequency(buf: &[u8]) -> (f32, &'static str) {
    let modulations = buf[0] >> 4;
    let freq_raw = u16::from_be_bytes([buf[0], buf[1]]) & 0x0FFF;
    let mut freq_khz = (freq_raw as u32 + 10000) * 10;
    if !freq_khz.is_multiple_of(25) {
        freq_khz += 25 - freq_khz % 25;
    }
    let freq_mhz = freq_khz as f32 / 1000.0;
    let modulation = match modulations {
        2 => "VDL-M2, D8PSK, 31500 bps",
        4 => "VDL-M3, D8PSK, 31500 bps",
        _ => "Unknown modulation",
    };
    (freq_mhz, modulation)
}

/// Extract the 24-bit ICAO address from a 4-byte DLC address field.
fn parse_dlc_addr_24(buf: &[u8]) -> u32 {
    let raw = ((buf[0] as u32) >> 1)
        | ((buf[1] as u32) << 6)
        | ((buf[2] as u32) << 13)
        | (((buf[3] & 0xFE) as u32) << 20);
    reverse_bits_28(raw) & 0x00FF_FFFF
}

fn reverse_bits_28(mut v: u32) -> u32 {
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
    r >> (32 - 28)
}

/// Parse the system mask (list of DLC addresses).
fn parse_system_mask(buf: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 4 <= buf.len() {
        let addr = parse_dlc_addr_24(&buf[pos..pos + 4]);
        out.push(format!("{:06X}", addr));
        pos += 4;
    }
    out
}

/// Parse ground-station location (3 bytes = 12-bit lat + 12-bit lon, signed, tenths of degrees).
fn parse_gs_location(buf: &[u8]) -> Option<GsLocation> {
    if buf.len() < 3 {
        return None;
    }
    // Upper 12 bits: lat
    let lat_raw = ((buf[0] as i16) << 8 | buf[1] as i16) >> 4;
    // Sign-extend from 12 bits
    let lat_i16 = (lat_raw << 4) >> 4;
    // Lower 12 bits: lon
    let lon_raw = ((buf[1] as i16) << 8 | buf[2] as i16) & 0x0FFF;
    let lon_i16 = (lon_raw << 4) >> 4;
    Some(GsLocation {
        lat: lat_i16 as f32 / 10.0,
        lon: lon_i16 as f32 / 10.0,
    })
}

/// Find the connection management parameter in VDL params, return (h, r) bits.
/// If not found, returns (1, 1) — which makes type GSIF or LPM.
fn find_conn_mgmt(p: &XidVdlParams) -> (u8, u8) {
    // Connection management TLV code = 0x01.
    // It appears as an "other" entry since we don't decode it specially.
    for tlv in &p.other {
        if tlv.typecode == 0x01 && !tlv.data.is_empty() {
            let val = tlv.data[0];
            let h = (val >> 1) & 1;
            let r = val & 1;
            return (h, r);
        }
    }
    (1, 1) // default: forces GSIF/LPM
}
