use std::sync::OnceLock;

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};

use crate::decode::acars::MessageDirection;
use crate::decode::payload::PayloadError;
use crate::decode::{DecodeError, DecodeResult};

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
#[serde(tag = "type", rename_all = "snake_case")]
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

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct CpdlcElement {
    pub id: u16,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<CpdlcElementBody>,
    pub is_additional: bool,
}

impl Serialize for CpdlcElement {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let body = self
            .body
            .as_ref()
            .filter(|body| !matches!(body, CpdlcElementBody::Null));
        let include_template = self.template.is_some()
            && !matches!(body, Some(CpdlcElementBody::FreeText(_)))
            && (body.is_some() || matches!(self.body, Some(CpdlcElementBody::Null)));
        let mut len = 3;
        if include_template {
            len += 1;
        }
        if body.is_some() {
            len += 1;
        }
        let mut map = serializer.serialize_map(Some(len))?;
        map.serialize_entry("id", &self.id)?;
        map.serialize_entry("name", &self.name)?;
        if include_template {
            map.serialize_entry("template", &self.template)?;
        }
        if let Some(body) = body {
            map.serialize_entry("body", body)?;
        }
        map.serialize_entry("is_additional", &self.is_additional)?;
        map.end()
    }
}

#[derive(Debug, Deserialize)]
struct CpdlcCatalog {
    uplink: Vec<CpdlcElementInfo>,
    downlink: Vec<CpdlcElementInfo>,
}

