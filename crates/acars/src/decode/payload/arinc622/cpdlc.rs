//! FANS-1/A CPDLC decoder for ARINC 622 payloads.
//!
//! This module decodes CPDLC application bodies carried by ARINC 622 IMIs such
//! as `AT1`, `CR1`, `CC1`, and `DR1`. It parses the FANS-1/A ATC message header,
//! message elements, and common element bodies into the shared
//! [`CpdlcPduSummary`] / [`CpdlcElement`] representation used by both FANS-1/A
//! and ATN B1 CPDLC output.
//!
//! Element names and message templates are backed by the bundled
//! `data/cpdlc_fans.json` catalog; unsupported bodies are preserved as element
//! IDs and names instead of causing the entire payload to be dropped.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::decode::acars::MessageDirection;
use crate::decode::payload::PayloadError;
use crate::decode::{DecodeError, DecodeResult};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CpdlcDecodeError {
    #[error("need {needed} bits, only {remaining} bits remain at bit {bit_pos}")]
    BitReadOutOfBounds {
        needed: usize,
        remaining: usize,
        bit_pos: usize,
    },
    #[error("CPDLC element id {0} out of range")]
    ElementIdOutOfRange(u16),
    #[error("invalid CPDLC position choice {0}")]
    InvalidPositionChoice(u64),
    #[error("invalid CPDLC direction enum {0}")]
    InvalidDirectionEnum(u64),
    #[error("invalid CPDLC procedure type {0}")]
    InvalidProcedureType(u64),
    #[error("invalid CPDLC error information enum {0}")]
    InvalidErrorInformation(u64),
    #[error("invalid CPDLC IA5 token {0:?}")]
    InvalidIa5Token(String),
    #[error("invalid NumericString PER code {0}")]
    InvalidNumericStringCode(u8),
    #[error("predeparture clearance body is not decoded yet")]
    PredepartureClearanceNotDecoded,
}

