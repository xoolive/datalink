//! X.25 packet decoder with CLNP compressed header, IDRP, and COTP.
//!
//! Handles the subset of the X.25 / ISO 8208 / CLNP / X.224 COTP stack
//! that appears in VDL Mode 2 traffic.

use serde::{Deserialize, Serialize};

const GFI_X25_MOD8: u8 = 0x1;
#[allow(dead_code)]
const X25_DATA: u8 = 0x00;
const X25_RR: u8 = 0x01;
const X25_REJ: u8 = 0x09;

const SN_PROTO_CLNP: u8 = 0x81;
const SN_PROTO_ESIS: u8 = 0x82;
const SN_PROTO_IDRP: u8 = 0x85;

const BISPDU_HDR_LEN: usize = 30;
const BISPDU_TYPE_KEEPALIVE: u8 = 4;

const COTP_TPDU_CR: u8 = 0xE0;
const COTP_TPDU_CC: u8 = 0xD0;
const COTP_TPDU_DR: u8 = 0x80;
const COTP_TPDU_DC: u8 = 0xC0;
const COTP_TPDU_DT: u8 = 0xF0;
const COTP_TPDU_ED: u8 = 0x10;
const COTP_TPDU_AK: u8 = 0x60;
const COTP_TPDU_EA: u8 = 0x20;
const COTP_TPDU_RJ: u8 = 0x50;
const COTP_TPDU_ER: u8 = 0x70;

/// One X.25 packet with decoded header and inner protocol layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct X25Packet {
    pub packet_type: String,
    pub chan_group: u8,
    pub chan_num: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sseq: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rseq: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub more: Option<bool>,
    pub reasm_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inner: Option<X25Inner>,
}

/// Inner protocol payload of an X.25 Data packet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum X25Inner {
    ClnpCompressed(ClnpCompressedPdu),
    #[serde(
        serialize_with = "crate::decode::helpers::serialize_bytes_hex_variant",
        deserialize_with = "crate::decode::helpers::deserialize_bytes_hex"
    )]
    Unknown(Vec<u8>),
}

/// Decoded CLNP compressed-header PDU.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClnpCompressedPdu {
    pub lref: u16,
    pub priority: u8,
    pub flags: u8,
    pub lifetime_sec: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdu_id: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inner: Option<ClnpInner>,
}

/// Inner content of a CLNP data PDU.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClnpInner {
    Idrp(IdrpPdu),
    Cotp(Vec<CotpPdu>),
    #[serde(
        serialize_with = "crate::decode::helpers::serialize_bytes_hex_variant",
        deserialize_with = "crate::decode::helpers::deserialize_bytes_hex"
    )]
    Unknown(Vec<u8>),
}

/// Decoded IDRP PDU.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdrpPdu {
    pub bispdu_type: String,
    pub seq: u32,
    pub ack: u32,
    pub credit_offered: u8,
    pub credit_avail: u8,
}

/// Decoded COTP TPDU.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CotpPdu {
    pub tpdu_type: String,
    pub dst_ref: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src_ref: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sseq: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rseq: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eot: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roa: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disc_reason: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disc_reason_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proto_class: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<CotpVarParam>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "crate::decode::helpers::serialize_opt_bytes_hex")]
    #[serde(deserialize_with = "crate::decode::helpers::deserialize_opt_bytes_hex")]
    pub user_data: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpdlc: Option<CpdlcMessage>,
    /// ATN B1 CPDLC summary if the COTP DT payload decoded as an ATN message.
    /// Uses the same element types as the FANS-1/A decoder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atn_cpdlc: Option<crate::decode::payload::arinc622::cpdlc::CpdlcPduSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpdlcMessage {
    pub parser: String,
    pub bit_offset: u8,
    pub free_text: String,
}

/// One decoded COTP variable-part parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CotpVarParam {
    pub id: u8,
    pub label: String,
    pub value: CotpParamValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CotpParamValue {
    #[serde(
        serialize_with = "crate::decode::helpers::serialize_bytes_hex_variant",
        deserialize_with = "crate::decode::helpers::deserialize_bytes_hex"
    )]
    Bytes(Vec<u8>),
    Uint(u64),
}

