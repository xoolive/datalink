use deku::prelude::*;
use serde::{Deserialize, Serialize};

use crate::decode::payload::PayloadError;
use crate::decode::{DecodeError, DecodeResult};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdscMessage {
    /// Ground station address that requested this ADS-C report.
    pub atsu_address: String,
    /// Aircraft registration (e.g. `"A7-ANR"`).
    pub registration: String,
    /// Decoded ADS-C tag list.
    pub tags: Vec<AdscTag>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AdscTag {
    Acknowledgement { contract_number: u8 },
    NegativeAcknowledgement(AdscNegativeAcknowledgement),
    NoncomplianceNotification(AdscNoncomplianceNotification),
    CancelEmergencyMode,
    BasicReport(AdscBasicReport),
    EmergencyBasicReport(AdscBasicReport),
    LateralDeviationChangeEvent(AdscBasicReport),
    FlightId(AdscFlightId),
    PredictedRoute(AdscPredictedRoute),
    EarthReferenceData(AdscEarthAirReference),
    AirReferenceData(AdscEarthAirReference),
    MeteoData(AdscMeteo),
    AirframeId(AdscAirframeId),
    VerticalRateChangeEvent(AdscBasicReport),
    AltitudeRangeEvent(AdscBasicReport),
    WaypointChangeEvent(AdscBasicReport),
    IntermediateProjection(AdscIntermediateProjection),
    FixedProjection(AdscFixedProjection),
}

impl AdscTag {
    pub fn id(&self) -> u8 {
        match self {
            Self::Acknowledgement { .. } => 3,
            Self::NegativeAcknowledgement(_) => 4,
            Self::NoncomplianceNotification(_) => 5,
            Self::CancelEmergencyMode => 6,
            Self::BasicReport(_) => 7,
            Self::EmergencyBasicReport(_) => 9,
            Self::LateralDeviationChangeEvent(_) => 10,
            Self::FlightId(_) => 12,
            Self::PredictedRoute(_) => 13,
            Self::EarthReferenceData(_) => 14,
            Self::AirReferenceData(_) => 15,
            Self::MeteoData(_) => 16,
            Self::AirframeId(_) => 17,
            Self::VerticalRateChangeEvent(_) => 18,
            Self::AltitudeRangeEvent(_) => 19,
            Self::WaypointChangeEvent(_) => 20,
            Self::IntermediateProjection(_) => 22,
            Self::FixedProjection(_) => 23,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdscNegativeAcknowledgement {
    pub contract_request_number: u8,
    pub reason: u8,
    pub extension: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdscNoncomplianceNotification {
    pub contract_request_number: u8,
    pub groups: Vec<AdscNoncomplianceGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdscNoncomplianceGroup {
    pub noncompliant_tag: u8,
    pub is_unrecognized: bool,
    pub is_whole_group_unavailable: bool,
    pub parameters: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdscBasicReport {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude_ft: i32,
    pub timestamp_seconds_past_hour: f64,
    pub nav_redundancy_ok: bool,
    pub position_accuracy_code: u8,
    pub tcas_ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdscFlightId {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdscPredictedRoute {
    pub next_latitude: f64,
    pub next_longitude: f64,
    pub next_altitude_ft: i32,
    pub next_eta_seconds: u16,
    pub next_next_latitude: f64,
    pub next_next_longitude: f64,
    pub next_next_altitude_ft: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdscEarthAirReference {
    pub heading_or_track_degrees: f64,
    pub heading_invalid: bool,
    pub speed: f64,
    pub vertical_speed_ft_per_min: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdscMeteo {
    pub wind_speed_kt: f64,
    pub wind_direction_true_degrees: f64,
    pub wind_direction_invalid: bool,
    pub temperature_c: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdscAirframeId {
    pub icao_hex: [u8; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdscIntermediateProjection {
    pub distance_nm: f64,
    pub track_degrees: f64,
    pub track_invalid: bool,
    pub altitude_ft: i32,
    pub eta_seconds: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdscFixedProjection {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude_ft: i32,
    pub eta_seconds: u16,
}

#[derive(Debug, DekuRead)]
struct NackRaw {
    contract_request_number: u8,
    reason: u8,
}

#[derive(Debug, DekuRead)]
#[deku(endian = "big")]
struct BasicReportRaw {
    #[deku(bits = 21)]
    latitude_raw: i32,
    #[deku(bits = 21)]
    longitude_raw: i32,
    #[deku(bits = 16)]
    altitude_raw: i16,
    #[deku(bits = 15)]
    timestamp_raw: u16,
    #[deku(bits = 7)]
    status_raw: u8,
}

#[derive(Debug, DekuRead)]
#[deku(endian = "big")]
struct FlightIdRaw {
    #[deku(bits = 6)]
    c1: u8,
    #[deku(bits = 6)]
    c2: u8,
    #[deku(bits = 6)]
    c3: u8,
    #[deku(bits = 6)]
    c4: u8,
    #[deku(bits = 6)]
    c5: u8,
    #[deku(bits = 6)]
    c6: u8,
    #[deku(bits = 6)]
    c7: u8,
    #[deku(bits = 6)]
    c8: u8,
}

#[derive(Debug, DekuRead)]
#[deku(endian = "big")]
struct PredictedRouteRaw {
    #[deku(bits = 21)]
    next_latitude_raw: i32,
    #[deku(bits = 21)]
    next_longitude_raw: i32,
    #[deku(bits = 16)]
    next_altitude_raw: i16,
    #[deku(bits = 14)]
    next_eta_seconds: u16,
    #[deku(bits = 21)]
    next_next_latitude_raw: i32,
    #[deku(bits = 21)]
    next_next_longitude_raw: i32,
    #[deku(bits = 16)]
    next_next_altitude_raw: i16,
}

#[derive(Debug, DekuRead)]
#[deku(endian = "big")]
struct EarthAirRefRaw {
    #[deku(bits = 1)]
    heading_invalid: u8,
    #[deku(bits = 12)]
    heading_raw: i16,
    #[deku(bits = 13)]
    speed_raw: u16,
    #[deku(bits = 12)]
    vertical_speed_raw: i16,
}

#[derive(Debug, DekuRead)]
#[deku(endian = "big")]
struct MeteoRaw {
    #[deku(bits = 9)]
    wind_speed_raw: u16,
    #[deku(bits = 1)]
    wind_direction_invalid: u8,
    #[deku(bits = 9)]
    wind_direction_raw: i16,
    #[deku(bits = 12)]
    temperature_raw: i16,
    #[deku(bits = 1)]
    _spare: u8,
}

#[derive(Debug, DekuRead)]
#[deku(endian = "big")]
struct IntermediateProjectionRaw {
    #[deku(bits = 16)]
    distance_raw: u16,
    #[deku(bits = 1)]
    track_invalid: u8,
    #[deku(bits = 12)]
    track_raw: i16,
    #[deku(bits = 16)]
    altitude_raw: i16,
    #[deku(bits = 14)]
    eta_seconds: u16,
    #[deku(bits = 5)]
    _spare: u8,
}

#[derive(Debug, DekuRead)]
#[deku(endian = "big")]
struct FixedProjectionRaw {
    #[deku(bits = 21)]
    latitude_raw: i32,
    #[deku(bits = 21)]
    longitude_raw: i32,
    #[deku(bits = 16)]
    altitude_raw: i16,
    #[deku(bits = 14)]
    eta_seconds: u16,
}

#[derive(Debug, DekuRead)]
#[deku(endian = "big")]
struct NoncomplianceGroupHeaderRaw {
    noncompliant_tag: u8,
    #[deku(bits = 1)]
    is_unrecognized: u8,
    #[deku(bits = 1)]
    is_whole_group_unavailable: u8,
    #[deku(bits = 2)]
    _reserved: u8,
    #[deku(bits = 4)]
    parameter_count: u8,
}

pub fn parse_adsc_app_text(txt: &str) -> DecodeResult<AdscMessage> {
    let text = txt.trim();
    if !text.starts_with('/') {
        return Err(DecodeError::InvalidPayload(PayloadError::Adsc));
    }

    let marker = ".ADS.";
    let marker_idx = text
        .find(marker)
        .ok_or(DecodeError::InvalidPayload(PayloadError::Adsc))?;
    if marker_idx < 2 {
        return Err(DecodeError::InvalidPayload(PayloadError::Adsc));
    }

    let atsu = &text[1..marker_idx];
    if atsu.is_empty() {
        return Err(DecodeError::InvalidPayload(PayloadError::Adsc));
    }

    let reg_plus_payload = text[marker_idx + marker.len()..].trim_start_matches('.');
    let split_at = find_registration_split(reg_plus_payload)
        .ok_or(DecodeError::InvalidPayload(PayloadError::Adsc))?;
    let registration = reg_plus_payload[..split_at].to_string();
    let payload_hex = reg_plus_payload[split_at..].to_ascii_uppercase();
    if payload_hex.len() < 4 || !payload_hex.len().is_multiple_of(2) {
        return Err(DecodeError::InvalidPayload(PayloadError::Adsc));
    }

    let crc_start = payload_hex.len() - 4;
    let payload_no_crc_hex = payload_hex[..crc_start].to_string();

    let tags = parse_adsc_payload_hex(&payload_no_crc_hex)?;

    Ok(AdscMessage {
        atsu_address: atsu.to_string(),
        registration,
        tags,
    })
}

pub fn parse_adsc_payload_hex(payload_no_crc_hex: &str) -> DecodeResult<Vec<AdscTag>> {
    let bytes = hex::decode(payload_no_crc_hex)
        .map_err(|_| DecodeError::InvalidPayload(PayloadError::Adsc))?;
    parse_adsc_payload_bytes(&bytes)
}

pub fn parse_adsc_payload_bytes(buf: &[u8]) -> DecodeResult<Vec<AdscTag>> {
    let mut idx = 0usize;
    let mut tags = Vec::new();

    while idx < buf.len() {
        let tag = buf[idx];
        idx += 1;

        match tag {
            3 => {
                let data = take(buf, &mut idx, 1)?;
                tags.push(AdscTag::Acknowledgement {
                    contract_number: data[0],
                });
            }
            4 => {
                let data = take(buf, &mut idx, 2)?;
                let (_, raw) =
                    NackRaw::from_bytes((data, 0)).map_err(|e| DecodeError::Deku(e.to_string()))?;
                let extension = if matches!(raw.reason, 1 | 2 | 7) {
                    Some(take(buf, &mut idx, 1)?[0])
                } else {
                    None
                };
                tags.push(AdscTag::NegativeAcknowledgement(
                    AdscNegativeAcknowledgement {
                        contract_request_number: raw.contract_request_number,
                        reason: raw.reason,
                        extension,
                    },
                ));
            }
            5 => {
                let contract_request_number = take(buf, &mut idx, 1)?[0];
                let group_count = take(buf, &mut idx, 1)?[0] as usize;
                let mut groups = Vec::with_capacity(group_count);

                for _ in 0..group_count {
                    let header = take(buf, &mut idx, 2)?;
                    let (_, raw) = NoncomplianceGroupHeaderRaw::from_bytes((header, 0))
                        .map_err(|e| DecodeError::Deku(e.to_string()))?;

                    let is_unrecognized = raw.is_unrecognized != 0;
                    let is_whole_group_unavailable = raw.is_whole_group_unavailable != 0;
                    let mut parameters = Vec::new();

                    if !is_unrecognized && !is_whole_group_unavailable {
                        let parameter_count = raw.parameter_count as usize;
                        let nibble_bytes = parameter_count / 2 + parameter_count % 2;
                        let param_bytes = take(buf, &mut idx, nibble_bytes)?;
                        for i in 0..parameter_count {
                            let value = if i % 2 == 0 {
                                (param_bytes[i / 2] >> 4) & 0x0f
                            } else {
                                param_bytes[i / 2] & 0x0f
                            };
                            parameters.push(value);
                        }
                    }

                    groups.push(AdscNoncomplianceGroup {
                        noncompliant_tag: raw.noncompliant_tag,
                        is_unrecognized,
                        is_whole_group_unavailable,
                        parameters,
                    });
                }

                tags.push(AdscTag::NoncomplianceNotification(
                    AdscNoncomplianceNotification {
                        contract_request_number,
                        groups,
                    },
                ));
            }
            6 => tags.push(AdscTag::CancelEmergencyMode),
            7 | 9 | 10 | 18 | 19 | 20 => {
                let report = decode_basic_report(take(buf, &mut idx, 10)?)?;
                let variant = match tag {
                    7 => AdscTag::BasicReport(report),
                    9 => AdscTag::EmergencyBasicReport(report),
                    10 => AdscTag::LateralDeviationChangeEvent(report),
                    18 => AdscTag::VerticalRateChangeEvent(report),
                    19 => AdscTag::AltitudeRangeEvent(report),
                    20 => AdscTag::WaypointChangeEvent(report),
                    _ => unreachable!(),
                };
                tags.push(variant);
            }
            12 => {
                let (_, raw) = FlightIdRaw::from_bytes((take(buf, &mut idx, 6)?, 0))
                    .map_err(|e| DecodeError::Deku(e.to_string()))?;
                let id: String = [
                    raw.c1, raw.c2, raw.c3, raw.c4, raw.c5, raw.c6, raw.c7, raw.c8,
                ]
                .into_iter()
                .map(decode_iso5_char)
                .collect::<String>()
                .trim_end()
                .to_string();
                tags.push(AdscTag::FlightId(AdscFlightId { id }));
            }
            13 => {
                let (_, raw) = PredictedRouteRaw::from_bytes((take(buf, &mut idx, 17)?, 0))
                    .map_err(|e| DecodeError::Deku(e.to_string()))?;
                tags.push(AdscTag::PredictedRoute(AdscPredictedRoute {
                    next_latitude: decode_coordinate(raw.next_latitude_raw),
                    next_longitude: decode_coordinate(raw.next_longitude_raw),
                    next_altitude_ft: decode_altitude(raw.next_altitude_raw),
                    next_eta_seconds: raw.next_eta_seconds,
                    next_next_latitude: decode_coordinate(raw.next_next_latitude_raw),
                    next_next_longitude: decode_coordinate(raw.next_next_longitude_raw),
                    next_next_altitude_ft: decode_altitude(raw.next_next_altitude_raw),
                }));
            }
            14 | 15 => {
                let (_, raw) = EarthAirRefRaw::from_bytes((take(buf, &mut idx, 5)?, 0))
                    .map_err(|e| DecodeError::Deku(e.to_string()))?;
                let report = AdscEarthAirReference {
                    heading_or_track_degrees: decode_heading(raw.heading_raw),
                    heading_invalid: raw.heading_invalid != 0,
                    speed: decode_speed(raw.speed_raw),
                    vertical_speed_ft_per_min: decode_vertical_speed(raw.vertical_speed_raw),
                };
                if tag == 14 {
                    tags.push(AdscTag::EarthReferenceData(report));
                } else {
                    tags.push(AdscTag::AirReferenceData(report));
                }
            }
            16 => {
                let (_, raw) = MeteoRaw::from_bytes((take(buf, &mut idx, 4)?, 0))
                    .map_err(|e| DecodeError::Deku(e.to_string()))?;
                tags.push(AdscTag::MeteoData(AdscMeteo {
                    wind_speed_kt: decode_speed(raw.wind_speed_raw),
                    wind_direction_true_degrees: decode_wind_direction(raw.wind_direction_raw),
                    wind_direction_invalid: raw.wind_direction_invalid != 0,
                    temperature_c: decode_temperature(raw.temperature_raw),
                }));
            }
            17 => {
                let data = take(buf, &mut idx, 3)?;
                tags.push(AdscTag::AirframeId(AdscAirframeId {
                    icao_hex: [data[0], data[1], data[2]],
                }));
            }
            22 => {
                let (_, raw) = IntermediateProjectionRaw::from_bytes((take(buf, &mut idx, 8)?, 0))
                    .map_err(|e| DecodeError::Deku(e.to_string()))?;
                tags.push(AdscTag::IntermediateProjection(
                    AdscIntermediateProjection {
                        distance_nm: decode_distance(raw.distance_raw),
                        track_degrees: decode_heading(raw.track_raw),
                        track_invalid: raw.track_invalid != 0,
                        altitude_ft: decode_altitude(raw.altitude_raw),
                        eta_seconds: raw.eta_seconds,
                    },
                ));
            }
            23 => {
                let (_, raw) = FixedProjectionRaw::from_bytes((take(buf, &mut idx, 9)?, 0))
                    .map_err(|e| DecodeError::Deku(e.to_string()))?;
                tags.push(AdscTag::FixedProjection(AdscFixedProjection {
                    latitude: decode_coordinate(raw.latitude_raw),
                    longitude: decode_coordinate(raw.longitude_raw),
                    altitude_ft: decode_altitude(raw.altitude_raw),
                    eta_seconds: raw.eta_seconds,
                }));
            }
            _ => return Err(DecodeError::InvalidPayload(PayloadError::Adsc)),
        }
    }

    Ok(tags)
}

fn decode_basic_report(data: &[u8]) -> DecodeResult<AdscBasicReport> {
    let (_, raw) =
        BasicReportRaw::from_bytes((data, 0)).map_err(|e| DecodeError::Deku(e.to_string()))?;
    let status = raw.status_raw;
    Ok(AdscBasicReport {
        latitude: decode_coordinate(raw.latitude_raw),
        longitude: decode_coordinate(raw.longitude_raw),
        altitude_ft: decode_altitude(raw.altitude_raw),
        timestamp_seconds_past_hour: decode_timestamp(raw.timestamp_raw),
        nav_redundancy_ok: (status & 0x01) != 0,
        position_accuracy_code: (status >> 1) & 0x07,
        tcas_ok: ((status >> 4) & 0x01) != 0,
    })
}

fn decode_iso5_char(value: u8) -> char {
    let mut code = value;
    if (code & 0x20) == 0 {
        code += 0x40;
    }
    code as char
}

fn decode_coordinate(value: i32) -> f64 {
    let max = 180.0 - 90.0 / 2f64.powi(19);
    max * (value as f64) / 0x000f_ffff as f64
}

fn decode_altitude(value: i16) -> i32 {
    (value as i32) * 4
}

fn decode_timestamp(value: u16) -> f64 {
    value as f64 * 0.125
}

fn decode_speed(value: u16) -> f64 {
    value as f64 / 2.0
}

fn decode_vertical_speed(value: i16) -> i32 {
    (value as i32) * 16
}

fn decode_distance(value: u16) -> f64 {
    value as f64 / 8.0
}

fn decode_heading(value: i16) -> f64 {
    let max = 180.0 - 90.0 / 2f64.powi(10);
    let mut result = max * (value as f64) / 0x07ff as f64;
    if result < 0.0 {
        result += 360.0;
    }
    result
}

fn decode_wind_direction(value: i16) -> f64 {
    let max = 180.0 - 90.0 / 2f64.powi(7);
    let mut result = max * (value as f64) / 0x00ff as f64;
    if result < 0.0 {
        result += 360.0;
    }
    result
}

fn decode_temperature(value: i16) -> f64 {
    let max = 512.0 - 256.0 / 2f64.powi(10);
    max * (value as f64) / 0x07ff as f64
}

fn take<'a>(buf: &'a [u8], idx: &mut usize, count: usize) -> DecodeResult<&'a [u8]> {
    if *idx + count > buf.len() {
        return Err(DecodeError::InvalidPayload(PayloadError::Adsc));
    }
    let out = &buf[*idx..*idx + count];
    *idx += count;
    Ok(out)
}

fn find_registration_split(value: &str) -> Option<usize> {
    let mut fallback: Option<usize> = None;
    for idx in 0..=value.len() {
        let reg = &value[..idx];
        let payload = &value[idx..];
        if payload.len() >= 4
            && payload.len().is_multiple_of(2)
            && payload.bytes().all(|b| b.is_ascii_hexdigit())
        {
            if fallback.is_none() {
                fallback = Some(idx);
            }
            let reg_len = reg.len();
            if (6..=7).contains(&reg_len)
                && reg
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
            {
                return Some(idx);
            }
        }
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_adsc_sample() {
        let msg = parse_adsc_app_text(
            "/BDOCAYA.ADS.A7-ANR073759D0C997088B86BC1F0D377770C71C488B805B38E698AB9AC88B80A626",
        )
        .unwrap();

        assert_eq!(msg.atsu_address, "BDOCAYA");
        assert_eq!(msg.registration, "A7-ANR");
        assert!(!msg.tags.is_empty());
        assert_eq!(msg.tags[0].id(), 7);
    }
}