/// Initial FANS-1/A CPDLC decode.
///
/// This is intentionally shallow: it decodes the common ATC message header and
/// first message-element CHOICE index from the unaligned PER payload, preserving
/// the full raw hex for later full ASN.1 work.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CpdlcMessage {
    pub payload_hex: String,
    pub payload_len_bytes: usize,
    /// Best-effort downlink interpretation as `FANSATCDownlinkMessage`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downlink: Option<CpdlcPduSummary>,
    /// Best-effort uplink interpretation as `FANSATCUplinkMessage`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uplink: Option<CpdlcPduSummary>,
    /// FANS-1/A CPDLC control message wrapper (`CR1`, `CC1`, `DR1`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control: Option<CpdlcControlMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum CpdlcControlMessage {
    ConnectRequest {
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<CpdlcPduSummary>,
    },
    ConnectConfirm {
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<CpdlcPduSummary>,
    },
    DisconnectRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpdlcControlKind {
    ConnectRequest,
    ConnectConfirm,
    DisconnectRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CpdlcPduSummary {
    pub header: AtcMessageHeader,
    pub elements: Vec<CpdlcElement>,
    pub remaining_bits_after_element: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CpdlcElement {
    pub id: u16,
    pub catalog_name: String,
    pub fragments: Vec<CpdlcPhraseFragment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<CpdlcElementBody>,
    pub is_additional: bool,
}

#[derive(Debug)]
struct CpdlcCatalog {
    uplink: Vec<CpdlcElementInfo>,
    downlink: Vec<CpdlcElementInfo>,
}

#[derive(Debug, Deserialize)]
struct RawCpdlcCatalog {
    uplink: Vec<RawCpdlcElementInfo>,
    downlink: Vec<RawCpdlcElementInfo>,
}

#[derive(Debug, Deserialize)]
struct RawCpdlcElementInfo {
    id: u16,
    name: String,
    template: String,
}

#[derive(Debug)]
pub(crate) struct CpdlcElementInfo {
    id: u16,
    pub(crate) catalog_name: String,
    fragments: Vec<CpdlcTemplateFragment>,
    body_slots: Option<Vec<CpdlcTemplateSlot>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CpdlcTemplateFragment {
    Text(String),
    Slot(CpdlcTemplateSlot),
}

// Phrase value fragments serialize one of these slots. The frontend resolves a
// slot against an element body by using `body.data` when `body.kind == slot`, or
// `body.data[slot]` for compound bodies. Keep `CpdlcTemplateSlot::as_str`, these
// serde names, and `CpdlcElementBody` variant/field names aligned.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CpdlcTemplateSlot {
    Altimeter,
    Altitude,
    Altitude2,
    AtisCode,
    BeaconCode,
    Degrees,
    Direction,
    DistanceOffset,
    ErrorInformation,
    FreeText,
    Frequency,
    IcaoFacilityDesignation,
    IcaoUnitName,
    Position,
    PositionReport,
    ProcedureName,
    RouteClearance,
    Speed,
    Time,
    Tp4Table,
    VersionNumber,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum CpdlcPhraseFragment {
    Text(String),
    Value(CpdlcTemplateSlot),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum CpdlcElementBody {
    Altitude(CpdlcAltitude),
    AltitudeTime {
        altitude: CpdlcAltitude,
        time: CpdlcTime,
    },
    FreeText(String),
    IcaoFacilityDesignation(IcaoFacilityDesignation),
    IcaoFacilityDesignationTp4Table {
        icao_facility_designation: IcaoFacilityDesignation,
        tp4_table: Tp4Table,
    },
    IcaoUnitNameFrequency {
        icao_unit_name: IcaoUnitName,
        frequency: CpdlcFrequency,
    },
    TimeIcaoUnitNameFrequency {
        time: CpdlcTime,
        icao_unit_name: IcaoUnitName,
        frequency: CpdlcFrequency,
    },
    DistanceOffsetDirection {
        distance_offset: DistanceOffset,
        direction: CpdlcDirection,
    },
    Position(CpdlcPosition),
    PositionAltitude {
        position: CpdlcPosition,
        altitude: CpdlcAltitude,
    },
    AltitudePosition {
        altitude: CpdlcAltitude,
        position: CpdlcPosition,
    },
    AltitudeAltitude {
        altitude: CpdlcAltitude,
        altitude2: CpdlcAltitude,
    },
    TimeAltitude {
        time: CpdlcTime,
        altitude: CpdlcAltitude,
    },
    PositionDistanceOffsetDirection {
        position: CpdlcPosition,
        distance_offset: DistanceOffset,
        direction: CpdlcDirection,
    },
    PositionIcaoUnitNameFrequency {
        position: CpdlcPosition,
        icao_unit_name: IcaoUnitName,
        frequency: CpdlcFrequency,
    },
    TimeDistanceOffsetDirection {
        time: CpdlcTime,
        distance_offset: DistanceOffset,
        direction: CpdlcDirection,
    },
    Frequency(CpdlcFrequency),
    Time(CpdlcTime),
    DirectionDegrees {
        direction: CpdlcDirection,
        degrees: CpdlcDegrees,
    },
    Degrees(CpdlcDegrees),
    AtisCode(String),
    ProcedureName(CpdlcProcedureName),
    Speed(CpdlcSpeed),
    TimeSpeed {
        time: CpdlcTime,
        speed: CpdlcSpeed,
    },
    PositionSpeedSpeed {
        position: CpdlcPosition,
        speed: [CpdlcSpeed; 2],
    },
    PositionAltitudeSpeed {
        position: CpdlcPosition,
        altitude: CpdlcAltitude,
        speed: CpdlcSpeed,
    },
    PositionTime {
        position: CpdlcPosition,
        time: CpdlcTime,
    },
    PositionTimeTime {
        position: CpdlcPosition,
        time: [CpdlcTime; 2],
    },
    TimePositionAltitude {
        time: CpdlcTime,
        position: CpdlcPosition,
        altitude: CpdlcAltitude,
    },
    PositionTimeAltitude {
        position: CpdlcPosition,
        time: CpdlcTime,
        altitude: CpdlcAltitude,
    },
    TimePositionAltitudeSpeed {
        time: CpdlcTime,
        position: CpdlcPosition,
        altitude: CpdlcAltitude,
        speed: CpdlcSpeed,
    },
    AltitudeSpeedSpeed {
        altitude: CpdlcAltitude,
        speed: [CpdlcSpeed; 2],
    },
    PositionPosition {
        position: [CpdlcPosition; 2],
    },
    PositionReport(Box<CpdlcPositionReport>),
    ErrorInformation(CpdlcErrorInformation),
    Altimeter(CpdlcAltimeter),
    RouteClearance(RouteClearance),
    OpaqueRouteClearance {
        remaining_bits: usize,
        payload_hex: String,
    },

    BeaconCode(String),
    VersionNumber(u8),
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteClearance {
    pub airport_departure: Option<String>,
    pub airport_destination: Option<String>,
    pub runway_departure: Option<String>,
    pub procedure_departure: Option<String>,
    pub runway_arrival: Option<String>,
    pub procedure_approach: Option<String>,
    pub procedure_arrival: Option<String>,
    pub airway_intercept: Option<String>,
    pub route_information: Option<Vec<RouteInformation>>,
    pub remaining_bits: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum RouteInformation {
    PublishedIdentifier {
        fix: String,
        position: Option<CpdlcPosition>,
    },
    LatitudeLongitude {
        latitude: f64,
        longitude: f64,
    },
    Airway(String),
    Track {
        name: String,
    },
    Unsupported {
        choice: u8,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum CpdlcAltitude {
    QnhFeet(u16),
    QnhMeters(u16),
    QfeFeet(u16),
    QfeMeters(u16),
    GnssFeet(u32),
    GnssMeters(u32),
    FlightLevel(u16),
    FlightLevelMetric(u16),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum IcaoFacilityDesignation {
    Icao(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CpdlcTime {
    pub hour: u8,
    pub minute: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Tp4Table {
    LabelA,
    LabelB,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IcaoUnitName {
    pub facility: IcaoFacilityIdentification,
    pub function: IcaoFacilityFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum IcaoFacilityIdentification {
    Designation(String),
    Name(String),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum IcaoFacilityFunction {
    Center,
    Approach,
    Tower,
    Final,
    GroundControl,
    ClearanceDelivery,
    Departure,
    Control,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum CpdlcFrequency {
    HfKhz(u32),
    VhfKhz(u32),
    UhfKhz(u32),
    SatChannel(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum DistanceOffset {
    Nm(u16),
    Km(u16),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CpdlcDirection {
    Left,
    Right,
    EitherSide,
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum CpdlcPosition {
    FixName(String),
    Navaid(String),
    Airport(String),
    LatitudeLongitude { latitude: f64, longitude: f64 },
    UnsupportedPlaceBearingDistance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CpdlcPositionReport {
    pub current_position: CpdlcPosition,
    pub current_time: CpdlcTime,
    pub altitude: CpdlcAltitude,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_fix: Option<CpdlcPosition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_fix_eta: Option<CpdlcTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_plus_one_fix: Option<CpdlcPosition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_eta: Option<CpdlcTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_fuel: Option<CpdlcTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<CpdlcTemperature>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winds: Option<CpdlcWinds>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turbulence: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icing: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<CpdlcSpeed>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ground_speed_knots: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical_change: Option<CpdlcVerticalChange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_angle: Option<CpdlcDegrees>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub true_heading: Option<CpdlcDegrees>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<CpdlcDistance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supplementary_information: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported_waypoint_position: Option<CpdlcPosition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported_waypoint_time: Option<CpdlcTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported_waypoint_altitude: Option<CpdlcAltitude>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum CpdlcTemperature {
    Celsius(i16),
    Fahrenheit(i16),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CpdlcWinds {
    pub direction_degrees: u16,
    pub speed: CpdlcWindSpeed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum CpdlcWindSpeed {
    Knots(u16),
    Kmh(u16),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CpdlcVerticalChange {
    pub direction: CpdlcVerticalDirection,
    pub rate: CpdlcVerticalRate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CpdlcVerticalDirection {
    Up,
    Down,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum CpdlcVerticalRate {
    FeetPerMinute(u16),
    MetersPerMinute(u16),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum CpdlcDistance {
    NauticalMiles(u16),
    Kilometers(u16),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum CpdlcDegrees {
    Magnetic(u16),
    True(u16),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CpdlcProcedureName {
    pub procedure_type: CpdlcProcedureType,
    pub procedure: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CpdlcProcedureType {
    Arrival,
    Approach,
    Departure,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum CpdlcSpeed {
    IndicatedKnots(u16),
    IndicatedKmh(u16),
    TrueKnots(u16),
    TrueKmh(u16),
    GroundKnots(u16),
    GroundKmh(u16),
    Mach(u16),
    MachLarge(u16),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum CpdlcAltimeter {
    InHgHundredths(u16),
    HectoPascals(u16),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CpdlcErrorInformation {
    ApplicationError,
    DuplicateMsgIdentificationNumber,
    UnrecognizedMsgReferenceNumber,
    EndServiceWithPendingMsgs,
    EndServiceWithNoValidResponse,
    InsufficientMsgStorageCapacity,
    NoAvailableMsgIdentificationNumber,
    CommandedTermination,
    InsufficientData,
    UnexpectedData,
    InvalidData,
    ReservedErrorMsg1,
    ReservedErrorMsg2,
    ReservedErrorMsg3,
    ReservedErrorMsg4,
    ReservedErrorMsg5,
    ReservedErrorMsg6,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AtcMessageHeader {
    pub msg_id: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg_ref: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<CpdlcTimestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CpdlcTimestamp {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

pub fn parse_cpdlc_payload_hex(payload_hex: &str) -> DecodeResult<CpdlcMessage> {
    parse_cpdlc_payload_hex_with_direction(payload_hex, MessageDirection::Unknown)
}

pub fn parse_cpdlc_payload_hex_with_direction(
    payload_hex: &str,
    direction: MessageDirection,
) -> DecodeResult<CpdlcMessage> {
    let bytes = hex::decode(payload_hex).map_err(|e| {
        DecodeError::InvalidPayload(PayloadError::Arinc622(format!(
            "invalid CPDLC hex payload: {e}"
        )))
    })?;
    let downlink = match direction {
        MessageDirection::AirToGround | MessageDirection::Unknown => {
            parse_pdu_summary(&bytes, PduKind::Downlink).ok()
        }
        MessageDirection::GroundToAir => None,
    };
    let uplink = match direction {
        MessageDirection::GroundToAir | MessageDirection::Unknown => {
            parse_pdu_summary(&bytes, PduKind::Uplink).ok()
        }
        MessageDirection::AirToGround => None,
    };
    Ok(CpdlcMessage {
        payload_hex: payload_hex.to_string(),
        payload_len_bytes: bytes.len(),
        downlink,
        uplink,
        control: None,
    })
}

pub fn parse_cpdlc_control_payload_hex(
    payload_hex: &str,
    kind: CpdlcControlKind,
) -> DecodeResult<CpdlcMessage> {
    let bytes = hex::decode(payload_hex).map_err(|e| {
        DecodeError::InvalidPayload(PayloadError::Arinc622(format!(
            "invalid CPDLC hex payload: {e}"
        )))
    })?;
    let message = match kind {
        CpdlcControlKind::ConnectRequest => parse_pdu_summary(&bytes, PduKind::Uplink).ok(),
        CpdlcControlKind::ConnectConfirm | CpdlcControlKind::DisconnectRequest => None,
    };
    let control = match kind {
        CpdlcControlKind::ConnectRequest => CpdlcControlMessage::ConnectRequest {
            message: message.clone(),
        },
        CpdlcControlKind::ConnectConfirm => CpdlcControlMessage::ConnectConfirm {
            message: message.clone(),
        },
        CpdlcControlKind::DisconnectRequest => CpdlcControlMessage::DisconnectRequest,
    };
    Ok(CpdlcMessage {
        payload_hex: payload_hex.to_string(),
        payload_len_bytes: bytes.len(),
        downlink: None,
        uplink: message,
        control: Some(control),
    })
}

#[derive(Debug, Clone, Copy)]
pub enum PduKind {
    Downlink,
    Uplink,
}

fn parse_pdu_summary(bytes: &[u8], kind: PduKind) -> Result<CpdlcPduSummary, CpdlcDecodeError> {
    let mut bits = BitReader::new(bytes);

    // FANSATC{Downlink,Uplink}Message has one optional root field:
    // an additional message-element SEQUENCE OF (SIZE(1..4)). PER emits the
    // root optional bitmap first, then the mandatory header and element, then
    // the optional sequence content when present.
    let has_additional_elements = bits.read_bool()?;

    let header = parse_header(&mut bits)?;
    let first = parse_element(&mut bits, kind, false)?;
    let mut elements = vec![first];

    if has_additional_elements {
        // Keep the whole PDU candidate if the additional sequence is beyond
        // the currently implemented body coverage. This still exposes the
        // mandatory element and preserves the remaining bits for diagnostics.
        let mut additional_reader = bits.clone();
        if let Ok(additional_count) = additional_reader.read_bits(2).map(|v| v as usize + 1) {
            let mut additional = Vec::with_capacity(additional_count);
            let mut ok = true;
            for _ in 0..additional_count {
                match parse_element(&mut additional_reader, kind, true) {
                    Ok(element) => additional.push(element),
                    Err(_) => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                elements.extend(additional);
                bits = additional_reader;
            }
        }
    }

    Ok(CpdlcPduSummary {
        header,
        elements,
        remaining_bits_after_element: bits.remaining(),
    })
}

fn parse_element(
    bits: &mut BitReader<'_>,
    kind: PduKind,
    is_additional: bool,
) -> Result<CpdlcElement, CpdlcDecodeError> {
    let id = bits.read_bits(8)? as u16;
    let info = element_info(kind, id).ok_or(CpdlcDecodeError::ElementIdOutOfRange(id))?;
    let body = info
        .body_slots
        .as_ref()
        .and_then(|_| parse_element_body(bits, kind, id).ok());
    let fragments = cpdlc_phrase_fragments(info, body.as_ref()).collect();
    Ok(CpdlcElement {
        id,
        catalog_name: info.catalog_name.clone(),
        fragments,
        body,
        is_additional,
    })
}

fn parse_element_body(
    bits: &mut BitReader<'_>,
    kind: PduKind,
    element_id: u16,
) -> Result<CpdlcElementBody, CpdlcDecodeError> {
    match (kind, element_id) {
        (PduKind::Downlink, 6 | 8 | 9 | 10 | 28 | 29 | 30 | 32 | 37 | 38 | 54 | 61 | 72) => {
            Ok(CpdlcElementBody::Altitude(parse_altitude(bits)?))
        }
        (PduKind::Downlink, 7 | 76 | 77) => {
            let first = parse_altitude(bits)?;
            let second = parse_altitude(bits)?;
            Ok(CpdlcElementBody::AltitudeAltitude {
                altitude: first,
                altitude2: second,
            })
        }
        (PduKind::Downlink, 11 | 12) => {
            let position = parse_position(bits)?;
            let altitude = parse_altitude(bits)?;
            Ok(CpdlcElementBody::PositionAltitude { position, altitude })
        }
        (PduKind::Downlink, 13 | 14) => {
            let time = parse_time(bits)?;
            let altitude = parse_altitude(bits)?;
            Ok(CpdlcElementBody::TimeAltitude { time, altitude })
        }
        (PduKind::Downlink, 15 | 27 | 60 | 80) => {
            let distance = parse_distance_offset(bits)?;
            let direction = parse_direction(bits)?;
            Ok(CpdlcElementBody::DistanceOffsetDirection {
                distance_offset: distance,
                direction,
            })
        }
        (PduKind::Downlink, 16) => {
            let position = parse_position(bits)?;
            let distance = parse_distance_offset(bits)?;
            let direction = parse_direction(bits)?;
            Ok(CpdlcElementBody::PositionDistanceOffsetDirection {
                position,
                distance_offset: distance,
                direction,
            })
        }
        (PduKind::Downlink, 18 | 34 | 39 | 49) => Ok(CpdlcElementBody::Speed(parse_speed(bits)?)),
        (PduKind::Downlink, 22 | 31 | 33 | 42 | 44 | 45) => {
            Ok(CpdlcElementBody::Position(parse_position(bits)?))
        }
        (PduKind::Downlink, 23) => Ok(CpdlcElementBody::ProcedureName(parse_procedure_name(bits)?)),
        (PduKind::Downlink, 24 | 40) => Ok(parse_route_clearance_body(bits, "route_clearance")),
        (PduKind::Downlink, 48) => Ok(CpdlcElementBody::PositionReport(Box::new(
            parse_position_report(bits)?,
        ))),
        (PduKind::Downlink, 26) => {
            let position = parse_position(bits)?;
            let route_clearance = parse_route_clearance(bits)?;
            Ok(CpdlcElementBody::RouteClearance(RouteClearance {
                remaining_bits: route_clearance.remaining_bits,
                ..route_clearance.with_position(position)
            }))
        }
        (PduKind::Downlink, 47) => Ok(CpdlcElementBody::BeaconCode(parse_beacon_code(bits)?)),
        (PduKind::Downlink, 62) => Ok(CpdlcElementBody::ErrorInformation(parse_error_information(
            bits,
        )?)),
        (PduKind::Downlink, 64) => Ok(CpdlcElementBody::IcaoFacilityDesignation(
            IcaoFacilityDesignation::Icao(parse_fixed_ia5(bits, 4)?),
        )),
        (PduKind::Downlink, 67 | 68) => Ok(CpdlcElementBody::FreeText(parse_free_text(bits)?)),
        (PduKind::Downlink, 71) => Ok(CpdlcElementBody::Degrees(parse_degrees(bits)?)),
        (PduKind::Downlink, 79) => Ok(CpdlcElementBody::AtisCode(parse_fixed_ia5(bits, 1)?)),
        (PduKind::Downlink, 73) => Ok(CpdlcElementBody::VersionNumber(bits.read_bits(4)? as u8)),
        (
            PduKind::Uplink,
            6 | 19 | 20 | 23 | 33 | 34 | 35 | 36 | 37 | 38 | 39 | 40 | 41 | 128 | 129 | 148 | 175,
        ) => Ok(CpdlcElementBody::Altitude(parse_altitude(bits)?)),
        (PduKind::Uplink, 13 | 15 | 17 | 21 | 24) => {
            let time = parse_time(bits)?;
            let altitude = parse_altitude(bits)?;
            Ok(CpdlcElementBody::TimeAltitude { time, altitude })
        }
        (PduKind::Uplink, 14 | 16 | 18 | 22 | 25 | 42 | 43 | 44 | 45 | 46 | 47 | 48 | 49 | 92) => {
            let position = parse_position(bits)?;
            let altitude = parse_altitude(bits)?;
            Ok(CpdlcElementBody::PositionAltitude { position, altitude })
        }
        (PduKind::Uplink, 26 | 28 | 150) => {
            let altitude = parse_altitude(bits)?;
            let time = parse_time(bits)?;
            Ok(CpdlcElementBody::AltitudeTime { altitude, time })
        }
        (PduKind::Uplink, 27 | 29 | 78 | 90 | 149) => {
            let altitude = parse_altitude(bits)?;
            let position = parse_position(bits)?;
            Ok(CpdlcElementBody::AltitudePosition { altitude, position })
        }
        (PduKind::Uplink, 30 | 31 | 32 | 180) => {
            let first = parse_altitude(bits)?;
            let second = parse_altitude(bits)?;
            Ok(CpdlcElementBody::AltitudeAltitude {
                altitude: first,
                altitude2: second,
            })
        }
        (PduKind::Uplink, 61) => {
            let position = parse_position(bits)?;
            let altitude = parse_altitude(bits)?;
            let speed = parse_speed(bits)?;
            Ok(CpdlcElementBody::PositionAltitudeSpeed {
                position,
                altitude,
                speed,
            })
        }
        (PduKind::Uplink, 51..=53) => {
            let position = parse_position(bits)?;
            let time = parse_time(bits)?;
            Ok(CpdlcElementBody::PositionTime { position, time })
        }
        (PduKind::Uplink, 54) => {
            let position = parse_position(bits)?;
            let times = [parse_time(bits)?, parse_time(bits)?];
            Ok(CpdlcElementBody::PositionTimeTime {
                position,
                time: times,
            })
        }
        (PduKind::Uplink, 59) => {
            let position = parse_position(bits)?;
            let time = parse_time(bits)?;
            let altitude = parse_altitude(bits)?;
            Ok(CpdlcElementBody::PositionTimeAltitude {
                position,
                time,
                altitude,
            })
        }
        (PduKind::Uplink, 62) => {
            let time = parse_time(bits)?;
            let position = parse_position(bits)?;
            let altitude = parse_altitude(bits)?;
            Ok(CpdlcElementBody::TimePositionAltitude {
                time,
                position,
                altitude,
            })
        }
        (PduKind::Uplink, 63) => {
            let time = parse_time(bits)?;
            let position = parse_position(bits)?;
            let altitude = parse_altitude(bits)?;
            let speed = parse_speed(bits)?;
            Ok(CpdlcElementBody::TimePositionAltitudeSpeed {
                time,
                position,
                altitude,
                speed,
            })
        }
        (PduKind::Uplink, 64 | 82 | 152) => {
            let distance = parse_distance_offset(bits)?;
            let direction = parse_direction(bits)?;
            Ok(CpdlcElementBody::DistanceOffsetDirection {
                distance_offset: distance,
                direction,
            })
        }
        (PduKind::Uplink, 65) => {
            let position = parse_position(bits)?;
            let distance = parse_distance_offset(bits)?;
            let direction = parse_direction(bits)?;
            Ok(CpdlcElementBody::PositionDistanceOffsetDirection {
                position,
                distance_offset: distance,
                direction,
            })
        }
        (PduKind::Uplink, 66) => {
            let time = parse_time(bits)?;
            let distance = parse_distance_offset(bits)?;
            let direction = parse_direction(bits)?;
            Ok(CpdlcElementBody::TimeDistanceOffsetDirection {
                time,
                distance_offset: distance,
                direction,
            })
        }
        (PduKind::Uplink, 7 | 9 | 69 | 71) => Ok(CpdlcElementBody::Time(parse_time(bits)?)),
        (PduKind::Uplink, 8 | 10 | 68 | 74 | 75 | 155) => {
            Ok(CpdlcElementBody::Position(parse_position(bits)?))
        }
        (PduKind::Uplink, 73) => Err(CpdlcDecodeError::PredepartureClearanceNotDecoded),
        (PduKind::Uplink, 79) => {
            let position = parse_position(bits)?;
            let route_clearance = parse_route_clearance(bits)?;
            Ok(CpdlcElementBody::RouteClearance(RouteClearance {
                remaining_bits: route_clearance.remaining_bits,
                ..route_clearance.with_position(position)
            }))
        }
        (PduKind::Uplink, 83) => {
            let position = parse_position(bits)?;
            let route_clearance = parse_route_clearance(bits)?;
            Ok(CpdlcElementBody::RouteClearance(RouteClearance {
                remaining_bits: route_clearance.remaining_bits,
                ..route_clearance.with_position(position)
            }))
        }
        (PduKind::Uplink, 80) => Ok(parse_route_clearance_body(bits, "uM80RouteClearance")),
        (PduKind::Uplink, 81) => Ok(CpdlcElementBody::ProcedureName(parse_procedure_name(bits)?)),
        (PduKind::Uplink, 77 | 88) => {
            let positions = [parse_position(bits)?, parse_position(bits)?];
            Ok(CpdlcElementBody::PositionPosition {
                position: positions,
            })
        }
        (PduKind::Uplink, 98) => {
            let direction = parse_direction(bits)?;
            let degrees = parse_degrees(bits)?;
            Ok(CpdlcElementBody::DirectionDegrees { direction, degrees })
        }
        (PduKind::Uplink, 100) => {
            let time = parse_time(bits)?;
            let speed = parse_speed(bits)?;
            Ok(CpdlcElementBody::TimeSpeed { time, speed })
        }
        (PduKind::Uplink, 93) => Ok(CpdlcElementBody::Time(parse_time(bits)?)),
        (PduKind::Uplink, 104) => {
            let position = parse_position(bits)?;
            let speeds = [parse_speed(bits)?, parse_speed(bits)?];
            Ok(CpdlcElementBody::PositionSpeedSpeed {
                position,
                speed: speeds,
            })
        }
        (PduKind::Uplink, 105) => {
            let altitude = parse_altitude(bits)?;
            let speeds = [parse_speed(bits)?, parse_speed(bits)?];
            Ok(CpdlcElementBody::AltitudeSpeedSpeed {
                altitude,
                speed: speeds,
            })
        }
        (PduKind::Uplink, 106 | 108 | 109 | 111 | 112 | 115 | 151) => {
            Ok(CpdlcElementBody::Speed(parse_speed(bits)?))
        }
        (PduKind::Uplink, 117 | 120) => {
            let unit = parse_icao_unit_name(bits)?;
            let frequency = parse_frequency(bits)?;
            Ok(CpdlcElementBody::IcaoUnitNameFrequency {
                icao_unit_name: unit,
                frequency,
            })
        }
        (PduKind::Uplink, 118 | 121) => {
            let position = parse_position(bits)?;
            let unit = parse_icao_unit_name(bits)?;
            let frequency = parse_frequency(bits)?;
            Ok(CpdlcElementBody::PositionIcaoUnitNameFrequency {
                position,
                icao_unit_name: unit,
                frequency,
            })
        }
        (PduKind::Uplink, 119) => {
            let time = parse_time(bits)?;
            let unit = parse_icao_unit_name(bits)?;
            let frequency = parse_frequency(bits)?;
            Ok(CpdlcElementBody::TimeIcaoUnitNameFrequency {
                time,
                icao_unit_name: unit,
                frequency,
            })
        }
        (PduKind::Uplink, 123) => Ok(CpdlcElementBody::BeaconCode(parse_beacon_code(bits)?)),
        (PduKind::Uplink, 130) => Ok(CpdlcElementBody::Position(parse_position(bits)?)),
        (PduKind::Uplink, 153) => Ok(CpdlcElementBody::Altimeter(parse_altimeter(bits)?)),
        (PduKind::Uplink, 157) => Ok(CpdlcElementBody::Frequency(parse_frequency(bits)?)),
        (PduKind::Uplink, 159) => Ok(CpdlcElementBody::ErrorInformation(parse_error_information(
            bits,
        )?)),
        (PduKind::Uplink, 160) => Ok(CpdlcElementBody::IcaoFacilityDesignation(
            IcaoFacilityDesignation::Icao(parse_fixed_ia5(bits, 4)?),
        )),
        (PduKind::Uplink, 163) => {
            let facility = parse_fixed_ia5(bits, 4)?;
            let table = if bits.read_bool()? {
                Tp4Table::LabelB
            } else {
                Tp4Table::LabelA
            };
            Ok(CpdlcElementBody::IcaoFacilityDesignationTp4Table {
                icao_facility_designation: IcaoFacilityDesignation::Icao(facility),
                tp4_table: table,
            })
        }
        (PduKind::Uplink, 169 | 170) => Ok(CpdlcElementBody::FreeText(parse_free_text(bits)?)),
        _ => Ok(CpdlcElementBody::Unsupported),
    }
}

fn parse_position(bits: &mut BitReader<'_>) -> Result<CpdlcPosition, CpdlcDecodeError> {
    match bits.read_bits(3)? {
        0 => {
            let len = bits.read_bits(3)? as usize + 1;
            Ok(CpdlcPosition::FixName(parse_token_ia5(bits, len)?))
        }
        1 => {
            let len = bits.read_bits(2)? as usize + 1;
            Ok(CpdlcPosition::Navaid(parse_token_ia5(bits, len)?))
        }
        2 => Ok(CpdlcPosition::Airport(parse_token_ia5(bits, 4)?)),
        3 => {
            let latitude = parse_latitude(bits)?;
            let longitude = parse_longitude(bits)?;
            Ok(CpdlcPosition::LatitudeLongitude {
                latitude,
                longitude,
            })
        }
        4 => Ok(CpdlcPosition::UnsupportedPlaceBearingDistance),
        other => Err(CpdlcDecodeError::InvalidPositionChoice(other)),
    }
}

fn parse_latitude(bits: &mut BitReader<'_>) -> Result<f64, CpdlcDecodeError> {
    let has_minutes = bits.read_bool()?;
    let deg = bits.read_bits(7)? as f64;
    let min = if has_minutes {
        bits.read_bits(10)? as f64 / 10.0
    } else {
        0.0
    };
    let mut value = deg + min / 60.0;
    if bits.read_bool()? {
        value = -value;
    }
    Ok(value)
}

fn parse_longitude(bits: &mut BitReader<'_>) -> Result<f64, CpdlcDecodeError> {
    let has_minutes = bits.read_bool()?;
    let deg = bits.read_bits(8)? as f64;
    let min = if has_minutes {
        bits.read_bits(10)? as f64 / 10.0
    } else {
        0.0
    };
    let mut value = deg + min / 60.0;
    if bits.read_bool()? {
        value = -value;
    }
    Ok(value)
}

impl RouteClearance {
    fn with_position(mut self, position: CpdlcPosition) -> Self {
        self.route_information
            .get_or_insert_with(Vec::new)
            .insert(0, RouteInformation::Unsupported { choice: 255 });
        if let Some(route) = &mut self.route_information {
            route.insert(
                0,
                match position {
                    CpdlcPosition::FixName(fix) => RouteInformation::PublishedIdentifier {
                        fix,
                        position: None,
                    },
                    CpdlcPosition::LatitudeLongitude {
                        latitude,
                        longitude,
                    } => RouteInformation::LatitudeLongitude {
                        latitude,
                        longitude,
                    },
                    CpdlcPosition::Navaid(value) | CpdlcPosition::Airport(value) => {
                        RouteInformation::PublishedIdentifier {
                            fix: value,
                            position: None,
                        }
                    }
                    CpdlcPosition::UnsupportedPlaceBearingDistance => {
                        RouteInformation::Unsupported { choice: 4 }
                    }
                },
            );
        }
        self
    }
}

fn parse_route_clearance_body(bits: &mut BitReader<'_>, _name: &str) -> CpdlcElementBody {
    let mut trial = bits.clone();
    match parse_route_clearance(&mut trial) {
        Ok(route) => {
            *bits = trial;
            CpdlcElementBody::RouteClearance(route)
        }
        Err(_) => CpdlcElementBody::OpaqueRouteClearance {
            remaining_bits: bits.remaining(),
            payload_hex: bits.read_remaining_bits_hex(),
        },
    }
}

fn parse_route_clearance(bits: &mut BitReader<'_>) -> Result<RouteClearance, CpdlcDecodeError> {
    let has_airport_departure = bits.read_bool()?;
    let has_airport_destination = bits.read_bool()?;
    let has_runway_departure = bits.read_bool()?;
    let has_procedure_departure = bits.read_bool()?;
    let has_runway_arrival = bits.read_bool()?;
    let has_procedure_approach = bits.read_bool()?;
    let has_procedure_arrival = bits.read_bool()?;
    let has_airway_intercept = bits.read_bool()?;
    let has_route_information = bits.read_bool()?;
    let _has_additional_route_information = bits.read_bool()?;

    let airport_departure = if has_airport_departure {
        Some(parse_token_ia5(bits, 4)?)
    } else {
        None
    };
    let airport_destination = if has_airport_destination {
        Some(parse_token_ia5(bits, 4)?)
    } else {
        None
    };
    let runway_departure = if has_runway_departure {
        Some(parse_runway(bits)?)
    } else {
        None
    };
    let procedure_departure = if has_procedure_departure {
        Some(parse_procedure_name(bits)?.procedure)
    } else {
        None
    };
    let runway_arrival = if has_runway_arrival {
        Some(parse_runway(bits)?)
    } else {
        None
    };
    let procedure_approach = if has_procedure_approach {
        Some(parse_procedure_name(bits)?.procedure)
    } else {
        None
    };
    let procedure_arrival = if has_procedure_arrival {
        Some(parse_procedure_name(bits)?.procedure)
    } else {
        None
    };
    let airway_intercept = if has_airway_intercept {
        let len = bits.read_bits(3)? as usize + 1;
        Some(parse_token_ia5(bits, len)?)
    } else {
        None
    };
    let route_information = if has_route_information {
        let len = bits.read_bits(7)? as usize + 1;
        let mut route = Vec::with_capacity(len);
        for _ in 0..len {
            route.push(parse_route_information(bits)?);
        }
        Some(route)
    } else {
        None
    };

    Ok(RouteClearance {
        airport_departure,
        airport_destination,
        runway_departure,
        procedure_departure,
        runway_arrival,
        procedure_approach,
        procedure_arrival,
        airway_intercept,
        route_information,
        remaining_bits: bits.remaining(),
    })
}

fn parse_route_information(bits: &mut BitReader<'_>) -> Result<RouteInformation, CpdlcDecodeError> {
    match bits.read_bits(3)? as u8 {
        0 => {
            let has_position = bits.read_bool()?;
            let len = bits.read_bits(3)? as usize + 1;
            let fix = parse_token_ia5(bits, len)?;
            let position = if has_position {
                let latitude = parse_latitude(bits)?;
                let longitude = parse_longitude(bits)?;
                Some(CpdlcPosition::LatitudeLongitude {
                    latitude,
                    longitude,
                })
            } else {
                None
            };
            Ok(RouteInformation::PublishedIdentifier { fix, position })
        }
        1 => {
            let latitude = parse_latitude(bits)?;
            let longitude = parse_longitude(bits)?;
            Ok(RouteInformation::LatitudeLongitude {
                latitude,
                longitude,
            })
        }
        4 => {
            let len = bits.read_bits(3)? as usize + 1;
            Ok(RouteInformation::Airway(parse_token_ia5(bits, len)?))
        }
        5 => {
            let len = bits.read_bits(3)? as usize + 1;
            Ok(RouteInformation::Track {
                name: parse_token_ia5(bits, len)?,
            })
        }
        choice => Ok(RouteInformation::Unsupported { choice }),
    }
}

fn parse_runway(bits: &mut BitReader<'_>) -> Result<String, CpdlcDecodeError> {
    let direction = bits.read_bits(6)? as u8 + 1;
    let configuration = match bits.read_bits(2)? {
        0 => "L",
        1 => "R",
        2 => "C",
        3 => "",
        _ => unreachable!(),
    };
    Ok(format!("{direction:02}{configuration}"))
}

fn parse_position_report(
    bits: &mut BitReader<'_>,
) -> Result<CpdlcPositionReport, CpdlcDecodeError> {
    let has_next_fix = bits.read_bool()?;
    let has_next_fix_eta = bits.read_bool()?;
    let has_next_plus_one_fix = bits.read_bool()?;
    let has_destination_eta = bits.read_bool()?;
    let has_remaining_fuel = bits.read_bool()?;
    let has_temperature = bits.read_bool()?;
    let has_winds = bits.read_bool()?;
    let has_turbulence = bits.read_bool()?;
    let has_icing = bits.read_bool()?;
    let has_speed = bits.read_bool()?;
    let has_ground_speed = bits.read_bool()?;
    let has_vertical_change = bits.read_bool()?;
    let has_track_angle = bits.read_bool()?;
    let has_true_heading = bits.read_bool()?;
    let has_distance = bits.read_bool()?;
    let has_supplementary_information = bits.read_bool()?;
    let has_reported_waypoint_position = bits.read_bool()?;
    let has_reported_waypoint_time = bits.read_bool()?;
    let has_reported_waypoint_altitude = bits.read_bool()?;

    let current_position = parse_position(bits)?;
    let current_time = parse_time(bits)?;
    let altitude = parse_altitude(bits)?;

    Ok(CpdlcPositionReport {
        current_position,
        current_time,
        altitude,
        next_fix: has_next_fix.then(|| parse_position(bits)).transpose()?,
        next_fix_eta: has_next_fix_eta.then(|| parse_time(bits)).transpose()?,
        next_plus_one_fix: has_next_plus_one_fix
            .then(|| parse_position(bits))
            .transpose()?,
        destination_eta: has_destination_eta.then(|| parse_time(bits)).transpose()?,
        remaining_fuel: has_remaining_fuel
            .then(|| parse_remaining_fuel(bits))
            .transpose()?,
        temperature: has_temperature
            .then(|| parse_temperature(bits))
            .transpose()?,
        winds: has_winds.then(|| parse_winds(bits)).transpose()?,
        turbulence: has_turbulence
            .then(|| bits.read_bits(2).map(|value| value as u8))
            .transpose()?,
        icing: has_icing
            .then(|| bits.read_bits(2).map(|value| value as u8))
            .transpose()?,
        speed: has_speed.then(|| parse_speed(bits)).transpose()?,
        ground_speed_knots: has_ground_speed
            .then(|| bits.read_bits(6).map(|value| (value as u16 + 7) * 10))
            .transpose()?,
        vertical_change: has_vertical_change
            .then(|| parse_vertical_change(bits))
            .transpose()?,
        track_angle: has_track_angle.then(|| parse_degrees(bits)).transpose()?,
        true_heading: has_true_heading.then(|| parse_degrees(bits)).transpose()?,
        distance: has_distance.then(|| parse_distance(bits)).transpose()?,
        supplementary_information: has_supplementary_information
            .then(|| parse_free_text(bits))
            .transpose()?,
        reported_waypoint_position: has_reported_waypoint_position
            .then(|| parse_position(bits))
            .transpose()?,
        reported_waypoint_time: has_reported_waypoint_time
            .then(|| parse_time(bits))
            .transpose()?,
        reported_waypoint_altitude: has_reported_waypoint_altitude
            .then(|| parse_altitude(bits))
            .transpose()?,
    })
}

fn parse_remaining_fuel(bits: &mut BitReader<'_>) -> Result<CpdlcTime, CpdlcDecodeError> {
    Ok(CpdlcTime {
        hour: bits.read_bits(5)? as u8,
        minute: bits.read_bits(6)? as u8,
    })
}

fn parse_temperature(bits: &mut BitReader<'_>) -> Result<CpdlcTemperature, CpdlcDecodeError> {
    if bits.read_bool()? {
        Ok(CpdlcTemperature::Fahrenheit(
            bits.read_bits(8)? as i16 - 105,
        ))
    } else {
        Ok(CpdlcTemperature::Celsius(bits.read_bits(7)? as i16 - 80))
    }
}

fn parse_winds(bits: &mut BitReader<'_>) -> Result<CpdlcWinds, CpdlcDecodeError> {
    let direction_degrees = bits.read_bits(9)? as u16 + 1;
    let speed = if bits.read_bool()? {
        CpdlcWindSpeed::Kmh(bits.read_bits(9)? as u16)
    } else {
        CpdlcWindSpeed::Knots(bits.read_bits(8)? as u16)
    };
    Ok(CpdlcWinds {
        direction_degrees,
        speed,
    })
}

fn parse_vertical_change(
    bits: &mut BitReader<'_>,
) -> Result<CpdlcVerticalChange, CpdlcDecodeError> {
    let direction = if bits.read_bool()? {
        CpdlcVerticalDirection::Down
    } else {
        CpdlcVerticalDirection::Up
    };
    let rate = if bits.read_bool()? {
        CpdlcVerticalRate::MetersPerMinute(bits.read_bits(8)? as u16 * 10)
    } else {
        CpdlcVerticalRate::FeetPerMinute(bits.read_bits(6)? as u16 * 100)
    };
    Ok(CpdlcVerticalChange { direction, rate })
}

fn parse_distance(bits: &mut BitReader<'_>) -> Result<CpdlcDistance, CpdlcDecodeError> {
    if bits.read_bool()? {
        Ok(CpdlcDistance::Kilometers(bits.read_bits(10)? as u16 + 1))
    } else {
        Ok(CpdlcDistance::NauticalMiles(bits.read_bits(14)? as u16))
    }
}

fn parse_speed(bits: &mut BitReader<'_>) -> Result<CpdlcSpeed, CpdlcDecodeError> {
    match bits.read_bits(3)? {
        0 => Ok(CpdlcSpeed::IndicatedKnots(bits.read_bits(5)? as u16 + 7)),
        1 => Ok(CpdlcSpeed::IndicatedKmh(bits.read_bits(7)? as u16 + 10)),
        2 => Ok(CpdlcSpeed::TrueKnots(bits.read_bits(6)? as u16 + 7)),
        3 => Ok(CpdlcSpeed::TrueKmh(bits.read_bits(7)? as u16 + 10)),
        4 => Ok(CpdlcSpeed::GroundKnots(bits.read_bits(6)? as u16 + 7)),
        5 => Ok(CpdlcSpeed::GroundKmh(bits.read_bits(8)? as u16 + 10)),
        6 => Ok(CpdlcSpeed::Mach(bits.read_bits(5)? as u16 + 61)),
        7 => Ok(CpdlcSpeed::MachLarge(bits.read_bits(9)? as u16 + 93)),
        _ => unreachable!(),
    }
}

fn parse_altimeter(bits: &mut BitReader<'_>) -> Result<CpdlcAltimeter, CpdlcDecodeError> {
    match bits.read_bits(1)? {
        0 => Ok(CpdlcAltimeter::InHgHundredths(
            bits.read_bits(10)? as u16 + 2200,
        )),
        1 => Ok(CpdlcAltimeter::HectoPascals(
            bits.read_bits(13)? as u16 + 7500,
        )),
        _ => unreachable!(),
    }
}

fn parse_error_information(
    bits: &mut BitReader<'_>,
) -> Result<CpdlcErrorInformation, CpdlcDecodeError> {
    match bits.read_bits(5)? {
        0 => Ok(CpdlcErrorInformation::ApplicationError),
        1 => Ok(CpdlcErrorInformation::DuplicateMsgIdentificationNumber),
        2 => Ok(CpdlcErrorInformation::UnrecognizedMsgReferenceNumber),
        3 => Ok(CpdlcErrorInformation::EndServiceWithPendingMsgs),
        4 => Ok(CpdlcErrorInformation::EndServiceWithNoValidResponse),
        5 => Ok(CpdlcErrorInformation::InsufficientMsgStorageCapacity),
        6 => Ok(CpdlcErrorInformation::NoAvailableMsgIdentificationNumber),
        7 => Ok(CpdlcErrorInformation::CommandedTermination),
        8 => Ok(CpdlcErrorInformation::InsufficientData),
        9 => Ok(CpdlcErrorInformation::UnexpectedData),
        10 => Ok(CpdlcErrorInformation::InvalidData),
        11 => Ok(CpdlcErrorInformation::ReservedErrorMsg1),
        12 => Ok(CpdlcErrorInformation::ReservedErrorMsg2),
        13 => Ok(CpdlcErrorInformation::ReservedErrorMsg3),
        14 => Ok(CpdlcErrorInformation::ReservedErrorMsg4),
        15 => Ok(CpdlcErrorInformation::ReservedErrorMsg5),
        16 => Ok(CpdlcErrorInformation::ReservedErrorMsg6),
        other => Err(CpdlcDecodeError::InvalidErrorInformation(other)),
    }
}

fn parse_altitude(bits: &mut BitReader<'_>) -> Result<CpdlcAltitude, CpdlcDecodeError> {
    match bits.read_bits(3)? as u8 {
        0 => Ok(CpdlcAltitude::QnhFeet(bits.read_bits(12)? as u16)),
        1 => Ok(CpdlcAltitude::QnhMeters(bits.read_bits(14)? as u16)),
        2 => Ok(CpdlcAltitude::QfeFeet(bits.read_bits(12)? as u16)),
        3 => Ok(CpdlcAltitude::QfeMeters(bits.read_bits(13)? as u16)),
        4 => Ok(CpdlcAltitude::GnssFeet(bits.read_bits(18)? as u32)),
        5 => Ok(CpdlcAltitude::GnssMeters(bits.read_bits(16)? as u32)),
        6 => Ok(CpdlcAltitude::FlightLevel(bits.read_bits(10)? as u16 + 30)),
        7 => Ok(CpdlcAltitude::FlightLevelMetric(
            bits.read_bits(11)? as u16 + 100,
        )),
        _ => unreachable!(),
    }
}

fn parse_time(bits: &mut BitReader<'_>) -> Result<CpdlcTime, CpdlcDecodeError> {
    Ok(CpdlcTime {
        hour: bits.read_bits(5)? as u8,
        minute: bits.read_bits(6)? as u8,
    })
}

fn parse_distance_offset(bits: &mut BitReader<'_>) -> Result<DistanceOffset, CpdlcDecodeError> {
    match bits.read_bits(1)? {
        0 => Ok(DistanceOffset::Nm(bits.read_bits(7)? as u16 + 1)),
        1 => Ok(DistanceOffset::Km(bits.read_bits(8)? as u16 + 1)),
        _ => unreachable!(),
    }
}

fn parse_direction(bits: &mut BitReader<'_>) -> Result<CpdlcDirection, CpdlcDecodeError> {
    match bits.read_bits(4)? {
        0 => Ok(CpdlcDirection::Left),
        1 => Ok(CpdlcDirection::Right),
        2 => Ok(CpdlcDirection::EitherSide),
        3 => Ok(CpdlcDirection::North),
        4 => Ok(CpdlcDirection::South),
        5 => Ok(CpdlcDirection::East),
        6 => Ok(CpdlcDirection::West),
        7 => Ok(CpdlcDirection::NorthEast),
        8 => Ok(CpdlcDirection::NorthWest),
        9 => Ok(CpdlcDirection::SouthEast),
        10 => Ok(CpdlcDirection::SouthWest),
        other => Err(CpdlcDecodeError::InvalidDirectionEnum(other)),
    }
}

fn parse_beacon_code(bits: &mut BitReader<'_>) -> Result<String, CpdlcDecodeError> {
    let mut out = String::with_capacity(4);
    for _ in 0..4 {
        out.push(char::from(b'0' + bits.read_bits(3)? as u8));
    }
    Ok(out)
}

fn parse_degrees(bits: &mut BitReader<'_>) -> Result<CpdlcDegrees, CpdlcDecodeError> {
    let value = bits.read_bits(9)? as u16 + 1;
    if bits.read_bool()? {
        Ok(CpdlcDegrees::True(value))
    } else {
        Ok(CpdlcDegrees::Magnetic(value))
    }
}

fn parse_procedure_name(bits: &mut BitReader<'_>) -> Result<CpdlcProcedureName, CpdlcDecodeError> {
    let has_transition = bits.read_bool()?;
    let procedure_type = match bits.read_bits(2)? {
        0 => CpdlcProcedureType::Arrival,
        1 => CpdlcProcedureType::Approach,
        2 => CpdlcProcedureType::Departure,
        other => return Err(CpdlcDecodeError::InvalidProcedureType(other)),
    };
    let procedure_len = bits.read_bits(3)? as usize + 1;
    let procedure = parse_ia5_chars(bits, procedure_len)?;
    let transition = if has_transition {
        let transition_len = bits.read_bits(3)? as usize + 1;
        Some(parse_ia5_chars(bits, transition_len)?)
    } else {
        None
    };
    Ok(CpdlcProcedureName {
        procedure_type,
        procedure,
        transition,
    })
}

fn parse_free_text(bits: &mut BitReader<'_>) -> Result<String, CpdlcDecodeError> {
    // FANSFreeText is IA5String (SIZE(1..256)) with 7-bit constrained chars.
    let len = bits.read_bits(8)? as usize + 1;
    parse_ia5_chars(bits, len)
}

fn parse_fixed_ia5(bits: &mut BitReader<'_>, len: usize) -> Result<String, CpdlcDecodeError> {
    parse_ia5_chars(bits, len)
}

fn parse_token_ia5(bits: &mut BitReader<'_>, len: usize) -> Result<String, CpdlcDecodeError> {
    let text = parse_ia5_chars(bits, len)?;
    if text
        .bytes()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b' ' || b == b'-')
    {
        Ok(text.trim_end().to_string())
    } else {
        Err(CpdlcDecodeError::InvalidIa5Token(text))
    }
}

fn parse_ia5_chars(bits: &mut BitReader<'_>, len: usize) -> Result<String, CpdlcDecodeError> {
    let mut out = String::with_capacity(len);
    for _ in 0..len {
        let ch = bits.read_bits(7)? as u8;
        out.push(char::from(ch));
    }
    Ok(out)
}

fn parse_icao_unit_name(bits: &mut BitReader<'_>) -> Result<IcaoUnitName, CpdlcDecodeError> {
    let facility = match bits.read_bits(1)? {
        0 => IcaoFacilityIdentification::Designation(parse_fixed_ia5(bits, 4)?),
        1 => {
            // SIZE(3..18): constrained length determinant has range 16 -> 4 bits.
            let len = bits.read_bits(4)? as usize + 3;
            IcaoFacilityIdentification::Name(parse_ia5_chars(bits, len)?)
        }
        _ => unreachable!(),
    };
    let function = match bits.read_bits(3)? {
        0 => IcaoFacilityFunction::Center,
        1 => IcaoFacilityFunction::Approach,
        2 => IcaoFacilityFunction::Tower,
        3 => IcaoFacilityFunction::Final,
        4 => IcaoFacilityFunction::GroundControl,
        5 => IcaoFacilityFunction::ClearanceDelivery,
        6 => IcaoFacilityFunction::Departure,
        7 => IcaoFacilityFunction::Control,
        _ => unreachable!(),
    };
    Ok(IcaoUnitName { facility, function })
}

fn parse_frequency(bits: &mut BitReader<'_>) -> Result<CpdlcFrequency, CpdlcDecodeError> {
    match bits.read_bits(2)? {
        0 => Ok(CpdlcFrequency::HfKhz(bits.read_bits(15)? as u32 + 2850)),
        1 => Ok(CpdlcFrequency::VhfKhz(bits.read_bits(15)? as u32 + 117_000)),
        2 => Ok(CpdlcFrequency::UhfKhz(bits.read_bits(18)? as u32 + 225_000)),
        3 => Ok(CpdlcFrequency::SatChannel(parse_numeric_string(bits, 12)?)),
        _ => unreachable!(),
    }
}

fn parse_numeric_string(bits: &mut BitReader<'_>, len: usize) -> Result<String, CpdlcDecodeError> {
    let mut out = String::with_capacity(len);
    for _ in 0..len {
        let code = bits.read_bits(4)? as u8;
        let ch = match code {
            0 => ' ',
            1..=10 => char::from(b'0' + (code - 1)),
            _ => return Err(CpdlcDecodeError::InvalidNumericStringCode(code)),
        };
        out.push(ch);
    }
    Ok(out)
}

fn element_info(kind: PduKind, id: u16) -> Option<&'static CpdlcElementInfo> {
    let catalog = cpdlc_catalog();
    let elements = match kind {
        PduKind::Downlink => &catalog.downlink,
        PduKind::Uplink => &catalog.uplink,
    };
    elements.get(id as usize).filter(|info| info.id == id)
}

fn cpdlc_catalog() -> &'static CpdlcCatalog {
    static CATALOG: OnceLock<CpdlcCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let catalog: RawCpdlcCatalog =
            serde_json::from_str(include_str!("../../../../data/cpdlc_fans.json"))
                .expect("valid bundled FANS-1/A CPDLC element catalog");
        catalog.into()
    })
}

/// Look up an ATN B1 element by direction and numeric ID.
pub(crate) fn atn_element_info(kind: PduKind, id: u16) -> Option<&'static CpdlcElementInfo> {
    let catalog = atn_catalog();
    let elements = match kind {
        PduKind::Uplink => &catalog.uplink,
        PduKind::Downlink => &catalog.downlink,
    };
    elements.iter().find(|info| info.id == id)
}

fn atn_catalog() -> &'static CpdlcCatalog {
    static ATN_CATALOG: OnceLock<CpdlcCatalog> = OnceLock::new();
    ATN_CATALOG.get_or_init(|| {
        let catalog: RawCpdlcCatalog =
            serde_json::from_str(include_str!("../../../../data/cpdlc_atn.json"))
                .expect("valid bundled ATN B1 CPDLC element catalog");
        catalog.into()
    })
}

impl From<RawCpdlcCatalog> for CpdlcCatalog {
    fn from(raw: RawCpdlcCatalog) -> Self {
        Self {
            uplink: raw.uplink.into_iter().map(Into::into).collect(),
            downlink: raw.downlink.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<RawCpdlcElementInfo> for CpdlcElementInfo {
    fn from(raw: RawCpdlcElementInfo) -> Self {
        let fragments: Vec<_> = parse_template_fragments(&raw.template).collect();
        let slots: Vec<_> = fragments
            .iter()
            .filter_map(|fragment| match fragment {
                CpdlcTemplateFragment::Slot(slot) => Some(*slot),
                CpdlcTemplateFragment::Text(_) => None,
            })
            .collect();
        Self {
            id: raw.id,
            catalog_name: raw.name,
            fragments,
            body_slots: (!slots.is_empty()).then_some(slots),
        }
    }
}

fn parse_template_fragments(template: &str) -> impl Iterator<Item = CpdlcTemplateFragment> + '_ {
    TemplateFragmentIter { rest: template }
}

struct TemplateFragmentIter<'a> {
    rest: &'a str,
}

impl Iterator for TemplateFragmentIter<'_> {
    type Item = CpdlcTemplateFragment;

    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() {
            return None;
        }
        let Some(start) = self.rest.find('[') else {
            let text = self.rest;
            self.rest = "";
            return Some(CpdlcTemplateFragment::Text(text.to_string()));
        };
        if start > 0 {
            let (text, rest) = self.rest.split_at(start);
            self.rest = rest;
            return Some(CpdlcTemplateFragment::Text(text.to_string()));
        }
        let Some(end) = self.rest.find(']') else {
            let text = self.rest;
            self.rest = "";
            return Some(CpdlcTemplateFragment::Text(text.to_string()));
        };
        let raw_slot = &self.rest[1..end];
        self.rest = &self.rest[end + 1..];
        Some(
            CpdlcTemplateSlot::parse(raw_slot)
                .map(CpdlcTemplateFragment::Slot)
                .unwrap_or_else(|| CpdlcTemplateFragment::Text(format!("[{raw_slot}]"))),
        )
    }
}

impl CpdlcTemplateSlot {
    fn as_str(self) -> &'static str {
        match self {
            Self::Altimeter => "altimeter",
            Self::Altitude => "altitude",
            Self::Altitude2 => "altitude2",
            Self::AtisCode => "atis_code",
            Self::BeaconCode => "beacon_code",
            Self::Degrees => "degrees",
            Self::Direction => "direction",
            Self::DistanceOffset => "distance_offset",
            Self::ErrorInformation => "error_information",
            Self::FreeText => "free_text",
            Self::Frequency => "frequency",
            Self::IcaoFacilityDesignation => "icao_facility_designation",
            Self::IcaoUnitName => "icao_unit_name",
            Self::Position => "position",
            Self::PositionReport => "position_report",
            Self::ProcedureName => "procedure_name",
            Self::RouteClearance => "route_clearance",
            Self::Speed => "speed",
            Self::Time => "time",
            Self::Tp4Table => "tp4_table",
            Self::VersionNumber => "version_number",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        let slot = value
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>();
        Some(match slot.as_str() {
            "altimeter" => Self::Altimeter,
            "altitude" => Self::Altitude,
            "altitude2" => Self::Altitude2,
            "atiscode" => Self::AtisCode,
            "beaconcode" => Self::BeaconCode,
            "degrees" => Self::Degrees,
            "direction" => Self::Direction,
            "distanceoffset" => Self::DistanceOffset,
            "errorinformation" => Self::ErrorInformation,
            "freetext" => Self::FreeText,
            "frequency" => Self::Frequency,
            "icaofacilitydesignation" => Self::IcaoFacilityDesignation,
            "icaounitname" => Self::IcaoUnitName,
            "position" => Self::Position,
            "positionreport" => Self::PositionReport,
            "procedurename" => Self::ProcedureName,
            "routeclearance" => Self::RouteClearance,
            "speed" => Self::Speed,
            "time" => Self::Time,
            "tp4table" => Self::Tp4Table,
            "versionnumber" => Self::VersionNumber,
            _ => return None,
        })
    }
}

pub(crate) fn cpdlc_phrase_fragments<'a>(
    info: &'a CpdlcElementInfo,
    body: Option<&'a CpdlcElementBody>,
) -> impl Iterator<Item = CpdlcPhraseFragment> + 'a {
    info.fragments
        .iter()
        .filter_map(move |fragment| match fragment {
            CpdlcTemplateFragment::Text(text) => Some(CpdlcPhraseFragment::Text(text.clone())),
            // emit unresolved references (some messages may be corrupted, let downstream handle)
            CpdlcTemplateFragment::Slot(slot) => (body.is_none()
                || body.is_some_and(|body| body.contains_slot(*slot)))
            .then_some(CpdlcPhraseFragment::Value(*slot)),
        })
}

impl CpdlcElementBody {
    fn contains_slot(&self, slot: CpdlcTemplateSlot) -> bool {
        let slot = slot.as_str();
        let Ok(body) = serde_json::to_value(self) else {
            return false;
        };

        body.get("kind").and_then(Value::as_str) == Some(slot)
            || body
                .get("data")
                .and_then(Value::as_object)
                .is_some_and(|object| object.contains_key(slot))
            || matches!(
                (slot, self),
                ("route_clearance", Self::OpaqueRouteClearance { .. })
            )
    }
}

fn parse_header(bits: &mut BitReader<'_>) -> Result<AtcMessageHeader, CpdlcDecodeError> {
    // FANSATCMessageHeader has two optional root members:
    // msgReferenceNumber and timestamp.
    let has_msg_ref = bits.read_bool()?;
    let has_timestamp = bits.read_bool()?;
    let msg_id = bits.read_bits(6)? as u8;
    let msg_ref = if has_msg_ref {
        Some(bits.read_bits(6)? as u8)
    } else {
        None
    };
    let timestamp = if has_timestamp {
        Some(CpdlcTimestamp {
            hour: bits.read_bits(5)? as u8,
            minute: bits.read_bits(6)? as u8,
            second: bits.read_bits(6)? as u8,
        })
    } else {
        None
    };
    Ok(AtcMessageHeader {
        msg_id,
        msg_ref,
        timestamp,
    })
}

#[derive(Clone)]
struct BitReader<'a> {
    bytes: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, bit_pos: 0 }
    }

    fn read_bool(&mut self) -> Result<bool, CpdlcDecodeError> {
        Ok(self.read_bits(1)? != 0)
    }

    fn read_bits(&mut self, n: usize) -> Result<u64, CpdlcDecodeError> {
        if self.remaining() < n {
            return Err(CpdlcDecodeError::BitReadOutOfBounds {
                needed: n,
                remaining: self.remaining(),
                bit_pos: self.bit_pos,
            });
        }
        let mut out = 0u64;
        for _ in 0..n {
            let byte = self.bytes[self.bit_pos / 8];
            let bit = (byte >> (7 - (self.bit_pos % 8))) & 1;
            out = (out << 1) | u64::from(bit);
            self.bit_pos += 1;
        }
        Ok(out)
    }

    fn remaining(&self) -> usize {
        self.bytes.len() * 8 - self.bit_pos
    }

    fn read_remaining_bits_hex(&mut self) -> String {
        let mut value = 0u8;
        let mut count = 0usize;
        let mut out = String::new();
        while self.remaining() > 0 {
            value = (value << 1) | u8::from(self.read_bool().unwrap_or(false));
            count += 1;
            if count == 4 {
                out.push(
                    char::from_digit(value as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
                value = 0;
                count = 0;
            }
        }
        if count > 0 {
            value <<= 4 - count;
            out.push(
                char::from_digit(value as u32, 16)
                    .unwrap()
                    .to_ascii_uppercase(),
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_short_at1_fixture() {
        let msg = parse_cpdlc_payload_hex("671C87E800").unwrap();
        assert_eq!(msg.payload_len_bytes, 5);
        assert!(msg.downlink.is_some());
        assert!(msg.uplink.is_some());
    }

    #[test]
    fn parses_cr1_fixture() {
        let msg = parse_cpdlc_payload_hex("21221BE8E5DAAF64").unwrap();
        assert_eq!(msg.payload_len_bytes, 8);
        assert!(msg.downlink.is_some() || msg.uplink.is_some());
    }

    #[test]
    fn parsed_catalog_slots_resolve_against_body_serde_names() {
        for kind in [PduKind::Uplink, PduKind::Downlink] {
            let elements = match kind {
                PduKind::Uplink => &cpdlc_catalog().uplink,
                PduKind::Downlink => &cpdlc_catalog().downlink,
            };

            for info in elements {
                let Some(slots) = &info.body_slots else {
                    continue;
                };
                let mut bits = BitReader::new(&[0x55; 256]);
                let Ok(body) = parse_element_body(&mut bits, kind, info.id) else {
                    continue;
                };
                if matches!(body, CpdlcElementBody::Unsupported) {
                    continue;
                }

                for slot in slots {
                    assert!(
                        body.contains_slot(*slot),
                        "{} {kind:?} id {} body {:?} does not expose template slot {:?}",
                        info.catalog_name,
                        info.id,
                        body,
                        slot
                    );
                }
            }
        }
    }
}
