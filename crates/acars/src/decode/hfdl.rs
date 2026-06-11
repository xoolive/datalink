use crate::decode::acars::{parse_acars_frame, AcarsMessage, MessageDirection};
use crate::decode::DecodeError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FcsResult {
    Pass,
    Fail,
    Incomplete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfdlMessage {
    pub len: usize,
    pub fcs: FcsResult,
    pub pdu: HfdlPdu,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum HfdlPdu {
    Spdu(Spdu),
    Mpdu(Mpdu),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spdu {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ground_station_id: Option<u8>,
    pub version: u8,
    pub rls_in_use: bool,
    pub iso8208_supported: bool,
    pub change_note: String,
    pub tdma_frame_index: u16,
    pub tdma_frame_offset: u8,
    pub min_priority: u8,
    pub system_table_version: u16,
    pub ground_stations: Vec<SpduGroundStation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "direction", content = "data", rename_all = "snake_case")]
pub enum Mpdu {
    Uplink(UplinkMpdu),
    Downlink(DownlinkMpdu),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UplinkMpdu {
    pub src_ground_station_id: u8,
    pub aircraft: Vec<MpduAircraft>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownlinkMpdu {
    pub src_aircraft_id: u8,
    pub dst_ground_station_id: u8,
    pub lpdu_count: usize,
    pub lpdu_lengths: Vec<usize>,
    pub lpdus: Vec<Lpdu>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpduGroundStation {
    pub id: u8,
    pub utc_sync: bool,
    pub frequencies_in_use_mask: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MpduAircraft {
    pub aircraft_id: u8,
    pub lpdu_count: usize,
    pub lpdu_lengths: Vec<usize>,
    pub lpdus: Vec<Lpdu>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lpdu {
    pub index: usize,
    pub len: usize,
    pub fcs: FcsResult,
    pub kind_code: String,
    pub kind_name: String,
    pub data: LpduData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum LpduData {
    Hfnpdu { hfnpdu: Box<HfnpduEnvelope> },
    IcaoReason { icao24: String, reason_code: u8 },
    IcaoAcId { icao24: String, aircraft_id: u8 },
    Icao { icao24: String },
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfnpduEnvelope {
    pub kind_code: String,
    pub kind_name: String,
    pub data: Hfnpdu,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Hfnpdu {
    SystemTablePartial {
        total_pdu_count: u8,
        pdu_sequence: u8,
        system_table_version: u16,
    },
    Performance {
        performance: Box<PerformanceData>,
    },
    SystemTableRequest {
        request_data: u16,
    },
    Frequency {
        frequency_data: Box<FrequencyData>,
    },
    Acars {
        acars: Box<AcarsMessage>,
    },
    Unknown {
        raw_hex: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceData {
    pub version: u8,
    pub flight_id: String,
    pub position: Position,
    pub time_utc: String,
    pub flight_leg: u8,
    pub ground_station_id: u8,
    pub frequency_id: u8,
    pub frequency_search_count: FrequencySearchCount,
    pub hf_data_disabled_duration_sec: DurationSecs,
    pub mpdus_received: MpduStats,
    pub mpdus_received_with_errors: MpduStats,
    pub spdus_received: u16,
    pub spdus_missed: u8,
    pub mpdus_transmitted: MpduStats,
    pub mpdus_delivered: MpduStats,
    pub frequency_change_code: u8,
    pub frequency_change_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrequencyData {
    pub flight_id: String,
    pub position: Position,
    pub time_utc: String,
    pub propagating_frequency_count: usize,
    pub ground_stations: Vec<GroundStationFreqs>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrequencySearchCount {
    pub previous_leg: u16,
    pub current_leg: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurationSecs {
    pub previous_leg: u16,
    pub current_leg: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MpduStats {
    pub bps_300: u8,
    pub bps_600: u8,
    pub bps_1200: u8,
    pub bps_1800: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundStationFreqs {
    pub ground_station_id: u8,
    pub propagating_frequencies_mask: u32,
    pub heard_frequencies_mask: u32,
}

pub fn parse_hfdl_pdu(buf: &[u8]) -> Result<HfdlMessage, DecodeError> {
    if buf.is_empty() {
        return Err(DecodeError::InvalidHfdlFrame("empty PDU".into()));
    }
    if buf[0] & 1 != 0 {
        parse_mpdu(buf)
    } else {
        parse_spdu(buf)
    }
}

fn parse_spdu(buf: &[u8]) -> Result<HfdlMessage, DecodeError> {
    let fcs = hfdl_fcs_ok(buf, 64);
    if buf.len() < 66 {
        return Err(DecodeError::InvalidHfdlFrame(format!(
            "SPDU frame too short: {} bytes",
            buf.len()
        )));
    }

    let spdu = Spdu {
        ground_station_id: buf.get(1).map(|v| v & 0x7f),
        version: (buf[0] >> 2) & 3,
        rls_in_use: buf[0] & 2 != 0,
        iso8208_supported: buf[0] & 0x20 != 0,
        change_note: spdu_change_note((buf[0] & 0xc0) >> 6).into(),
        tdma_frame_index: (buf[2] as u16) | (((buf[3] & 0x0f) as u16) << 8),
        tdma_frame_offset: buf[3] >> 4,
        min_priority: buf[52] & 0x0f,
        system_table_version: (buf[53] as u16) | (((buf[54] & 0x0f) as u16) << 8),
        ground_stations: vec![
            SpduGroundStation {
                id: buf[1] & 0x7f,
                utc_sync: buf[1] & 0x80 != 0,
                frequencies_in_use_mask: ((buf[54] >> 4) as u32)
                    | ((buf[55] as u32) << 4)
                    | ((buf[56] as u32) << 12),
            },
            SpduGroundStation {
                id: buf[57] & 0x7f,
                utc_sync: buf[57] & 0x80 != 0,
                frequencies_in_use_mask: (buf[58] as u32)
                    | ((buf[59] as u32) << 8)
                    | (((buf[60] & 0x0f) as u32) << 16),
            },
            SpduGroundStation {
                id: (buf[60] >> 4) | ((buf[61] & 0x07) << 4),
                utc_sync: buf[61] & 0x08 != 0,
                frequencies_in_use_mask: ((buf[61] >> 4) as u32)
                    | ((buf[62] as u32) << 4)
                    | ((buf[63] as u32) << 12),
            },
        ],
    };

    Ok(HfdlMessage {
        len: buf.len(),
        fcs,
        pdu: HfdlPdu::Spdu(spdu),
    })
}

fn parse_mpdu(buf: &[u8]) -> Result<HfdlMessage, DecodeError> {
    let downlink = buf[0] & 0x02 != 0;
    if downlink {
        parse_downlink_mpdu(buf)
    } else {
        parse_uplink_mpdu(buf)
    }
}

fn parse_downlink_mpdu(buf: &[u8]) -> Result<HfdlMessage, DecodeError> {
    if buf.len() < 8 {
        return Err(DecodeError::InvalidHfdlFrame(format!(
            "downlink MPDU too short: {} bytes",
            buf.len()
        )));
    }
    let lpdu_count = ((buf[0] >> 2) & 0x0f) as usize;
    let header_len = 6 + lpdu_count;
    let fcs = hfdl_fcs_ok(buf, header_len);
    let lpdu_lengths: Vec<usize> = buf
        .get(6..6 + lpdu_count)
        .unwrap_or_default()
        .iter()
        .map(|v| *v as usize + 1)
        .collect();

    if buf.len() < header_len + 2 {
        return Err(DecodeError::InvalidHfdlFrame(
            "downlink MPDU incomplete LPDUs".into(),
        ));
    }

    let data_start = header_len + 2;
    let lpdus = parse_lpdu_list(
        &lpdu_lengths,
        buf.get(data_start..).unwrap_or_default(),
        MessageDirection::AirToGround,
    )?;

    Ok(HfdlMessage {
        len: buf.len(),
        fcs,
        pdu: HfdlPdu::Mpdu(Mpdu::Downlink(DownlinkMpdu {
            src_aircraft_id: buf[2],
            dst_ground_station_id: buf[1] & 0x7f,
            lpdu_count,
            lpdu_lengths,
            lpdus,
        })),
    })
}

fn parse_uplink_mpdu(buf: &[u8]) -> Result<HfdlMessage, DecodeError> {
    if buf.len() < 5 {
        return Err(DecodeError::InvalidHfdlFrame(format!(
            "uplink MPDU too short: {} bytes",
            buf.len()
        )));
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
    let fcs = hfdl_fcs_ok(buf, pos);
    if buf.len() < pos + 2 {
        return Err(DecodeError::InvalidHfdlFrame(
            "uplink MPDU incomplete LPDUs".into(),
        ));
    }

    let mut data = buf.get(pos + 2..).unwrap_or_default();
    let mut aircraft = Vec::new();
    for (aircraft_id, lengths) in aircraft_headers {
        let lpdus = parse_lpdu_list(&lengths, data, MessageDirection::GroundToAir)?;
        let consumed: usize = lengths.iter().sum();
        data = data.get(consumed..).unwrap_or_default();
        aircraft.push(MpduAircraft {
            aircraft_id,
            lpdu_count: lengths.len(),
            lpdu_lengths: lengths,
            lpdus,
        });
    }

    Ok(HfdlMessage {
        len: buf.len(),
        fcs,
        pdu: HfdlPdu::Mpdu(Mpdu::Uplink(UplinkMpdu {
            src_ground_station_id: buf[1] & 0x7f,
            aircraft,
        })),
    })
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
) -> Result<Vec<Lpdu>, DecodeError> {
    let mut lpdus = Vec::with_capacity(lengths.len());
    for (idx, &len) in lengths.iter().enumerate() {
        let lpdu = data.get(..len).unwrap_or(data);
        data = data.get(len..).unwrap_or_default();
        lpdus.push(parse_lpdu(idx, lpdu, acars_direction)?);
    }
    Ok(lpdus)
}

fn parse_lpdu(
    index: usize,
    buf: &[u8],
    acars_direction: MessageDirection,
) -> Result<Lpdu, DecodeError> {
    if buf.len() < 3 {
        return Err(DecodeError::InvalidHfdlFrame("LPDU too short".into()));
    }
    let body_len = buf.len() - 2;
    let fcs = hfdl_fcs_ok(buf, body_len);
    let body = &buf[..body_len];
    let lpdu_type = body[0];
    let kind_code = format!("0x{lpdu_type:02X}");
    let kind_name = lpdu_type_name(lpdu_type).to_string();

    let data = match lpdu_type {
        0x0D | 0x1D if body.len() > 1 => LpduData::Hfnpdu {
            hfnpdu: Box::new(parse_hfnpdu(&body[1..], acars_direction)?),
        },
        0x2F | 0x3F if body.len() >= 5 => LpduData::IcaoReason {
            icao24: icao_hex(&body[1..4]),
            reason_code: body[4],
        },
        0x5F | 0x9F if body.len() >= 5 => LpduData::IcaoAcId {
            icao24: icao_hex(&body[1..4]),
            aircraft_id: body[4],
        },
        0x4F | 0x8F | 0xBF if body.len() >= 4 => LpduData::Icao {
            icao24: icao_hex(&body[1..4]),
        },
        _ => LpduData::Unknown,
    };

    Ok(Lpdu {
        index,
        len: buf.len(),
        fcs,
        kind_code,
        kind_name,
        data,
    })
}

fn parse_hfnpdu(
    buf: &[u8],
    acars_direction: MessageDirection,
) -> Result<HfnpduEnvelope, DecodeError> {
    if buf.is_empty() {
        return Err(DecodeError::InvalidHfdlFrame("empty HFNPDU".into()));
    }
    if buf[0] != 0xFF {
        return Err(DecodeError::InvalidHfdlFrame("not an HFNPDU".into()));
    }
    if buf.len() < 2 {
        return Err(DecodeError::InvalidHfdlFrame(format!(
            "HFNPDU too short: {} bytes",
            buf.len()
        )));
    }
    let hfnpdu_type = buf[1];
    let kind_code = format!("0x{hfnpdu_type:02X}");
    let kind_name = hfnpdu_type_name(hfnpdu_type).to_string();

    let data = match hfnpdu_type {
        0xD0 if buf.len() >= 5 => Hfnpdu::SystemTablePartial {
            total_pdu_count: (buf[2] >> 4) + 1,
            pdu_sequence: buf[2] & 0x0f,
            system_table_version: (buf[3] as u16 >> 4) | ((buf[4] as u16) << 4),
        },
        0xD1 if buf.len() >= 47 => Hfnpdu::Performance {
            performance: Box::new(parse_performance_data(buf)),
        },
        0xD2 if buf.len() >= 4 => Hfnpdu::SystemTableRequest {
            request_data: u16::from_le_bytes([buf[2], buf[3]]),
        },
        0xD5 if buf.len() >= 15 => Hfnpdu::Frequency {
            frequency_data: Box::new(parse_frequency_data(buf)),
        },
        0xFF => {
            let acars_bytes = &buf[2..];
            match parse_acars_frame(acars_bytes, acars_direction) {
                Ok(acars) => Hfnpdu::Acars {
                    acars: Box::new(acars),
                },
                Err(_) => Hfnpdu::Unknown {
                    raw_hex: hex::encode_upper(acars_bytes),
                },
            }
        }
        _ => Hfnpdu::Unknown {
            raw_hex: hex::encode_upper(buf),
        },
    };

    Ok(HfnpduEnvelope {
        kind_code,
        kind_name,
        data,
    })
}

fn parse_performance_data(buf: &[u8]) -> PerformanceData {
    let flight_id = ascii_trim(&buf[2..8]);
    let lat_raw = (buf[8] as u32) | ((buf[9] as u32) << 8) | (((buf[10] & 0x0f) as u32) << 16);
    let lon_raw =
        ((buf[10] as u32 & 0xf0) >> 4) | ((buf[11] as u32) << 4) | ((buf[12] as u32) << 12);
    let utc = 2 * u16::from_le_bytes([buf[13], buf[14]]) as u32;
    PerformanceData {
        version: buf[15],
        flight_id,
        position: Position {
            lat: parse_hfdl_coordinate(lat_raw),
            lon: parse_hfdl_coordinate(lon_raw),
        },
        time_utc: format_hms(utc),
        flight_leg: buf[16],
        ground_station_id: buf[17] & 0x7f,
        frequency_id: buf[18],
        frequency_search_count: FrequencySearchCount {
            previous_leg: u16::from_le_bytes([buf[19], buf[20]]),
            current_leg: u16::from_le_bytes([buf[21], buf[22]]),
        },
        hf_data_disabled_duration_sec: DurationSecs {
            previous_leg: u16::from_le_bytes([buf[23], buf[24]]),
            current_leg: u16::from_le_bytes([buf[25], buf[26]]),
        },
        mpdus_received: mpdu_stats(&buf[27..31]),
        mpdus_received_with_errors: mpdu_stats(&buf[31..35]),
        spdus_received: u16::from_le_bytes([buf[35], buf[36]]),
        spdus_missed: buf[37],
        mpdus_transmitted: mpdu_stats(&buf[38..42]),
        mpdus_delivered: mpdu_stats(&buf[42..46]),
        frequency_change_code: buf[46] & 0x0f,
        frequency_change_reason: frequency_change_reason(buf[46] & 0x0f).into(),
    }
}

fn parse_frequency_data(buf: &[u8]) -> FrequencyData {
    let flight_id = ascii_trim(&buf[2..8]);
    let lat_raw = (buf[8] as u32) | ((buf[9] as u32) << 8) | (((buf[10] & 0x0f) as u32) << 16);
    let lon_raw =
        ((buf[10] as u32 & 0xf0) >> 4) | ((buf[11] as u32) << 4) | ((buf[12] as u32) << 12);
    let utc = 2 * u16::from_le_bytes([buf[13], buf[14]]) as u32;
    let mut freqs = Vec::new();
    let mut pos = 15usize;
    while pos + 6 <= buf.len() && freqs.len() < 6 {
        freqs.push(GroundStationFreqs {
            ground_station_id: buf[pos] & 0x7f,
            propagating_frequencies_mask: (buf[pos + 1] as u32)
                | ((buf[pos + 2] as u32) << 8)
                | (((buf[pos + 3] & 0x0f) as u32) << 16),
            heard_frequencies_mask: ((buf[pos + 3] as u32 & 0xf0) >> 4)
                | ((buf[pos + 4] as u32) << 4)
                | ((buf[pos + 5] as u32) << 12),
        });
        pos += 6;
    }
    FrequencyData {
        flight_id,
        position: Position {
            lat: parse_hfdl_coordinate(lat_raw),
            lon: parse_hfdl_coordinate(lon_raw),
        },
        time_utc: format_hms(utc),
        propagating_frequency_count: freqs.len(),
        ground_stations: freqs,
    }
}

fn mpdu_stats(bytes: &[u8]) -> MpduStats {
    MpduStats {
        bps_300: bytes[3],
        bps_600: bytes[2],
        bps_1200: bytes[1],
        bps_1800: bytes[0],
    }
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