#[derive(Debug, Deserialize)]
struct CpdlcElementInfo {
    id: u16,
    name: String,
    template: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub enum CpdlcElementBody {
    Null,
    Altitude(CpdlcAltitude),
    AltitudeTime {
        altitude: CpdlcAltitude,
        time: CpdlcTime,
    },
    FreeText(String),
    IcaoFacilityDesignation(String),
    IcaoFacilityDesignationTp4Table {
        facility: String,
        table: Tp4Table,
    },
    IcaoUnitNameFrequency {
        unit: IcaoUnitName,
        frequency: CpdlcFrequency,
    },
    TimeIcaoUnitNameFrequency {
        time: CpdlcTime,
        unit: IcaoUnitName,
        frequency: CpdlcFrequency,
    },
    DistanceOffsetDirection {
        distance: DistanceOffset,
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
        first: CpdlcAltitude,
        second: CpdlcAltitude,
    },
    TimeAltitude {
        time: CpdlcTime,
        altitude: CpdlcAltitude,
    },
    PositionDistanceOffsetDirection {
        position: CpdlcPosition,
        distance: DistanceOffset,
        direction: CpdlcDirection,
    },
    PositionIcaoUnitNameFrequency {
        position: CpdlcPosition,
        unit: IcaoUnitName,
        frequency: CpdlcFrequency,
    },
    TimeDistanceOffsetDirection {
        time: CpdlcTime,
        distance: DistanceOffset,
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
        speeds: [CpdlcSpeed; 2],
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
        times: [CpdlcTime; 2],
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
        speeds: [CpdlcSpeed; 2],
    },
    PositionPosition {
        positions: [CpdlcPosition; 2],
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

impl Serialize for CpdlcElementBody {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        match self {
            Self::Null => map.serialize_entry("null", &true)?,
            Self::Altitude(altitude) => map.serialize_entry("altitude", altitude)?,
            Self::AltitudeTime { altitude, time } => {
                map.serialize_entry("altitude", altitude)?;
                map.serialize_entry("time", time)?;
            }
            Self::FreeText(text) => map.serialize_entry("free_text", text)?,
            Self::IcaoFacilityDesignation(facility) => {
                map.serialize_entry("facility", &FacilityJson::Icao(facility))?;
            }
            Self::IcaoFacilityDesignationTp4Table { facility, table } => {
                map.serialize_entry("facility", &FacilityJson::Icao(facility))?;
                map.serialize_entry("tp4_table", table)?;
            }
            Self::IcaoUnitNameFrequency { unit, frequency } => {
                map.serialize_entry("icao_unit", unit)?;
                map.serialize_entry("frequency", frequency)?;
            }
            Self::TimeIcaoUnitNameFrequency {
                time,
                unit,
                frequency,
            } => {
                map.serialize_entry("time", time)?;
                map.serialize_entry("icao_unit", unit)?;
                map.serialize_entry("frequency", frequency)?;
            }
            Self::DistanceOffsetDirection {
                distance,
                direction,
            } => {
                map.serialize_entry("distance", distance)?;
                map.serialize_entry("direction", direction)?;
            }
            Self::Position(position) => map.serialize_entry("position", position)?,
            Self::PositionAltitude { position, altitude } => {
                map.serialize_entry("position", position)?;
                map.serialize_entry("altitude", altitude)?;
            }
            Self::AltitudePosition { altitude, position } => {
                map.serialize_entry("altitude", altitude)?;
                map.serialize_entry("position", position)?;
            }
            Self::AltitudeAltitude { first, second } => {
                map.serialize_entry("altitude", first)?;
                map.serialize_entry("altitude2", second)?;
            }
            Self::TimeAltitude { time, altitude } => {
                map.serialize_entry("time", time)?;
                map.serialize_entry("altitude", altitude)?;
            }
            Self::PositionDistanceOffsetDirection {
                position,
                distance,
                direction,
            } => {
                map.serialize_entry("position", position)?;
                map.serialize_entry("distance", distance)?;
                map.serialize_entry("direction", direction)?;
            }
            Self::PositionIcaoUnitNameFrequency {
                position,
                unit,
                frequency,
            } => {
                map.serialize_entry("position", position)?;
                map.serialize_entry("icao_unit", unit)?;
                map.serialize_entry("frequency", frequency)?;
            }
            Self::TimeDistanceOffsetDirection {
                time,
                distance,
                direction,
            } => {
                map.serialize_entry("time", time)?;
                map.serialize_entry("distance", distance)?;
                map.serialize_entry("direction", direction)?;
            }
            Self::Frequency(frequency) => map.serialize_entry("frequency", frequency)?,
            Self::Time(time) => map.serialize_entry("time", time)?,
            Self::DirectionDegrees { direction, degrees } => {
                map.serialize_entry("direction", direction)?;
                map.serialize_entry("degrees", degrees)?;
            }
            Self::Degrees(degrees) => map.serialize_entry("degrees", degrees)?,
            Self::AtisCode(code) => map.serialize_entry("atis", code)?,
            Self::ProcedureName(procedure) => map.serialize_entry("procedure", procedure)?,
            Self::Speed(speed) => map.serialize_entry("speed", speed)?,
            Self::TimeSpeed { time, speed } => {
                map.serialize_entry("time", time)?;
                map.serialize_entry("speed", speed)?;
            }
            Self::PositionSpeedSpeed { position, speeds } => {
                map.serialize_entry("position", position)?;
                map.serialize_entry("speeds", speeds)?;
            }
            Self::PositionAltitudeSpeed {
                position,
                altitude,
                speed,
            } => {
                map.serialize_entry("position", position)?;
                map.serialize_entry("altitude", altitude)?;
                map.serialize_entry("speed", speed)?;
            }
            Self::PositionTime { position, time } => {
                map.serialize_entry("position", position)?;
                map.serialize_entry("time", time)?;
            }
            Self::PositionTimeTime { position, times } => {
                map.serialize_entry("position", position)?;
                map.serialize_entry("times", times)?;
            }
            Self::TimePositionAltitude {
                time,
                position,
                altitude,
            } => {
                map.serialize_entry("time", time)?;
                map.serialize_entry("position", position)?;
                map.serialize_entry("altitude", altitude)?;
            }
            Self::PositionTimeAltitude {
                position,
                time,
                altitude,
            } => {
                map.serialize_entry("position", position)?;
                map.serialize_entry("time", time)?;
                map.serialize_entry("altitude", altitude)?;
            }
            Self::TimePositionAltitudeSpeed {
                time,
                position,
                altitude,
                speed,
            } => {
                map.serialize_entry("time", time)?;
                map.serialize_entry("position", position)?;
                map.serialize_entry("altitude", altitude)?;
                map.serialize_entry("speed", speed)?;
            }
            Self::AltitudeSpeedSpeed { altitude, speeds } => {
                map.serialize_entry("altitude", altitude)?;
                map.serialize_entry("speeds", speeds)?;
            }
            Self::PositionPosition { positions } => map.serialize_entry("positions", positions)?,
            Self::PositionReport(report) => map.serialize_entry("position_report", report)?,
            Self::ErrorInformation(error) => map.serialize_entry("error", error)?,
            Self::Altimeter(altimeter) => map.serialize_entry("altimeter", altimeter)?,
            Self::RouteClearance(route) => map.serialize_entry("route_clearance", route)?,
            Self::OpaqueRouteClearance {
                remaining_bits,
                payload_hex,
            } => {
                let opaque = OpaquePayloadJson {
                    hex: payload_hex,
                    remaining_bits: *remaining_bits,
                };
                map.serialize_entry("route_clearance", &opaque)?;
            }
            Self::BeaconCode(code) => map.serialize_entry("beacon_code", code)?,
            Self::VersionNumber(version) => map.serialize_entry("version", version)?,
            Self::Unsupported => map.serialize_entry("unsupported", &true)?,
        }
        map.end()
    }
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
#[serde(tag = "type", content = "value")]
pub enum RouteInformation {
    PublishedIdentifier { fix: String, position: Option<CpdlcPosition> },
    LatitudeLongitude { latitude: f64, longitude: f64 },
    Airway(String),
    Track { name: String },
    Unsupported { choice: u8 },
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
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

#[derive(Serialize)]
enum FacilityJson<'a> {
    #[serde(rename = "ICAO")]
    Icao(&'a str),
}

#[derive(Serialize)]
struct OpaquePayloadJson<'a> {
    hex: &'a str,
    remaining_bits: usize,
}

impl Serialize for CpdlcAltitude {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            Self::QnhFeet(value) => map.serialize_entry("QNH_ft", value)?,
            Self::QnhMeters(value) => map.serialize_entry("QNH_m", value)?,
            Self::QfeFeet(value) => map.serialize_entry("QFE_ft", value)?,
            Self::QfeMeters(value) => map.serialize_entry("QFE_m", value)?,
            Self::GnssFeet(value) => map.serialize_entry("GNSS_ft", value)?,
            Self::GnssMeters(value) => map.serialize_entry("GNSS_m", value)?,
            Self::FlightLevel(value) => map.serialize_entry("FL", value)?,
            Self::FlightLevelMetric(value) => map.serialize_entry("FL_m", value)?,
        }
        map.end()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CpdlcTime {
    pub hour: u8,
    pub minute: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
#[serde(tag = "type", content = "value")]
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
#[serde(tag = "type", content = "value")]
pub enum CpdlcFrequency {
    HfKhz(u32),
    VhfKhz(u32),
    UhfKhz(u32),
    SatChannel(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "unit", content = "value")]
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
#[serde(tag = "type", content = "value")]
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
#[serde(tag = "unit", content = "value")]
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
#[serde(tag = "unit", content = "value")]
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
#[serde(tag = "unit", content = "value")]
pub enum CpdlcVerticalRate {
    FeetPerMinute(u16),
    MetersPerMinute(u16),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "unit", content = "value")]
pub enum CpdlcDistance {
    NauticalMiles(u16),
    Kilometers(u16),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "reference", content = "value")]
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
#[serde(tag = "type", content = "value")]
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
#[serde(tag = "unit", content = "value")]
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

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CpdlcTimestamp {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl Serialize for CpdlcTimestamp {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!(
            "{:02}:{:02}:{:02}",
            self.hour, self.minute, self.second
        ))
    }
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
enum PduKind {
    Downlink,
    Uplink,
}

fn parse_pdu_summary(bytes: &[u8], kind: PduKind) -> Result<CpdlcPduSummary, String> {
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
) -> Result<CpdlcElement, String> {
    let id = bits.read_bits(8)? as u16;
    let info =
        element_info(kind, id).ok_or_else(|| format!("CPDLC element id {id} out of range"))?;
    let body = parse_element_body(bits, kind, id).ok();
    Ok(CpdlcElement {
        id,
        name: info.name.clone(),
        template: info.template.clone(),
        body,
        is_additional,
    })
}

fn parse_element_body(
    bits: &mut BitReader<'_>,
    kind: PduKind,
    element_id: u16,
) -> Result<CpdlcElementBody, String> {
    match (kind, element_id) {
        (PduKind::Downlink, id) if is_downlink_null(id) => Ok(CpdlcElementBody::Null),
        (PduKind::Uplink, id) if is_uplink_null(id) => Ok(CpdlcElementBody::Null),
        (PduKind::Downlink, 6 | 8 | 9 | 10 | 28 | 29 | 30 | 32 | 37 | 38 | 54 | 61 | 72) => {
            Ok(CpdlcElementBody::Altitude(parse_altitude(bits)?))
        }
        (PduKind::Downlink, 7 | 76 | 77) => {
            let first = parse_altitude(bits)?;
            let second = parse_altitude(bits)?;
            Ok(CpdlcElementBody::AltitudeAltitude { first, second })
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
                distance,
                direction,
            })
        }
        (PduKind::Downlink, 16) => {
            let position = parse_position(bits)?;
            let distance = parse_distance_offset(bits)?;
            let direction = parse_direction(bits)?;
            Ok(CpdlcElementBody::PositionDistanceOffsetDirection {
                position,
                distance,
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
        (PduKind::Downlink, 64) => Ok(CpdlcElementBody::IcaoFacilityDesignation(parse_fixed_ia5(
            bits, 4,
        )?)),
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
            Ok(CpdlcElementBody::AltitudeAltitude { first, second })
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
            Ok(CpdlcElementBody::PositionTimeTime { position, times })
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
                distance,
                direction,
            })
        }
        (PduKind::Uplink, 65) => {
            let position = parse_position(bits)?;
            let distance = parse_distance_offset(bits)?;
            let direction = parse_direction(bits)?;
            Ok(CpdlcElementBody::PositionDistanceOffsetDirection {
                position,
                distance,
                direction,
            })
        }
        (PduKind::Uplink, 66) => {
            let time = parse_time(bits)?;
            let distance = parse_distance_offset(bits)?;
            let direction = parse_direction(bits)?;
            Ok(CpdlcElementBody::TimeDistanceOffsetDirection {
                time,
                distance,
                direction,
            })
        }
        (PduKind::Uplink, 7 | 9 | 69 | 71) => Ok(CpdlcElementBody::Time(parse_time(bits)?)),
        (PduKind::Uplink, 8 | 10 | 68 | 74 | 75 | 155) => {
            Ok(CpdlcElementBody::Position(parse_position(bits)?))
        }
        (PduKind::Uplink, 73) => Err("predeparture clearance body is not decoded yet".to_string()),
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
            Ok(CpdlcElementBody::PositionPosition { positions })
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
            Ok(CpdlcElementBody::PositionSpeedSpeed { position, speeds })
        }
        (PduKind::Uplink, 105) => {
            let altitude = parse_altitude(bits)?;
            let speeds = [parse_speed(bits)?, parse_speed(bits)?];
            Ok(CpdlcElementBody::AltitudeSpeedSpeed { altitude, speeds })
        }
        (PduKind::Uplink, 106 | 108 | 109 | 111 | 112 | 115 | 151) => {
            Ok(CpdlcElementBody::Speed(parse_speed(bits)?))
        }
        (PduKind::Uplink, 117 | 120) => {
            let unit = parse_icao_unit_name(bits)?;
            let frequency = parse_frequency(bits)?;
            Ok(CpdlcElementBody::IcaoUnitNameFrequency { unit, frequency })
        }
        (PduKind::Uplink, 118 | 121) => {
            let position = parse_position(bits)?;
            let unit = parse_icao_unit_name(bits)?;
            let frequency = parse_frequency(bits)?;
            Ok(CpdlcElementBody::PositionIcaoUnitNameFrequency {
                position,
                unit,
                frequency,
            })
        }
        (PduKind::Uplink, 119) => {
            let time = parse_time(bits)?;
            let unit = parse_icao_unit_name(bits)?;
            let frequency = parse_frequency(bits)?;
            Ok(CpdlcElementBody::TimeIcaoUnitNameFrequency {
                time,
                unit,
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
        (PduKind::Uplink, 160) => Ok(CpdlcElementBody::IcaoFacilityDesignation(parse_fixed_ia5(
            bits, 4,
        )?)),
        (PduKind::Uplink, 163) => {
            let facility = parse_fixed_ia5(bits, 4)?;
            let table = if bits.read_bool()? {
                Tp4Table::LabelB
            } else {
                Tp4Table::LabelA
            };
            Ok(CpdlcElementBody::IcaoFacilityDesignationTp4Table { facility, table })
        }
        (PduKind::Uplink, 169 | 170) => Ok(CpdlcElementBody::FreeText(parse_free_text(bits)?)),
        _ => Ok(CpdlcElementBody::Unsupported),
    }
}

fn parse_position(bits: &mut BitReader<'_>) -> Result<CpdlcPosition, String> {
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
        other => Err(format!("invalid CPDLC position choice {other}")),
    }
}

fn parse_latitude(bits: &mut BitReader<'_>) -> Result<f64, String> {
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

fn parse_longitude(bits: &mut BitReader<'_>) -> Result<f64, String> {
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

fn parse_route_clearance(bits: &mut BitReader<'_>) -> Result<RouteClearance, String> {
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

fn parse_route_information(bits: &mut BitReader<'_>) -> Result<RouteInformation, String> {
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

fn parse_runway(bits: &mut BitReader<'_>) -> Result<String, String> {
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

fn parse_position_report(bits: &mut BitReader<'_>) -> Result<CpdlcPositionReport, String> {
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
        temperature: has_temperature.then(|| parse_temperature(bits)).transpose()?,
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

fn parse_remaining_fuel(bits: &mut BitReader<'_>) -> Result<CpdlcTime, String> {
    Ok(CpdlcTime {
        hour: bits.read_bits(5)? as u8,
        minute: bits.read_bits(6)? as u8,
    })
}

fn parse_temperature(bits: &mut BitReader<'_>) -> Result<CpdlcTemperature, String> {
    if bits.read_bool()? {
        Ok(CpdlcTemperature::Fahrenheit(bits.read_bits(8)? as i16 - 105))
    } else {
        Ok(CpdlcTemperature::Celsius(bits.read_bits(7)? as i16 - 80))
    }
}

fn parse_winds(bits: &mut BitReader<'_>) -> Result<CpdlcWinds, String> {
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

fn parse_vertical_change(bits: &mut BitReader<'_>) -> Result<CpdlcVerticalChange, String> {
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

fn parse_distance(bits: &mut BitReader<'_>) -> Result<CpdlcDistance, String> {
    if bits.read_bool()? {
        Ok(CpdlcDistance::Kilometers(bits.read_bits(10)? as u16 + 1))
    } else {
        Ok(CpdlcDistance::NauticalMiles(bits.read_bits(14)? as u16))
    }
}

fn parse_speed(bits: &mut BitReader<'_>) -> Result<CpdlcSpeed, String> {
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

fn parse_altimeter(bits: &mut BitReader<'_>) -> Result<CpdlcAltimeter, String> {
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

fn parse_error_information(bits: &mut BitReader<'_>) -> Result<CpdlcErrorInformation, String> {
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
        other => Err(format!("invalid CPDLC error information enum {other}")),
    }
}

fn parse_altitude(bits: &mut BitReader<'_>) -> Result<CpdlcAltitude, String> {
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

fn parse_time(bits: &mut BitReader<'_>) -> Result<CpdlcTime, String> {
    Ok(CpdlcTime {
        hour: bits.read_bits(5)? as u8,
        minute: bits.read_bits(6)? as u8,
    })
}

fn parse_distance_offset(bits: &mut BitReader<'_>) -> Result<DistanceOffset, String> {
    match bits.read_bits(1)? {
        0 => Ok(DistanceOffset::Nm(bits.read_bits(7)? as u16 + 1)),
        1 => Ok(DistanceOffset::Km(bits.read_bits(8)? as u16 + 1)),
        _ => unreachable!(),
    }
}

fn parse_direction(bits: &mut BitReader<'_>) -> Result<CpdlcDirection, String> {
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
        other => Err(format!("invalid CPDLC direction enum {other}")),
    }
}

fn parse_beacon_code(bits: &mut BitReader<'_>) -> Result<String, String> {
    let mut out = String::with_capacity(4);
    for _ in 0..4 {
        out.push(char::from(b'0' + bits.read_bits(3)? as u8));
    }
    Ok(out)
}

fn parse_degrees(bits: &mut BitReader<'_>) -> Result<CpdlcDegrees, String> {
    let value = bits.read_bits(9)? as u16 + 1;
    if bits.read_bool()? {
        Ok(CpdlcDegrees::True(value))
    } else {
        Ok(CpdlcDegrees::Magnetic(value))
    }
}

fn parse_procedure_name(bits: &mut BitReader<'_>) -> Result<CpdlcProcedureName, String> {
    let has_transition = bits.read_bool()?;
    let procedure_type = match bits.read_bits(2)? {
        0 => CpdlcProcedureType::Arrival,
        1 => CpdlcProcedureType::Approach,
        2 => CpdlcProcedureType::Departure,
        other => return Err(format!("invalid CPDLC procedure type {other}")),
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

fn parse_free_text(bits: &mut BitReader<'_>) -> Result<String, String> {
    // FANSFreeText is IA5String (SIZE(1..256)) with 7-bit constrained chars.
    let len = bits.read_bits(8)? as usize + 1;
    parse_ia5_chars(bits, len)
}

fn parse_fixed_ia5(bits: &mut BitReader<'_>, len: usize) -> Result<String, String> {
    parse_ia5_chars(bits, len)
}

fn parse_token_ia5(bits: &mut BitReader<'_>, len: usize) -> Result<String, String> {
    let text = parse_ia5_chars(bits, len)?;
    if text
        .bytes()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b' ' || b == b'-')
    {
        Ok(text.trim_end().to_string())
    } else {
        Err(format!("invalid CPDLC IA5 token {text:?}"))
    }
}

fn parse_ia5_chars(bits: &mut BitReader<'_>, len: usize) -> Result<String, String> {
    let mut out = String::with_capacity(len);
    for _ in 0..len {
        let ch = bits.read_bits(7)? as u8;
        out.push(char::from(ch));
    }
    Ok(out)
}

fn parse_icao_unit_name(bits: &mut BitReader<'_>) -> Result<IcaoUnitName, String> {
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

fn parse_frequency(bits: &mut BitReader<'_>) -> Result<CpdlcFrequency, String> {
    match bits.read_bits(2)? {
        0 => Ok(CpdlcFrequency::HfKhz(bits.read_bits(15)? as u32 + 2850)),
        1 => Ok(CpdlcFrequency::VhfKhz(bits.read_bits(15)? as u32 + 117_000)),
        2 => Ok(CpdlcFrequency::UhfKhz(bits.read_bits(18)? as u32 + 225_000)),
        3 => Ok(CpdlcFrequency::SatChannel(parse_numeric_string(bits, 12)?)),
        _ => unreachable!(),
    }
}

fn parse_numeric_string(bits: &mut BitReader<'_>, len: usize) -> Result<String, String> {
    let mut out = String::with_capacity(len);
    for _ in 0..len {
        let code = bits.read_bits(4)? as u8;
        let ch = match code {
            0 => ' ',
            1..=10 => char::from(b'0' + (code - 1)),
            _ => return Err(format!("invalid NumericString PER code {code}")),
        };
        out.push(ch);
    }
    Ok(out)
}

fn is_downlink_null(id: u16) -> bool {
    element_info(PduKind::Downlink, id).is_some_and(|info| info.name.ends_with("NULL"))
}

fn is_uplink_null(id: u16) -> bool {
    element_info(PduKind::Uplink, id).is_some_and(|info| info.name.ends_with("NULL"))
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
        serde_json::from_str(include_str!("../../../../data/cpdlc.json"))
            .expect("valid bundled CPDLC element catalog")
    })
}

fn parse_header(bits: &mut BitReader<'_>) -> Result<AtcMessageHeader, String> {
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

    fn read_bool(&mut self) -> Result<bool, String> {
        Ok(self.read_bits(1)? != 0)
    }

    fn read_bits(&mut self, n: usize) -> Result<u64, String> {
        if self.remaining() < n {
            return Err(format!(
                "need {n} bits, only {} bits remain at bit {}",
                self.remaining(),
                self.bit_pos
            ));
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
}
