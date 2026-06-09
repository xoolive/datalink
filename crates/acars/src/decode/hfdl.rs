use crate::decode::{
    acars::{parse_acars_frame, MessageDirection},
    compact::compact_value,
};
use serde_json::json;

pub fn parse_hfdl_pdu(buf: &[u8]) -> serde_json::Value {
    if buf.is_empty() {
        return json!({ "bearer": "hfdl", "parse_ok": false, "error": "empty PDU" });
    }
    if buf[0] & 1 != 0 {
        parse_mpdu(buf)
    } else {
        parse_spdu(buf)
    }
}

fn parse_spdu(buf: &[u8]) -> serde_json::Value {
    let fcs_ok = hfdl_fcs_ok(buf, 64);
    let mut out = json!({
        "bearer": "hfdl",
        "pdu": "spdu",
        "parse_ok": buf.len() >= 66,
        "fcs_ok": fcs_ok.as_json(),
        "len": buf.len(),
        "ground_station_id": buf.get(1).map(|v| v & 0x7f),
    });
    if buf.len() >= 66 {
        let obj = out.as_object_mut().unwrap();
        obj.insert("version".into(), ((buf[0] >> 2) & 3).into());
        obj.insert("rls_in_use".into(), (buf[0] & 2 != 0).into());
        obj.insert("iso8208_supported".into(), (buf[0] & 0x20 != 0).into());
        obj.insert(
            "change_note".into(),
            spdu_change_note((buf[0] & 0xc0) >> 6).into(),
        );
        obj.insert(
            "tdma_frame_index".into(),
            ((buf[2] as u16) | (((buf[3] & 0x0f) as u16) << 8)).into(),
        );
        obj.insert("tdma_frame_offset".into(), (buf[3] >> 4).into());
        obj.insert("min_priority".into(), (buf[52] & 0x0f).into());
        obj.insert(
            "system_table_version".into(),
            ((buf[53] as u16) | (((buf[54] & 0x0f) as u16) << 8)).into(),
        );
        obj.insert("ground_stations".into(), json!([
            {"id": buf[1] & 0x7f, "utc_sync": buf[1] & 0x80 != 0, "frequencies_in_use_mask": ((buf[54] >> 4) as u32) | ((buf[55] as u32) << 4) | ((buf[56] as u32) << 12)},
            {"id": buf[57] & 0x7f, "utc_sync": buf[57] & 0x80 != 0, "frequencies_in_use_mask": (buf[58] as u32) | ((buf[59] as u32) << 8) | (((buf[60] & 0x0f) as u32) << 16)},
            {"id": (buf[60] >> 4) | ((buf[61] & 0x07) << 4), "utc_sync": buf[61] & 0x08 != 0, "frequencies_in_use_mask": ((buf[61] >> 4) as u32) | ((buf[62] as u32) << 4) | ((buf[63] as u32) << 12)}
        ]));
    }
    out
}

fn parse_mpdu(buf: &[u8]) -> serde_json::Value {
    let downlink = buf[0] & 0x02 != 0;
    if downlink {
        parse_downlink_mpdu(buf)
    } else {
        parse_uplink_mpdu(buf)
    }
}

fn parse_downlink_mpdu(buf: &[u8]) -> serde_json::Value {
    if buf.len() < 8 {
        return json!({ "bearer": "hfdl", "pdu": "mpdu", "direction": "downlink", "parse_ok": false, "error": "too short" });
    }
    let lpdu_count = ((buf[0] >> 2) & 0x0f) as usize;
    let header_len = 6 + lpdu_count;
    let fcs_ok = hfdl_fcs_ok(buf, header_len);
    let lpdu_lengths: Vec<usize> = buf
        .get(6..6 + lpdu_count)
        .unwrap_or_default()
        .iter()
        .map(|v| *v as usize + 1)
        .collect();
    let parse_ok = buf.len() >= header_len + 2;
    let lpdus = if parse_ok {
        let data_start = header_len + 2;
        parse_lpdu_list(
            &lpdu_lengths,
            buf.get(data_start..).unwrap_or_default(),
            MessageDirection::AirToGround,
        )
    } else {
        Vec::new()
    };
    json!({
        "bearer": "hfdl",
        "pdu": "mpdu",
        "direction": "downlink",
        "parse_ok": parse_ok,
        "fcs_ok": fcs_ok.as_json(),
        "len": buf.len(),
        "src_aircraft_id": buf[2],
        "dst_ground_station_id": buf[1] & 0x7f,
        "lpdu_count": lpdu_count,
        "lpdu_lengths": lpdu_lengths,
        "lpdus": lpdus,
    })
}