/// Parse an X.25 payload (the bytes after the AVLC header, not including FCS).
pub fn parse_x25_packet(buf: &[u8]) -> X25Packet {
    if buf.len() < 3 {
        return X25Packet {
            packet_type: "Unknown".into(),
            chan_group: 0,
            chan_num: 0,
            sseq: None,
            rseq: None,
            more: None,
            reasm_status: "skipped".into(),
            inner: None,
        };
    }

    let gfi = (buf[0] >> 4) & 0x0F;
    let chan_group = buf[0] & 0x0F;
    let chan_num = buf[1];
    let type_byte = buf[2];

    if gfi != GFI_X25_MOD8 {
        return X25Packet {
            packet_type: "Unknown".into(),
            chan_group,
            chan_num,
            sseq: None,
            rseq: None,
            more: None,
            reasm_status: "skipped".into(),
            inner: None,
        };
    }

    let payload = &buf[3..];

    // Determine packet type.
    if (type_byte & 1) == 0 {
        // Data packet
        let sseq = (type_byte >> 1) & 0x7;
        let more = (type_byte >> 4) & 1 != 0;
        let rseq = (type_byte >> 5) & 0x7;
        let inner = if !payload.is_empty() {
            Some(parse_x25_user_data(payload))
        } else {
            None
        };
        X25Packet {
            packet_type: "Data".into(),
            chan_group,
            chan_num,
            sseq: Some(sseq),
            rseq: Some(rseq),
            more: Some(more),
            reasm_status: "skipped".into(),
            inner,
        }
    } else {
        let pkttype = type_byte & 0x1F;
        if pkttype == X25_RR {
            let rseq = (type_byte >> 5) & 0x7;
            X25Packet {
                packet_type: "Receive Ready".into(),
                chan_group,
                chan_num,
                sseq: None,
                rseq: Some(rseq),
                more: None,
                reasm_status: "skipped".into(),
                inner: None,
            }
        } else if pkttype == X25_REJ {
            let rseq = (type_byte >> 5) & 0x7;
            X25Packet {
                packet_type: "Receive Reject".into(),
                chan_group,
                chan_num,
                sseq: None,
                rseq: Some(rseq),
                more: None,
                reasm_status: "skipped".into(),
                inner: None,
            }
        } else {
            X25Packet {
                packet_type: format!("Unknown(0x{:02x})", type_byte),
                chan_group,
                chan_num,
                sseq: None,
                rseq: None,
                more: None,
                reasm_status: "skipped".into(),
                inner: None,
            }
        }
    }
}

fn parse_x25_user_data(buf: &[u8]) -> X25Inner {
    if buf.is_empty() {
        return X25Inner::Unknown(vec![]);
    }
    let proto = buf[0];
    if proto == SN_PROTO_CLNP || proto == SN_PROTO_ESIS {
        // Uncompressed CLNP / ESIS — not decoded
        return X25Inner::Unknown(buf.to_vec());
    }
    let pdu_type = proto >> 4;
    if matches!(pdu_type, 0 | 1 | 2 | 3 | 6 | 7 | 9 | 10) {
        // Compressed CLNP header
        return X25Inner::ClnpCompressed(parse_clnp_compressed(buf));
    }
    X25Inner::Unknown(buf.to_vec())
}

