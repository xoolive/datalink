//! Redis/JSONL event schema shared by the datalink binary and downstream consumers.

use serde::{Deserialize, Serialize};

/// Broad category of input source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceClass {
    /// Raw I/Q samples from a file, stdin, or SDR.
    Iq,
    /// Upstream decoded events such as Airframes.io websocket messages.
    Events,
    /// Standalone frames or payloads that do not require demodulation.
    Frames,
}

// TODO remove alias
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Bearer {
    /// Classic VHF ACARS / Plain Old ACARS bearer.
    #[serde(alias = "acars", alias = "vhf")]
    Vhf,
    /// VDL Mode 2 bearer, normally AVLC frames on 136 MHz channels.
    #[serde(alias = "vdl", alias = "vdl2")]
    Vdl2,
    /// HF Data Link bearer.
    #[serde(alias = "hf", alias = "hfdl")]
    Hfdl,
    /// Standalone decoded application payload with no live RF bearer.
    Decoded,
    /// Unknown or upstream-provided bearer value that does not match a known variant.
    #[serde(other)]
    #[default]
    Unknown,
}

impl Bearer {
    pub fn as_str(&self) -> &'static str {
        match self {
            Bearer::Vhf => "vhf",
            Bearer::Vdl2 => "vdl2",
            Bearer::Hfdl => "hfdl",
            Bearer::Decoded => "decoded",
            Bearer::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for Bearer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecodedEvent {
    /// Event class emitted by the receiver; currently `"message"` for decoded rows.
    pub event: String,
    /// Receive or upstream event timestamp as Unix epoch seconds when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
    /// Physical or feed bearer that produced the message.
    pub bearer: Bearer,
    /// Metadata describing the file, SDR, websocket, or frame source.
    pub source: SourceMetadata,
    /// Receiver metadata for channelized I/Q sources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver: Option<ReceiverMetadata>,
    /// Best-effort aircraft identity extracted from frame addresses or payload metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aircraft: Option<Aircraft>,
    /// Best-effort normalized position/altitude/speed summary extracted from the payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kinematics: Option<acars::decode::compact::Kinematics>,
    /// Raw decoded frame bytes as uppercase hexadecimal when the source exposes them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_frame_hex: Option<String>,
    /// Protocol-specific decoded message body.
    pub message: ProtocolMessage,
}

// TODO re-evaluate if we really want Box
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum ProtocolMessage {
    /// Message received from the Airframes.io websocket and normalized by this CLI.
    Airframes(Box<AirframesMessage>),
    /// VDL2 AVLC frame, including link-layer addresses and dispatched payload.
    Avlc(Box<acars::decode::avlc::AvlcFrame>),
    /// Classic ACARS frame decoded directly from VHF ACARS or standalone input.
    Acars(Box<acars::decode::acars::AcarsMessage>),
    /// HFDL frame decoded into SPDU or MPDU structures.
    Hfdl(Box<acars::decode::hfdl::HfdlMessage>),
    /// Standalone ACARS application payload decoded without a surrounding bearer frame.
    App(Box<acars::decode::payload::AcarsAppPayload>),
}

impl ProtocolMessage {
    pub fn kinematics(&self) -> Option<acars::decode::compact::Kinematics> {
        use acars::decode::compact::ExtractKinematics;
        match self {
            Self::Airframes(msg) => {
                if let Some(app) = &msg.app {
                    app.kinematics()
                } else {
                    msg.payload.kinematics()
                }
            }
            Self::Avlc(frame) => frame.kinematics(),
            Self::Acars(msg) => msg.kinematics(),
            Self::Hfdl(msg) => msg.kinematics(),
            Self::App(app) => app.kinematics(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Aircraft {
    /// ICAO 24-bit aircraft address as six lowercase hexadecimal characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icao24: Option<String>,
    /// Upstream aircraft identifier when available, currently from Airframes.io rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aircraft_id: Option<u64>,
    /// Aircraft registration or tail number when present in the payload or upstream row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceMetadata {
    /// Stable source identifier from configuration or the frontend default.
    pub id: String,
    /// Human-readable source label.
    pub name: String,
    /// Source class: I/Q samples, already-decoded events, or standalone frames.
    pub class: SourceClass,
    /// Source format hint such as `cf32`, `cu8`, `wav`, or `airframes.io`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiverMetadata {
    /// Bearer decoded by this receiver pipeline.
    pub bearer: Bearer,
    /// Channel center frequency in Hz for channelized I/Q receivers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_hz: Option<u32>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
/// Upstream Airframes.io message payload.
///
/// Fields are optional because the global stream contains a mix of fully decoded
/// ACARS rows, metadata-only VDL rows, station events, and partial records.
pub struct AirframesPayload {
    pub label: Option<String>,
    pub text: Option<String>,
    pub from_hex: Option<String>,
    pub to_hex: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<f64>,
    pub track: Option<f64>,
    #[serde(default)]
    pub source_type: Bearer,
    #[serde(deserialize_with = "deserialize_timestamp", default)]
    pub timestamp: Option<f64>,
    #[serde(deserialize_with = "deserialize_timestamp", default)]
    pub created_at: Option<f64>,
    pub frequency: Option<f64>,
    pub id: Option<String>,
    pub airframe_id: Option<u64>,
    pub flight_id: Option<u64>,
    pub tail: Option<String>,
    pub link_direction: Option<String>,
    pub airframe: Option<AirframesAirframe>,
    pub flight: Option<AirframesFlight>,
}

impl AirframesPayload {
    pub fn kinematics(&self) -> Option<acars::decode::compact::Kinematics> {
        let payload_kinematics = kinematics_from_airframes_position(
            self.latitude,
            self.longitude,
            self.altitude,
            self.track,
            "airframes_payload",
        );
        let flight_kinematics = self.flight.as_ref().and_then(|flight| {
            kinematics_from_airframes_position(
                flight.latitude,
                flight.longitude,
                flight.altitude,
                flight.track,
                "airframes_flight",
            )
        });
        match (payload_kinematics, flight_kinematics) {
            (Some(payload), Some(flight)) => Some(payload.merge(flight)),
            (Some(payload), None) => Some(payload),
            (None, Some(flight)) => Some(flight),
            (None, None) => None,
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct AirframesAirframe {
    pub icao: Option<String>,
    pub tail: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct AirframesFlight {
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<f64>,
    pub track: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Airframes row plus best-effort normalized addresses and decoded app payload.
pub struct AirframesMessage {
    pub payload: AirframesPayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src: Option<AirframesAddr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dst: Option<AirframesAddr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<acars::decode::payload::AcarsAppPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirframesAddr {
    pub icao24: String,
    pub addr_type: AirframesAddrType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AirframesAddrType {
    /// Address is believed to identify an aircraft.
    Aircraft,
    /// Address is believed to identify a ground station.
    GroundStation,
    /// Address role could not be inferred from the row.
    Unknown,
}

fn kinematics_from_airframes_position(
    latitude: Option<f64>,
    longitude: Option<f64>,
    altitude: Option<f64>,
    track: Option<f64>,
    derived_from: &str,
) -> Option<acars::decode::compact::Kinematics> {
    let (Some(latitude), Some(longitude)) = (latitude, longitude) else {
        return None;
    };
    if latitude == 0.0 && longitude == 0.0 {
        return None;
    }
    Some(acars::decode::compact::Kinematics {
        position: Some(acars::decode::compact::Position {
            latitude,
            longitude,
        }),
        altitude_ft: normalize_airframes_altitude(altitude),
        track,
        ground_speed_knots: None,
        derived_from: Some(derived_from.to_string()),
    })
}

fn normalize_airframes_altitude(altitude: Option<f64>) -> Option<i32> {
    altitude.and_then(|value| {
        let rounded = value.round();
        ((i32::MIN as f64)..=(i32::MAX as f64))
            .contains(&rounded)
            .then_some(rounded as i32)
    })
}

fn deserialize_timestamp<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct TsVisitor;

    impl<'de> serde::de::Visitor<'de> for TsVisitor {
        type Value = Option<f64>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a float or a timestamp string")
        }

        fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
            Ok(Some(value))
        }

        fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
            Ok(Some(value as f64))
        }

        fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
            if let Ok(n) = value.parse::<f64>() {
                Ok(Some(n))
            } else if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
                Ok(Some(dt.timestamp_micros() as f64 / 1_000_000.0))
            } else {
                Ok(None)
            }
        }

        fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D2: serde::Deserializer<'de>>(
            self,
            deserializer: D2,
        ) -> Result<Self::Value, D2::Error> {
            deserializer.deserialize_any(self)
        }
    }

    deserializer.deserialize_any(TsVisitor)
}