fn parse_uplink_mpdu(buf: &[u8]) -> serde_json::Value {
    if buf.len() < 5 {
        return json!({ "bearer": "hfdl", "pdu": "mpdu", "direction": "uplink", "parse_ok": false, "error": "too short" });
    }
    let aircraft_count = (((buf[0] & 0x70) >> 4) + 1) as usize;
    let mut pos = 2usize;
    let mut aircraft_headers = Vec::new();
    for _ in 0..aircraft_count {
        if pos + 2 > buf.len() {
            break;
        }
        let aircraft_id = buf[pos];
        let lpdu_count = (buf[pos + 1] >> 4) as usize;
        pos += 2;
        let lengths: Vec<usize> = buf
            .get(pos..pos + lpdu_count)
            .unwrap_or_default()
            .iter()
            .map(|v| *v as usize + 1)
            .collect();
        pos += lpdu_count;
        aircraft_headers.push((aircraft_id, lengths));
    }
    let fcs_ok = hfdl_fcs_ok(buf, pos);
    let parse_ok = buf.len() >= pos + 2;
    let mut data = buf.get(pos + 2..).unwrap_or_default();
    let aircraft: Vec<_> = aircraft_headers
        .into_iter()
        .map(|(aircraft_id, lengths)| {
            let lpdus = parse_lpdu_list(&lengths, data, MessageDirection::GroundToAir);
            let consumed: usize = lengths.iter().sum();
            data = data.get(consumed..).unwrap_or_default();
            json!({
                "aircraft_id": aircraft_id,
                "lpdu_count": lengths.len(),
                "lpdu_lengths": lengths,
                "lpdus": lpdus,
            })
        })
        .collect();
    json!({
        "bearer": "hfdl",
        "pdu": "mpdu",
        "direction": "uplink",
        "parse_ok": parse_ok,
        "fcs_ok": fcs_ok.as_json(),
        "len": buf.len(),
        "src_ground_station_id": buf[1] & 0x7f,
        "aircraft": aircraft,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FcsResult {
    Pass,
    Fail,
    Incomplete,
}

impl FcsResult {
    fn as_json(self) -> serde_json::Value {
        match self {
            Self::Pass => true.into(),
            Self::Fail => false.into(),
            Self::Incomplete => serde_json::Value::Null,
        }
    }
}

fn hfdl_fcs_ok(buf: &[u8], header_len: usize) -> FcsResult {
    if buf.len() < header_len + 2 {
        return FcsResult::Incomplete;
    }
    let got = u16::from_le_bytes([buf[header_len], buf[header_len + 1]]);
    let expected = hfdl_fcs(&buf[..header_len]);
    if got == expected {
        FcsResult::Pass
    } else {
        FcsResult::Fail
    }
}

fn hfdl_fcs(data: &[u8]) -> u16 {
    crc16_ccitt_reflected(data, 0xffff) ^ 0xffff
}

fn parse_lpdu_list(
    lengths: &[usize],
    mut data: &[u8],
    acars_direction: MessageDirection,
) -> Vec<serde_json::Value> {
    lengths
        .iter()
        .enumerate()
        .map(|(idx, len)| {
            let lpdu = data.get(..*len).unwrap_or(data);
            data = data.get(*len..).unwrap_or_default();
            parse_lpdu(idx, lpdu, acars_direction)
        })
        .collect()
}

fn parse_lpdu(index: usize, buf: &[u8], acars_direction: MessageDirection) -> serde_json::Value {
    if buf.len() < 3 {
        return json!({
            "index": index,
            "parse_ok": false,
            "error": "too short",
            "len": buf.len(),
        });
    }
    let body_len = buf.len() - 2;
    let fcs_ok = hfdl_fcs_ok(buf, body_len);
    let body = &buf[..body_len];
    let lpdu_type = body[0];
    let mut out = json!({
        "index": index,
        "parse_ok": true,
        "fcs_ok": fcs_ok.as_json(),
        "len": buf.len(),
        "type": format!("0x{lpdu_type:02X}"),
        "type_name": lpdu_type_name(lpdu_type),
    });

    if let Some(obj) = out.as_object_mut() {
        match lpdu_type {
            0x0D | 0x1D if body.len() > 1 => {
                obj.insert("hfnpdu".into(), parse_hfnpdu(&body[1..], acars_direction));
            }
            0x2F | 0x3F if body.len() >= 5 => {
                obj.insert("icao24".into(), icao_hex(&body[1..4]).into());
                obj.insert("reason_code".into(), body[4].into());
            }
            0x5F | 0x9F if body.len() >= 5 => {
                obj.insert("icao24".into(), icao_hex(&body[1..4]).into());
                obj.insert("aircraft_id".into(), body[4].into());
            }
            0x4F | 0x8F | 0xBF if body.len() >= 4 => {
                obj.insert("icao24".into(), icao_hex(&body[1..4]).into());
            }
            _ => {}
        }
    }
    out
}

fn parse_hfnpdu(buf: &[u8], acars_direction: MessageDirection) -> serde_json::Value {
    if buf.is_empty() {
        return json!({ "parse_ok": false, "error": "empty HFNPDU" });
    }
    if buf[0] != 0xFF {
        return json!({
            "parse_ok": false,
            "error": "not an HFNPDU",
            "raw_hex": hex::encode_upper(buf),
        });
    }
    if buf.len() < 2 {
        return json!({ "parse_ok": false, "error": "too short", "raw_hex": hex::encode_upper(buf) });
    }
    let hfnpdu_type = buf[1];
    let mut out = json!({
        "parse_ok": true,
        "type": format!("0x{hfnpdu_type:02X}"),
        "type_name": hfnpdu_type_name(hfnpdu_type),
    });
    if let Some(obj) = out.as_object_mut() {
        match hfnpdu_type {
            0xD0 if buf.len() >= 5 => {
                obj.insert("total_pdu_count".into(), ((buf[2] >> 4) + 1).into());
                obj.insert("pdu_sequence".into(), (buf[2] & 0x0f).into());
                obj.insert(
                    "system_table_version".into(),
                    ((buf[3] as u16 >> 4) | ((buf[4] as u16) << 4)).into(),
                );
            }
            0xD1 if buf.len() >= 47 => {
                obj.insert("performance".into(), parse_performance_data(buf));
            }
            0xD2 if buf.len() >= 4 => {
                obj.insert(
                    "request_data".into(),
                    u16::from_le_bytes([buf[2], buf[3]]).into(),
                );
            }
            0xD5 if buf.len() >= 15 => {
                obj.insert("frequency_data".into(), parse_frequency_data(buf));
            }
            0xFF => {
                let acars_bytes = &buf[2..];
                match parse_acars_frame(acars_bytes, acars_direction) {
                    Ok(msg) => {
                        let raw = serde_json::to_value(&msg)
                            .unwrap_or_else(|e| json!({ "serialize_error": e.to_string() }));
                        obj.insert("acars".into(), compact_value(raw, false));
                    }
                    Err(err) => {
                        obj.insert(
                            "acars".into(),
                            json!({
                                "parse_ok": false,
                                "error": err.to_string(),
                                "raw_hex": hex::encode_upper(acars_bytes),
                            }),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn parse_performance_data(buf: &[u8]) -> serde_json::Value {
    let flight_id = ascii_trim(&buf[2..8]);
    let lat_raw = (buf[8] as u32) | ((buf[9] as u32) << 8) | (((buf[10] & 0x0f) as u32) << 16);
    let lon_raw =
        ((buf[10] as u32 & 0xf0) >> 4) | ((buf[11] as u32) << 4) | ((buf[12] as u32) << 12);
    let utc = 2 * u16::from_le_bytes([buf[13], buf[14]]) as u32;
    json!({
        "version": buf[15],
        "flight_id": flight_id,
        "position": { "lat": parse_hfdl_coordinate(lat_raw), "lon": parse_hfdl_coordinate(lon_raw) },
        "time_utc": format_hms(utc),
        "flight_leg": buf[16],
        "ground_station_id": buf[17] & 0x7f,
        "frequency_id": buf[18],
        "frequency_search_count": {
            "previous_leg": u16::from_le_bytes([buf[19], buf[20]]),
            "current_leg": u16::from_le_bytes([buf[21], buf[22]]),
        },
        "hf_data_disabled_duration_sec": {
            "previous_leg": u16::from_le_bytes([buf[23], buf[24]]),
            "current_leg": u16::from_le_bytes([buf[25], buf[26]]),
        },
        "mpdus_received": mpdu_stats(&buf[27..31]),
        "mpdus_received_with_errors": mpdu_stats(&buf[31..35]),
        "spdus_received": u16::from_le_bytes([buf[35], buf[36]]),
        "spdus_missed": buf[37],
        "mpdus_transmitted": mpdu_stats(&buf[38..42]),
        "mpdus_delivered": mpdu_stats(&buf[42..46]),
        "frequency_change_code": buf[46] & 0x0f,
        "frequency_change_reason": frequency_change_reason(buf[46] & 0x0f),
    })
}

fn parse_frequency_data(buf: &[u8]) -> serde_json::Value {
    let flight_id = ascii_trim(&buf[2..8]);
    let lat_raw = (buf[8] as u32) | ((buf[9] as u32) << 8) | (((buf[10] & 0x0f) as u32) << 16);
    let lon_raw =
        ((buf[10] as u32 & 0xf0) >> 4) | ((buf[11] as u32) << 4) | ((buf[12] as u32) << 12);
    let utc = 2 * u16::from_le_bytes([buf[13], buf[14]]) as u32;
    let mut freqs = Vec::new();
    let mut pos = 15usize;
    while pos + 6 <= buf.len() && freqs.len() < 6 {
        freqs.push(json!({
            "ground_station_id": buf[pos] & 0x7f,
            "propagating_frequencies_mask": (buf[pos + 1] as u32) | ((buf[pos + 2] as u32) << 8) | (((buf[pos + 3] & 0x0f) as u32) << 16),
            "heard_frequencies_mask": ((buf[pos + 3] as u32 & 0xf0) >> 4) | ((buf[pos + 4] as u32) << 4) | ((buf[pos + 5] as u32) << 12),
        }));
        pos += 6;
    }
    json!({
        "flight_id": flight_id,
        "position": { "lat": parse_hfdl_coordinate(lat_raw), "lon": parse_hfdl_coordinate(lon_raw) },
        "time_utc": format_hms(utc),
        "propagating_frequency_count": freqs.len(),
        "ground_stations": freqs,
    })
}

fn mpdu_stats(bytes: &[u8]) -> serde_json::Value {
    json!({ "300bps": bytes[3], "600bps": bytes[2], "1200bps": bytes[1], "1800bps": bytes[0] })
}

fn parse_hfdl_coordinate(raw: u32) -> f64 {
    let signed = if raw & (1 << 19) != 0 {
        raw as i32 - (1 << 20)
    } else {
        raw as i32
    };
    signed as f64 * 180.0 / 0x7ffff as f64
}

fn format_hms(total_seconds: u32) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        total_seconds / 3600,
        (total_seconds % 3600) / 60,
        total_seconds % 60
    )
}

fn ascii_trim(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_matches(|c| c == '\0' || c == ' ')
        .to_string()
}

fn frequency_change_reason(code: u8) -> &'static str {
    match code {
        0 => "First frequency search in this flight leg",
        1 => "Too many NACKs",
        2 => "SPDUs no longer received",
        3 => "HFDL disabled",
        4 => "Ground station frequency change",
        5 => "Ground station down / channel down",
        6 => "Poor uplink channel quality",
        7 => "No change",
        _ => "Unknown",
    }
}

fn spdu_change_note(code: u8) -> &'static str {
    match code {
        0 => "None",
        1 => "Channel down",
        2 => "Upcoming frequency change",
        3 => "Ground station down",
        _ => "Unknown",
    }
}

fn lpdu_type_name(typ: u8) -> &'static str {
    match typ {
        0x0D => "Unnumbered data",
        0x1D => "Unnumbered acked data",
        0x2F => "Logon denied",
        0x3F => "Logoff request",
        0x4F => "Logon resume",
        0x5F => "Logon resume confirm",
        0x8F => "Logon request normal",
        0x9F => "Logon confirm",
        0xBF => "Logon request DLS",
        _ => "Unknown",
    }
}

fn hfnpdu_type_name(typ: u8) -> &'static str {
    match typ {
        0xD0 => "System table partial",
        0xD1 => "Performance data",
        0xD2 => "System table request",
        0xD5 => "Frequency data",
        0xDE => "Delayed echo",
        0xFF => "Enveloped data",
        _ => "Unknown",
    }
}

fn icao_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

fn crc16_ccitt_reflected(data: &[u8], init: u16) -> u16 {
    let mut crc = init;
    for &byte in data {
        crc ^= byte as u16;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0x8408;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}