fn parse_clnp_compressed(buf: &[u8]) -> ClnpCompressedPdu {
    const CLNP_MIN: usize = 4;
    if buf.len() < CLNP_MIN {
        return ClnpCompressedPdu {
            lref: 0,
            priority: 0,
            flags: 0,
            lifetime_sec: 0.0,
            pdu_id: None,
            inner: None,
        };
    }

    let pdu_type = buf[0] >> 4;
    let priority = buf[0] & 0x0F;
    let lifetime = buf[1];
    let flags = buf[2];
    let lref_byte3 = buf[3];
    let exp = (lref_byte3 >> 7) & 1;
    let lref_a = lref_byte3 & 0x7F;

    let lifetime_sec = lifetime as f32 / 2.0;

    let is_derived = matches!(pdu_type, 6 | 7 | 9 | 10);
    let is_segmentation_permitted = matches!(pdu_type, 1 | 3) || is_derived;

    let mut pos = 4usize;

    let lref = if exp != 0 {
        if pos >= buf.len() {
            return ClnpCompressedPdu {
                lref: 0,
                priority,
                flags,
                lifetime_sec,
                pdu_id: None,
                inner: None,
            };
        }
        let lref_b = buf[pos];
        pos += 1;
        ((lref_a as u16) << 8) | lref_b as u16
    } else {
        lref_a as u16
    };

    let pdu_id = if is_segmentation_permitted {
        if pos + 2 > buf.len() {
            return ClnpCompressedPdu {
                lref,
                priority,
                flags,
                lifetime_sec,
                pdu_id: None,
                inner: None,
            };
        }
        let id = u16::from_be_bytes([buf[pos], buf[pos + 1]]);
        pos += 2;
        Some(id)
    } else {
        None
    };

    if is_derived {
        // Skip offset (2B) + total_pdu_len (2B)
        pos += 4;
    }

    let inner = if pos < buf.len() {
        Some(parse_clnp_payload(&buf[pos..]))
    } else {
        None
    };

    ClnpCompressedPdu {
        lref,
        priority,
        flags,
        lifetime_sec,
        pdu_id,
        inner,
    }
}

fn parse_clnp_payload(buf: &[u8]) -> ClnpInner {
    if buf.is_empty() {
        return ClnpInner::Unknown(vec![]);
    }
    if buf[0] == SN_PROTO_IDRP {
        if let Some(idrp) = parse_idrp(buf) {
            return ClnpInner::Idrp(idrp);
        }
    }
    // In observed VDL2 ATN traffic, non-IDRP CLNP payloads here are COTP segments.
    let cot = parse_cotp_concatenated(buf);
    if !cot.is_empty() {
        return ClnpInner::Cotp(cot);
    }
    ClnpInner::Unknown(buf.to_vec())
}

fn parse_idrp(buf: &[u8]) -> Option<IdrpPdu> {
    if buf.len() < BISPDU_HDR_LEN {
        return None;
    }
    if buf[0] != SN_PROTO_IDRP {
        return None;
    }
    let bispdu_type = buf[3];
    let seq = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let ack = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);
    let coff = buf[12];
    let cavail = buf[13];

    let type_name = match bispdu_type {
        1 => "Open",
        2 => "Update",
        3 => "Error",
        BISPDU_TYPE_KEEPALIVE => "Keepalive",
        5 => "Cease",
        6 => "RIB Refresh",
        _ => "Unknown",
    };

    Some(IdrpPdu {
        bispdu_type: type_name.to_string(),
        seq,
        ack,
        credit_offered: coff,
        credit_avail: cavail,
    })
}

fn parse_cotp_concatenated(buf: &[u8]) -> Vec<CotpPdu> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < buf.len() {
        if pos + 2 > buf.len() {
            break;
        }
        let li = buf[pos] as usize;
        if li == 0 || li == 255 {
            break;
        }
        if pos + 1 + li > buf.len() {
            break;
        }
        let hdr = &buf[pos + 1..pos + 1 + li];
        if hdr.is_empty() {
            break;
        }
        let after_hdr = pos + 1 + li;
        if let Some((pdu, user_data_end)) = parse_cotp_tpdu(li, hdr, buf, after_hdr) {
            out.push(pdu);
            pos = user_data_end;
        } else {
            break;
        }
    }
    out
}

