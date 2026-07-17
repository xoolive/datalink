//! ARINC 622 ADS-C application payloads.
//!
//! ADS-C (Automatic Dependent Surveillance — Contract) messages are carried in
//! FANS-1/A ARINC 622 envelopes with IMI `ADS`. Downlink reports from the
//! aircraft include position, altitude, route, meteo, and event tags; uplink
//! messages from the ground request or cancel contracts.
//!
//! The public entry point for standalone application text is
//! [`parse_adsc_app_text`]. Normal ACARS label routing reaches this module via
//! [`super::parse_with_direction`] and [`super::parse_and_dispatch`].

use deku::prelude::*;
use serde::{Deserialize, Serialize};

use crate::decode::payload::PayloadError;
use crate::decode::{DecodeError, DecodeResult};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(transparent)]
pub struct AdscMessage {
    /// Decoded ADS-C tag list, serialized transparently as the ADS-C payload.
    pub tags: Vec<AdscTag>,
}

/// ADS-C disconnect (`DIS`) reason carried in the high nibble of its payload byte.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdscDisconnectReason {
    ReasonNotSpecified,
    Congestion,
    ApplicationNotAvailable,
    NormalDisconnect,
    Unknown,
}

pub fn parse_adsc_disconnect_payload_hex(payload_hex: &str) -> DecodeResult<AdscDisconnectReason> {
    let bytes =
        hex::decode(payload_hex).map_err(|_| DecodeError::InvalidPayload(PayloadError::Adsc))?;
    let [raw] = bytes.as_slice() else {
        return Err(DecodeError::InvalidPayload(PayloadError::Adsc));
    };
    Ok(match raw >> 4 {
        0 => AdscDisconnectReason::ReasonNotSpecified,
        1 => AdscDisconnectReason::Congestion,
        2 => AdscDisconnectReason::ApplicationNotAvailable,
        8 => AdscDisconnectReason::NormalDisconnect,
        _ => AdscDisconnectReason::Unknown,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AdscTag {
    // Downlink tags.
    Acknowledgement { contract_number: u8 },
    NegativeAcknowledgement(AdscNegativeAcknowledgement),
    NoncomplianceNotification(AdscNoncomplianceNotification),
    CancelEmergencyMode,
    BasicReport(AdscBasicReport),
    EmergencyBasicReport(AdscBasicReport),
    LateralDeviationChangeEvent(AdscBasicReport),
    FlightId(AdscFlightId),
    PredictedRoute(AdscPredictedRoute),
    EarthReferenceData(AdscEarthReferenceData),
    AirReferenceData(AdscAirReferenceData),
    MeteoData(AdscMeteo),
    AirframeId(AdscAirframeId),
    VerticalRateChangeEvent(AdscBasicReport),
    AltitudeRangeEvent(AdscBasicReport),
    WaypointChangeEvent(AdscBasicReport),
    IntermediateProjection(AdscIntermediateProjection),
    FixedProjection(AdscFixedProjection),
    // Uplink tags.
    CancelAllContracts,
    CancelContract { contract_number: u8 },
    PeriodicContractRequest(AdscPeriodicContractRequest),
    EventContractRequest(AdscEventContractRequest),
    EmergencyPeriodicContractRequest(AdscPeriodicContractRequest),
}

/// A periodic or emergency-periodic contract.
///
/// The reporting interval controls the cadence of the basic report. Additional
/// report groups are requested independently and carry their own moduli.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdscPeriodicContractRequest {
    pub contract_number: u8,
    pub report_interval_secs: u32,
    pub requested_groups: Vec<AdscPeriodicReportGroup>,
}

/// An event contract containing one or more independent event triggers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdscEventContractRequest {
    pub contract_number: u8,
    pub events: Vec<AdscEventTrigger>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AdscPeriodicReportGroup {
    FlightId {
        modulus: u8,
    },
    PredictedRoute {
        modulus: u8,
    },
    EarthReferenceData {
        modulus: u8,
    },
    AirReferenceData {
        modulus: u8,
    },
    MeteoData {
        modulus: u8,
    },
    AirframeId {
        modulus: u8,
    },
    AircraftIntentData {
        modulus: u8,
        projection_time_mins: u8,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AdscEventTrigger {
    LateralDeviationChange { threshold_nm: f64 },
    VerticalSpeedChange { threshold_ft_per_min: i32 },
    AltitudeRange { ceiling_ft: i32, floor_ft: i32 },
    WaypointChange,
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
            Self::CancelAllContracts => 1,
            Self::CancelContract { .. } => 2,
            Self::PeriodicContractRequest(_) => 7,
            Self::EventContractRequest(_) => 8,
            Self::EmergencyPeriodicContractRequest(_) => 9,
        }
    }
}

/// Reason carried by an ADS-C negative acknowledgement (Tag 4). Mirrors the
/// FANS-1/A / libacars reason-code table; reasons 1, 2, and 7 also carry an
/// extension byte (erroneous octet or tag number) kept separately as
/// `extension`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdscNackReason {
    DuplicateGroupTag,
    DuplicateReportingIntervalTag,
    EventContractRequestWithNoData,
    ImproperOperationalModeTag,
    CancelRequestOfNonexistentContract,
    RequestedContractAlreadyExists,
    UndefinedContractRequestTag,
    UndefinedError,
    NotEnoughDataInRequest,
    InvalidAltitudeRange,
    VerticalSpeedThresholdIsZero,
    AircraftIntentProjectionTimeIsZero,
    LateralDeviationThresholdIsZero,
    Unknown { code: u8 },
}

impl AdscNackReason {
    pub fn from_byte(code: u8) -> Self {
        match code {
            1 => Self::DuplicateGroupTag,
            2 => Self::DuplicateReportingIntervalTag,
            3 => Self::EventContractRequestWithNoData,
            4 => Self::ImproperOperationalModeTag,
            5 => Self::CancelRequestOfNonexistentContract,
            6 => Self::RequestedContractAlreadyExists,
            7 => Self::UndefinedContractRequestTag,
            8 => Self::UndefinedError,
            9 => Self::NotEnoughDataInRequest,
            10 => Self::InvalidAltitudeRange,
            11 => Self::VerticalSpeedThresholdIsZero,
            12 => Self::AircraftIntentProjectionTimeIsZero,
            13 => Self::LateralDeviationThresholdIsZero,
            other => Self::Unknown { code: other },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdscNegativeAcknowledgement {
    pub contract_request_number: u8,
    pub reason: AdscNackReason,
    pub extension: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdscNoncomplianceNotification {
    pub contract_request_number: u8,
    pub groups: Vec<AdscNoncomplianceGroup>,
}

/// Which request sub-tag a non-compliance notification refers to. This is a
/// name-only mirror of the request-group tag numbers (10–21); it carries no
/// payload, so it is kept separate from the payload-bearing request enums.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdscNoncompliantTag {
    LateralDeviationChange,
    ReportInterval,
    FlightId,
    PredictedRoute,
    EarthReferenceData,
    AirReferenceData,
    MeteoData,
    AirframeId,
    VerticalSpeedChange,
    AltitudeRange,
    ReportWaypointChanges,
    AircraftIntentData,
    Unknown { tag: u8 },
}

impl AdscNoncompliantTag {
    pub fn from_byte(tag: u8) -> Self {
        match tag {
            10 => Self::LateralDeviationChange,
            11 => Self::ReportInterval,
            12 => Self::FlightId,
            13 => Self::PredictedRoute,
            14 => Self::EarthReferenceData,
            15 => Self::AirReferenceData,
            16 => Self::MeteoData,
            17 => Self::AirframeId,
            18 => Self::VerticalSpeedChange,
            19 => Self::AltitudeRange,
            20 => Self::ReportWaypointChanges,
            21 => Self::AircraftIntentData,
            other => Self::Unknown { tag: other },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdscNoncomplianceGroup {
    pub noncompliant_tag: AdscNoncompliantTag,
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
    pub callsign: String,
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

/// Tag 14 — Earth Reference Data: the ground-relative side of the wind
/// triangle (true track + ground speed) plus vertical speed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdscEarthReferenceData {
    /// `None` when `track_invalid` is true; the field is then omitted from JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub true_track_degrees: Option<f64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub track_invalid: bool,
    pub ground_speed_kt: f64,
    pub vertical_speed_ft_per_min: i32,
}

/// Tag 15 — Air Reference Data: the air-relative side of the wind triangle
/// (true heading + Mach number) plus vertical speed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdscAirReferenceData {
    /// `None` when `heading_invalid` is true; the field is then omitted from JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub true_heading_degrees: Option<f64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub heading_invalid: bool,
    pub mach: f64,
    pub vertical_speed_ft_per_min: i32,
}

fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdscMeteo {
    pub wind_speed_kt: f64,
    /// `None` when `wind_direction_invalid` is true; the field is then omitted
    /// from JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wind_direction_true_degrees: Option<f64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub wind_direction_invalid: bool,
    pub temperature_c: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdscAirframeId {
    pub icao24: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdscIntermediateProjection {
    pub distance_nm: f64,
    /// `None` when `track_invalid` is true; the field is then omitted from JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_degrees: Option<f64>,
    #[serde(default, skip_serializing_if = "is_false")]
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

use crate::decode::acars::MessageDirection;

pub fn parse_adsc_app_text(txt: &str) -> DecodeResult<AdscMessage> {
    parse_adsc_app_text_with_direction(txt, MessageDirection::Unknown)
}

pub fn parse_adsc_app_text_with_direction(
    txt: &str,
    direction: MessageDirection,
) -> DecodeResult<AdscMessage> {
    let message = super::parse_with_direction(txt, direction)?;
    match message.payload {
        super::Payload::Adsc(adsc) => Ok(adsc),
        _ => Err(DecodeError::InvalidPayload(PayloadError::Adsc)),
    }
}

pub fn parse_adsc_payload_hex(payload_no_crc_hex: &str) -> DecodeResult<Vec<AdscTag>> {
    parse_adsc_payload_hex_with_direction(payload_no_crc_hex, MessageDirection::Unknown)
}

pub fn parse_adsc_payload_hex_with_direction(
    payload_no_crc_hex: &str,
    direction: MessageDirection,
) -> DecodeResult<Vec<AdscTag>> {
    let bytes = hex::decode(payload_no_crc_hex)
        .map_err(|_| DecodeError::InvalidPayload(PayloadError::Adsc))?;
    parse_adsc_payload_bytes_with_direction(&bytes, direction)
}

pub fn parse_adsc_payload_bytes(buf: &[u8]) -> DecodeResult<Vec<AdscTag>> {
    parse_adsc_payload_bytes_with_direction(buf, MessageDirection::Unknown)
}

pub fn parse_adsc_payload_bytes_with_direction(
    buf: &[u8],
    direction: MessageDirection,
) -> DecodeResult<Vec<AdscTag>> {
    // Uplink (ground-to-air) messages use a different tag table than downlink.
    let is_uplink = direction == MessageDirection::GroundToAir;
    let mut idx = 0usize;
    let mut tags = Vec::new();

    while idx < buf.len() {
        let tag = buf[idx];
        idx += 1;

        // Route through uplink tag table when direction is ground-to-air.
        if is_uplink {
            match tag {
                1 => tags.push(AdscTag::CancelAllContracts),
                2 => {
                    let data = take(buf, &mut idx, 1)?;
                    tags.push(AdscTag::CancelContract {
                        contract_number: data[0],
                    });
                }
                6 => {
                    let data = take(buf, &mut idx, 1)?;
                    tags.push(AdscTag::CancelContract {
                        contract_number: data[0],
                    });
                }
                7 => tags.push(AdscTag::PeriodicContractRequest(
                    parse_periodic_contract_request(buf, &mut idx)?,
                )),
                8 => tags.push(AdscTag::EventContractRequest(parse_event_contract_request(
                    buf, &mut idx,
                )?)),
                9 => tags.push(AdscTag::EmergencyPeriodicContractRequest(
                    parse_periodic_contract_request(buf, &mut idx)?,
                )),
                _ => return Err(DecodeError::InvalidPayload(PayloadError::Adsc)),
            }
            continue;
        }

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
                        reason: AdscNackReason::from_byte(raw.reason),
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
                        noncompliant_tag: AdscNoncompliantTag::from_byte(raw.noncompliant_tag),
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
                tags.push(AdscTag::FlightId(AdscFlightId { callsign: id }));
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
                // Tags 14 and 15 share the same bit layout (EarthAirRefRaw) but
                // carry different physical quantities: Tag 14 is the
                // ground-relative side of the wind triangle (true track + ground
                // speed in knots), while Tag 15 is the air-relative side (true
                // heading + Mach number). The speed field therefore uses a
                // different scale per tag. See the OpenSky ADS-C dataset
                // (Zenodo 14659997) and the libacars reference decoder, which
                // applies the Mach scale (/2000) only for Tag 15.
                let heading_degrees = decode_heading(raw.heading_raw);
                let invalid = raw.heading_invalid != 0;
                let vertical_speed_ft_per_min = decode_vertical_speed(raw.vertical_speed_raw);
                // The angle is only meaningful when its validity bit is clear; keep it
                // as Some then, otherwise None so serde omits it (and track_invalid / heading_invalid is emitted instead).
                if tag == 14 {
                    tags.push(AdscTag::EarthReferenceData(AdscEarthReferenceData {
                        true_track_degrees: (!invalid).then_some(heading_degrees),
                        track_invalid: invalid,
                        ground_speed_kt: decode_ground_speed(raw.speed_raw),
                        vertical_speed_ft_per_min,
                    }));
                } else {
                    tags.push(AdscTag::AirReferenceData(AdscAirReferenceData {
                        true_heading_degrees: (!invalid).then_some(heading_degrees),
                        heading_invalid: invalid,
                        mach: decode_mach(raw.speed_raw),
                        vertical_speed_ft_per_min,
                    }));
                }
            }
            16 => {
                let (_, raw) = MeteoRaw::from_bytes((take(buf, &mut idx, 4)?, 0))
                    .map_err(|e| DecodeError::Deku(e.to_string()))?;
                let wind_direction_invalid = raw.wind_direction_invalid != 0;
                tags.push(AdscTag::MeteoData(AdscMeteo {
                    wind_speed_kt: decode_speed(raw.wind_speed_raw),
                    wind_direction_true_degrees: (!wind_direction_invalid)
                        .then_some(decode_wind_direction(raw.wind_direction_raw)),
                    wind_direction_invalid,
                    temperature_c: decode_temperature(raw.temperature_raw),
                }));
            }
            17 => {
                let data = take(buf, &mut idx, 3)?;
                tags.push(AdscTag::AirframeId(AdscAirframeId {
                    icao24: format!("{:02x}{:02x}{:02x}", data[0], data[1], data[2]),
                }));
            }
            22 => {
                let (_, raw) = IntermediateProjectionRaw::from_bytes((take(buf, &mut idx, 8)?, 0))
                    .map_err(|e| DecodeError::Deku(e.to_string()))?;
                let track_invalid = raw.track_invalid != 0;
                tags.push(AdscTag::IntermediateProjection(
                    AdscIntermediateProjection {
                        distance_nm: decode_distance(raw.distance_raw),
                        track_degrees: (!track_invalid).then_some(decode_heading(raw.track_raw)),
                        track_invalid,
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

fn parse_periodic_contract_request(
    buf: &[u8],
    idx: &mut usize,
) -> DecodeResult<AdscPeriodicContractRequest> {
    let contract_number = take(buf, idx, 1)?[0];
    if buf.get(*idx) != Some(&11) {
        return Err(DecodeError::InvalidPayload(PayloadError::Adsc));
    }
    *idx += 1;
    let report_interval_secs = decode_report_interval(take(buf, idx, 1)?[0]);
    let mut requested_groups = Vec::new();

    while let Some(&tag) = buf.get(*idx) {
        let group = match tag {
            12 => AdscPeriodicReportGroup::FlightId {
                modulus: take_tagged_u8(buf, idx)?,
            },
            13 => AdscPeriodicReportGroup::PredictedRoute {
                modulus: take_tagged_u8(buf, idx)?,
            },
            14 => AdscPeriodicReportGroup::EarthReferenceData {
                modulus: take_tagged_u8(buf, idx)?,
            },
            15 => AdscPeriodicReportGroup::AirReferenceData {
                modulus: take_tagged_u8(buf, idx)?,
            },
            16 => AdscPeriodicReportGroup::MeteoData {
                modulus: take_tagged_u8(buf, idx)?,
            },
            17 => AdscPeriodicReportGroup::AirframeId {
                modulus: take_tagged_u8(buf, idx)?,
            },
            21 => {
                *idx += 1;
                let data = take(buf, idx, 2)?;
                AdscPeriodicReportGroup::AircraftIntentData {
                    modulus: data[0],
                    projection_time_mins: data[1],
                }
            }
            _ => break,
        };
        requested_groups.push(group);
    }

    Ok(AdscPeriodicContractRequest {
        contract_number,
        report_interval_secs,
        requested_groups,
    })
}

fn parse_event_contract_request(
    buf: &[u8],
    idx: &mut usize,
) -> DecodeResult<AdscEventContractRequest> {
    let contract_number = take(buf, idx, 1)?[0];
    let mut events = Vec::new();

    while let Some(&tag) = buf.get(*idx) {
        let event = match tag {
            10 => AdscEventTrigger::LateralDeviationChange {
                threshold_nm: take_tagged_u8(buf, idx)? as f64 / 8.0,
            },
            18 => AdscEventTrigger::VerticalSpeedChange {
                threshold_ft_per_min: (take_tagged_u8(buf, idx)? as i8) as i32 * 64,
            },
            19 => {
                *idx += 1;
                let data = take(buf, idx, 4)?;
                let ceiling_raw = ((data[0] as u16) << 8) | data[1] as u16;
                let floor_raw = ((data[2] as u16) << 8) | data[3] as u16;
                AdscEventTrigger::AltitudeRange {
                    ceiling_ft: decode_altitude_u16(ceiling_raw),
                    floor_ft: decode_altitude_u16(floor_raw),
                }
            }
            20 => {
                *idx += 1;
                AdscEventTrigger::WaypointChange
            }
            _ => break,
        };
        events.push(event);
    }

    Ok(AdscEventContractRequest {
        contract_number,
        events,
    })
}

fn take_tagged_u8(buf: &[u8], idx: &mut usize) -> DecodeResult<u8> {
    *idx += 1;
    Ok(take(buf, idx, 1)?[0])
}

fn decode_report_interval(encoded: u8) -> u32 {
    let scaling_factor = match (encoded & 0xc0) >> 6 {
        2 => 8,
        3 => 64,
        value => value as u32,
    };
    let rate = (encoded & 0x3f) as u32;
    scaling_factor * (rate + 1)
}

fn decode_altitude_u16(raw: u16) -> i32 {
    // Same formula as `decode_altitude` but from a pre-assembled u16.
    let unsigned = i32::from(raw as i16);
    if unsigned < 0 {
        (unsigned * 25).wrapping_sub(200)
    } else {
        unsigned * 25 - 200
    }
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

fn decode_ground_speed(value: u16) -> f64 {
    // Tag 14 Earth Reference ground speed, in knots (LSB = 0.5 kt).
    value as f64 / 2.0
}

fn decode_mach(value: u16) -> f64 {
    // Tag 15 Air Reference Mach number. The 13-bit field is encoded with the
    // same 0.5 LSB as the ground-speed/wind fields, then divided by 1000 to
    // yield Mach — so the raw value maps to Mach via /2000 (e.g. 1674 -> 0.837).
    value as f64 / 2000.0
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_adsc_sample() {
        let msg = parse_adsc_app_text(
            "/BDOCAYA.ADS.A7-ANR073759D0C997088B86BC1F0D377770C71C488B805B38E698AB9AC88B80A626",
        )
        .unwrap();

        assert!(!msg.tags.is_empty());
        assert_eq!(msg.tags[0].id(), 7);
    }

    #[test]
    fn parse_adsc_uplink_contract_request() {
        use crate::decode::acars::MessageDirection;
        // /UPGCAYA.ADS.B-324P07020BCD0D010E0110014BAA
        // Tag 7 = periodic_contract_req, contract=2
        // Sub-tags: 0x0B(11)=interval 0xCD, 0x0D(13)=predicted_route mod=1,
        //           0x0E(14)=earth_ref mod=1, 0x10(16)=meteo mod=1
        let msg = parse_adsc_app_text_with_direction(
            "/UPGCAYA.ADS.B-324P07020BCD0D010E0110014BAA",
            MessageDirection::GroundToAir,
        )
        .unwrap();
        assert_eq!(msg.tags.len(), 1);
        let AdscTag::PeriodicContractRequest(req) = &msg.tags[0] else {
            panic!("expected PeriodicContractRequest, got {:?}", msg.tags[0]);
        };
        assert_eq!(req.contract_number, 2);
        assert_eq!(req.report_interval_secs, 896);
        assert_eq!(req.requested_groups.len(), 3);
        assert!(matches!(
            &req.requested_groups[0],
            AdscPeriodicReportGroup::PredictedRoute { modulus: 1 }
        ));
        assert!(matches!(
            &req.requested_groups[1],
            AdscPeriodicReportGroup::EarthReferenceData { modulus: 1 }
        ));
        assert!(matches!(
            &req.requested_groups[2],
            AdscPeriodicReportGroup::MeteoData { modulus: 1 }
        ));
    }

    #[test]
    fn parse_adsc_uplink_multi_group() {
        use crate::decode::acars::MessageDirection;
        // /OAKODYA.ADS.N509DT07030B970C000D010E0110000F01150001B64C
        // contract_num=3, groups: interval=192s, flight_id mod=0,
        //   predicted_route mod=1, earth_ref mod=1, meteo mod=0, air_ref mod=1,
        //   acft_intent_data(mod=0, proj=1)
        let msg = parse_adsc_app_text_with_direction(
            "/OAKODYA.ADS.N509DT07030B970C000D010E0110000F01150001B64C",
            MessageDirection::GroundToAir,
        )
        .unwrap();
        let AdscTag::PeriodicContractRequest(req) = &msg.tags[0] else {
            panic!("expected PeriodicContractRequest");
        };
        assert_eq!(req.contract_number, 3);
        // interval: 0x97 -> sf=2->8, rate=0x17=23 -> 8*(23+1)=192
        assert_eq!(req.report_interval_secs, 192);
        assert_eq!(req.requested_groups.len(), 6);
        assert!(matches!(
            req.requested_groups.last().unwrap(),
            AdscPeriodicReportGroup::AircraftIntentData {
                modulus: 0,
                projection_time_mins: 1
            }
        ));
    }

    #[test]
    fn parse_adsc_uplink_event_triggers() {
        use crate::decode::acars::MessageDirection;
        // contract_num=5, independent waypoint-change and lateral-deviation
        // event triggers; the latter has a 5 NM threshold.
        let msg = parse_adsc_app_text_with_direction(
            "/OAKODYA.ADS.N2645U0805140A28E574",
            MessageDirection::GroundToAir,
        )
        .unwrap();
        let AdscTag::EventContractRequest(req) = &msg.tags[0] else {
            panic!("expected EventContractRequest");
        };
        assert_eq!(req.contract_number, 5);
        assert_eq!(req.events.len(), 2);
        assert!(matches!(req.events[0], AdscEventTrigger::WaypointChange));
        assert!(matches!(
            req.events[1],
            AdscEventTrigger::LateralDeviationChange { threshold_nm: 5.0 }
        ));
    }

    #[test]
    fn parse_adsc_uplink_periodic_and_event_requests() {
        use crate::decode::acars::MessageDirection;
        let msg = parse_adsc_app_text_with_direction(
            "/ANCATYA.ADS.N704GT07000BC80C000D010E0110000F011500010801140A288520",
            MessageDirection::GroundToAir,
        )
        .unwrap();
        assert_eq!(msg.tags.len(), 2);
        let AdscTag::PeriodicContractRequest(periodic) = &msg.tags[0] else {
            panic!("expected PeriodicContractRequest");
        };
        assert_eq!(periodic.contract_number, 0);
        assert_eq!(periodic.report_interval_secs, 576);
        let AdscTag::EventContractRequest(event) = &msg.tags[1] else {
            panic!("expected EventContractRequest");
        };
        assert_eq!(event.contract_number, 1);
        assert_eq!(event.events.len(), 2);
    }

    #[test]
    fn reject_periodic_request_without_reporting_interval() {
        use crate::decode::acars::MessageDirection;
        let result =
            parse_adsc_payload_bytes_with_direction(&[7, 1, 13, 1], MessageDirection::GroundToAir);
        assert!(result.is_err());
    }

    #[test]
    fn parse_emergency_periodic_request() {
        use crate::decode::acars::MessageDirection;
        let tags = parse_adsc_payload_bytes_with_direction(
            &[9, 3, 11, 0xcd, 13, 1],
            MessageDirection::GroundToAir,
        )
        .unwrap();
        let AdscTag::EmergencyPeriodicContractRequest(request) = &tags[0] else {
            panic!("expected EmergencyPeriodicContractRequest");
        };
        assert_eq!(request.contract_number, 3);
        assert_eq!(request.report_interval_secs, 896);
        assert!(matches!(
            request.requested_groups[0],
            AdscPeriodicReportGroup::PredictedRoute { modulus: 1 }
        ));
    }

    #[test]
    fn flight_id_is_serialized_as_callsign() {
        // /CCUCAYA.ADS.A6-BNC... carries a flight-id group with ETD403.
        let msg = parse_adsc_app_text(
            "/CCUCAYA.ADS.A6-BNC0301070B0B0A0CA048C99D2297170B88B1FC2188CA04AD0C154134C338200D0B14DA0B6088CA005B0B63BA00F508CA000E65110340040F64F9A740041013143EAC1189653AB2C1",
        )
        .unwrap();
        let fid = msg
            .tags
            .iter()
            .find_map(|t| match t {
                AdscTag::FlightId(d) => Some(d),
                _ => None,
            })
            .expect("expected a flight-id group");
        assert_eq!(fid.callsign, "ETD403");
    }

    #[test]
    fn airframe_id_is_serialized_as_icao24_lowercase_hex() {
        // A6-BNC / ETD403 => UAE block 0x896000–0x896FFF.
        let msg = parse_adsc_app_text(
            "/CCUCAYA.ADS.A6-BNC0301070B0B0A0CA048C99D2297170B88B1FC2188CA04AD0C154134C338200D0B14DA0B6088CA005B0B63BA00F508CA000E65110340040F64F9A740041013143EAC1189653AB2C1",
        )
        .unwrap();
        let aid = msg
            .tags
            .iter()
            .find_map(|t| match t {
                AdscTag::AirframeId(d) => Some(d),
                _ => None,
            })
            .expect("expected an airframe-id group");
        assert_eq!(aid.icao24, "89653a");
    }

    #[test]
    fn noncompliance_tag_is_semantic_enum() {
        // /SEZCAYA.ADS.HZ-ARC... reports the reporting-interval group (tag 11)
        // as whole-group-unavailable.
        let msg = parse_adsc_app_text(
            "/SEZCAYA.ADS.HZ-ARC03010501010B4007F8D93908C88946C9519F0DF8E391086689470027FA7190FA510947000E6E60F30000AD7C",
        )
        .unwrap();
        let nc = msg
            .tags
            .iter()
            .find_map(|t| match t {
                AdscTag::NoncomplianceNotification(d) => Some(d),
                _ => None,
            })
            .expect("expected a non-compliance notification");
        assert_eq!(nc.groups.len(), 1);
        assert_eq!(
            nc.groups[0].noncompliant_tag,
            AdscNoncompliantTag::ReportInterval
        );
        assert!(nc.groups[0].is_whole_group_unavailable);
        assert!(!nc.groups[0].is_unrecognized);
    }

    #[test]
    fn intermediate_projection_keeps_track_invalid_flag() {
        // /FUKJJYA.ADS.HL8701... carries a Tag 22 intermediate projection. The
        // track validity is carried both as `track_degrees: Option<f64>` (None
        // when invalid) and as the `track_invalid` boolean, mirroring the
        // earth/air reference groups.
        let msg = parse_adsc_app_text(
            "/FUKJJYA.ADS.HL87010716B9138FDE0946EBB81F16007863D928E0106017177C037438C94708AA0D16C16B8E38C9470083182D8355554947000E63B8CE00040F63A9A540045A49",
        )
        .unwrap();
        let proj = msg
            .tags
            .iter()
            .find_map(|t| match t {
                AdscTag::IntermediateProjection(d) => Some(d),
                _ => None,
            })
            .expect("expected an intermediate projection");
        assert!(proj.distance_nm.is_finite());
        assert_eq!(proj.track_degrees.is_none(), proj.track_invalid);
        assert!(proj.track_degrees.is_none_or(|t| t.is_finite()));
        assert!(proj.eta_seconds <= 0x3fff);
    }

    #[test]
    fn nack_reason_is_semantic_enum() {
        // /ALGCAYA.ADS.ET-AWM0401069CA4: Tag 4 NAK, contract 1, reason 6
        // (requested contract already exists), no extension.
        let msg = parse_adsc_app_text("/ALGCAYA.ADS.ET-AWM0401069CA4").unwrap();
        let nak = msg
            .tags
            .iter()
            .find_map(|t| match t {
                AdscTag::NegativeAcknowledgement(d) => Some(d),
                _ => None,
            })
            .expect("expected a negative acknowledgement");
        assert_eq!(nak.contract_request_number, 1);
        assert_eq!(nak.reason, AdscNackReason::RequestedContractAlreadyExists);
        assert!(nak.extension.is_none());
    }
}