fn parse_cotp_tpdu(
    li: usize,
    hdr: &[u8],
    full_buf: &[u8],
    after_hdr: usize,
) -> Option<(CotpPdu, usize)> {
    if hdr.is_empty() {
        return None;
    }
    let code_byte = hdr[0];
    let code = match code_byte & 0xF0 {
        COTP_TPDU_CR | COTP_TPDU_CC | COTP_TPDU_AK | COTP_TPDU_RJ => code_byte & 0xF0,
        COTP_TPDU_DT => code_byte & 0xFE,
        _ => code_byte,
    };
    let credit_nibble = code_byte & 0x0F;
    let roa = if code_byte & 0xF0 == COTP_TPDU_DT {
        code_byte & 1
    } else {
        0
    };

    if hdr.len() < 3 {
        return None;
    }
    let dst_ref = u16::from_be_bytes([hdr[1], hdr[2]]);

    let type_name = cotp_tpdu_name(code);

    let mut pdu = CotpPdu {
        tpdu_type: type_name.to_string(),
        dst_ref,
        src_ref: None,
        sseq: None,
        rseq: None,
        eot: None,
        roa: None,
        credit: None,
        disc_reason: None,
        disc_reason_name: None,
        proto_class: None,
        options: None,
        params: Vec::new(),
        user_data: None,
        cpdlc: None,
        atn_cpdlc: None,
    };

    let mut var_part_offset: usize = 0;
    let mut final_pdu = false;

    match code {
        COTP_TPDU_CR | COTP_TPDU_CC | COTP_TPDU_DR => {
            if hdr.len() < 6 {
                return None;
            }
            pdu.src_ref = Some(u16::from_be_bytes([hdr[3], hdr[4]]));
            if code == COTP_TPDU_DR {
                pdu.disc_reason = Some(hdr[5]);
                pdu.disc_reason_name = Some(cotp_dr_reason(hdr[5]).to_string());
            } else {
                pdu.proto_class = Some(hdr[5] >> 4);
                pdu.options = Some(hdr[5] & 0x0F);
                pdu.credit = Some(credit_nibble as u16);
            }
            var_part_offset = 6;
            final_pdu = true;
        }
        COTP_TPDU_DC => {
            if hdr.len() < 5 {
                return None;
            }
            pdu.src_ref = Some(u16::from_be_bytes([hdr[3], hdr[4]]));
            var_part_offset = 5;
        }
        COTP_TPDU_DT | COTP_TPDU_ED => {
            if hdr.len() < 4 {
                return None;
            }
            let is_extended = (li & 1) != 0;
            if is_extended {
                if hdr.len() < 7 {
                    return None;
                }
                let seq = u32::from_be_bytes([hdr[3], hdr[4], hdr[5], hdr[6]]) & 0x7FFF_FFFF;
                pdu.eot = Some((hdr[3] & 0x80) != 0);
                pdu.sseq = Some(seq);
                var_part_offset = 7;
            } else {
                pdu.eot = Some((hdr[3] & 0x80) != 0);
                pdu.sseq = Some((hdr[3] & 0x7F) as u32);
                var_part_offset = 4;
            }
            pdu.roa = Some(roa);
            final_pdu = true;
        }
        COTP_TPDU_AK => {
            let is_extended = (li & 1) != 0;
            if is_extended {
                if hdr.len() < 9 {
                    return None;
                }
                let seq = u32::from_be_bytes([hdr[3], hdr[4], hdr[5], hdr[6]]) & 0x7FFF_FFFF;
                pdu.rseq = Some(seq);
                pdu.credit = Some(u16::from_be_bytes([hdr[7], hdr[8]]));
                var_part_offset = 9;
            } else {
                if hdr.len() < 4 {
                    return None;
                }
                pdu.rseq = Some((hdr[3] & 0x7F) as u32);
                pdu.credit = Some(credit_nibble as u16);
                var_part_offset = 4;
            }
        }
        COTP_TPDU_EA => {
            let is_extended = (li & 1) != 0;
            if is_extended {
                if hdr.len() < 7 {
                    return None;
                }
                let seq = u32::from_be_bytes([hdr[3], hdr[4], hdr[5], hdr[6]]) & 0x7FFF_FFFF;
                pdu.rseq = Some(seq);
                var_part_offset = 7;
            } else {
                if hdr.len() < 4 {
                    return None;
                }
                pdu.rseq = Some((hdr[3] & 0x7F) as u32);
                var_part_offset = 4;
            }
        }
        COTP_TPDU_RJ => {
            let is_extended = (li & 1) != 0;
            if is_extended {
                if hdr.len() < 9 {
                    return None;
                }
                let seq = u32::from_be_bytes([hdr[3], hdr[4], hdr[5], hdr[6]]) & 0x7FFF_FFFF;
                pdu.rseq = Some(seq);
                pdu.credit = Some(u16::from_be_bytes([hdr[7], hdr[8]]));
            } else {
                if hdr.len() < 4 {
                    return None;
                }
                pdu.rseq = Some((hdr[3] & 0x7F) as u32);
            }
        }
        COTP_TPDU_ER => {
            if hdr.len() < 4 {
                return None;
            }
            // reject cause at hdr[3]
            var_part_offset = 4;
        }
        _ => return None,
    }

    // Parse variable part TLVs if any.
    if var_part_offset > 0 && li > var_part_offset {
        let vp = &hdr[var_part_offset..li];
        pdu.params = parse_cotp_var_params(vp);
    }

    // User data (only for DT, ED, CR, CC, DR with payload).
    let mut user_data_end = after_hdr;
    if final_pdu && after_hdr < full_buf.len() {
        let user_data = &full_buf[after_hdr..];
        if !user_data.is_empty() {
            pdu.user_data = Some(user_data.to_vec());
            if code == COTP_TPDU_DT {
                pdu.cpdlc = parse_cpdlc_user_data(user_data);
                #[cfg(feature = "rasn")]
                {
                    pdu.atn_cpdlc = crate::decode::payload::atn_b1::decode_cotp_user_data(
                        user_data,
                    )
                    .map(|pdu| match pdu {
                        crate::decode::payload::atn_b1::AtnB1Pdu::Cpdlc(summary) => summary,
                    });
                }
            }
        }
        user_data_end = full_buf.len();
    }

    Some((pdu, user_data_end))
}

/// Parse COTP variable-part TLVs (id: 1B, len: 1B, data: len bytes).
fn parse_cotp_var_params(buf: &[u8]) -> Vec<CotpVarParam> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 2 <= buf.len() {
        let id = buf[pos];
        let len = buf[pos + 1] as usize;
        pos += 2;
        if pos + len > buf.len() {
            break;
        }
        let data = &buf[pos..pos + len];
        let (label, value) = decode_cotp_param(id, data);
        out.push(CotpVarParam {
            id,
            label: label.to_string(),
            value,
        });
        pos += len;
    }
    out
}

fn decode_cotp_param(id: u8, data: &[u8]) -> (&'static str, CotpParamValue) {
    match id {
        0x08 => ("atn_checksum", CotpParamValue::Bytes(data.to_vec())),
        0x85 => ("ack_time_ms", read_u16(data)),
        0x87 => ("priority", read_u16(data)),
        0x8a => ("subseq_num", read_u16(data)),
        0x8b => ("reassignment_time_sec", read_u16(data)),
        0x8c => ("flow_control", CotpParamValue::Bytes(data.to_vec())),
        0xc0 => ("tpdu_size_bytes", decode_tpdu_size(data)),
        0xc1 => (
            "calling_transport_selector",
            CotpParamValue::Bytes(data.to_vec()),
        ),
        0xc2 => (
            "called_responding_transport_selector",
            CotpParamValue::Bytes(data.to_vec()),
        ),
        0xc3 => ("checksum", CotpParamValue::Bytes(data.to_vec())),
        0xc4 => ("additional_options", read_uint(data)),
        0xf2 => ("inactivity_timer_ms", read_u32(data)),
        _ => ("unknown", CotpParamValue::Bytes(data.to_vec())),
    }
}

fn decode_tpdu_size(data: &[u8]) -> CotpParamValue {
    if data.len() == 1 {
        CotpParamValue::Uint(1u64 << data[0])
    } else {
        CotpParamValue::Bytes(data.to_vec())
    }
}

fn read_u16(data: &[u8]) -> CotpParamValue {
    if data.len() >= 2 {
        CotpParamValue::Uint(u16::from_be_bytes([data[0], data[1]]) as u64)
    } else {
        CotpParamValue::Bytes(data.to_vec())
    }
}

fn read_u32(data: &[u8]) -> CotpParamValue {
    if data.len() >= 4 {
        CotpParamValue::Uint(u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as u64)
    } else {
        CotpParamValue::Bytes(data.to_vec())
    }
}

fn read_uint(data: &[u8]) -> CotpParamValue {
    match data.len() {
        1 => CotpParamValue::Uint(data[0] as u64),
        2 => read_u16(data),
        4 => read_u32(data),
        _ => CotpParamValue::Bytes(data.to_vec()),
    }
}

fn parse_cpdlc_user_data(data: &[u8]) -> Option<CpdlcMessage> {
    let mut best: Option<(u8, String)> = None;

    for bit_offset in 0..7u8 {
        let decoded = decode_7bit(data, bit_offset);
        if let Some(text) = longest_cpdlc_text_run(&decoded) {
            match &best {
                Some((_, cur)) if cur.len() >= text.len() => {}
                _ => best = Some((bit_offset, text)),
            }
        }
    }

    best.map(|(bit_offset, free_text)| CpdlcMessage {
        parser: "heuristic_7bit".to_string(),
        bit_offset,
        free_text,
    })
}

fn decode_7bit(data: &[u8], bit_offset: u8) -> String {
    let total_bits = data.len() * 8;
    let mut out = String::new();
    let mut bit = bit_offset as usize;

    while bit + 7 <= total_bits {
        let mut value: u8 = 0;
        for i in 0..7 {
            let absolute = bit + i;
            let byte_idx = absolute / 8;
            let bit_in_byte = 7 - (absolute % 8);
            let b = (data[byte_idx] >> bit_in_byte) & 1;
            value = (value << 1) | b;
        }
        let c = if value.is_ascii_graphic() || value == b' ' {
            value as char
        } else {
            '.'
        };
        out.push(c);
        bit += 7;
    }

    out
}

fn longest_cpdlc_text_run(decoded: &str) -> Option<String> {
    fn allowed(c: char) -> bool {
        c.is_ascii_uppercase()
            || c.is_ascii_digit()
            || matches!(c, ' ' | ',' | '.' | '/' | '-' | '\'' | ':' | '(' | ')')
    }

    let mut best = String::new();
    let mut cur = String::new();

    for c in decoded.chars() {
        if allowed(c) {
            cur.push(c);
        } else {
            let candidate = cur.trim_matches('.').trim();
            if candidate.len() > best.len() && candidate.len() >= 16 && candidate.contains(' ') {
                best = candidate.to_string();
            }
            cur.clear();
        }
    }

    let candidate = cur.trim_matches('.').trim();
    if candidate.len() > best.len() && candidate.len() >= 16 && candidate.contains(' ') {
        best = candidate.to_string();
    }

    if best.is_empty() {
        return None;
    }

    let cleaned = if let Some(idx) = best.find("..") {
        best[..idx].trim().to_string()
    } else {
        best
    };

    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn cotp_tpdu_name(code: u8) -> &'static str {
    match code {
        COTP_TPDU_CR => "Connect Request",
        COTP_TPDU_CC => "Connect Confirm",
        COTP_TPDU_DR => "Disconnect Request",
        COTP_TPDU_DC => "Disconnect Confirm",
        COTP_TPDU_DT => "Data",
        COTP_TPDU_ED => "Expedited Data",
        COTP_TPDU_AK => "Data Ack",
        COTP_TPDU_EA => "Expedited Data Ack",
        COTP_TPDU_RJ => "Reject",
        COTP_TPDU_ER => "Error",
        _ => "Unknown",
    }
}

fn cotp_dr_reason(code: u8) -> &'static str {
    match code {
        0 => "Reason not specified",
        1 => "TSAP congestion",
        2 => "Session entity not attached to TSAP",
        3 => "Unknown address",
        128 => "Normal disconnect",
        129 => "Remote transport entity congestion",
        130 => "Connection negotiation failed",
        131 => "Duplicate source reference",
        132 => "Mismatched references",
        133 => "Protocol error",
        135 => "Reference overflow",
        136 => "Connection request refused",
        138 => "Header or parameter length invalid",
        _ => "Unknown",
    }
}
