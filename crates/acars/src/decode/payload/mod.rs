//! Application-layer payload modules.
//!
//! This module groups app protocols that sit above bearer/envelope decoders:
//! ACARS label payloads, ARINC 622/FANS-1/A payloads, and ATN B1 payloads
//! reached via VDL2 X.25/COTP/ULCS. The top-level `AcarsAppPayload` enum
//! covers only payloads decoded from ACARS messages.
//!
//! Errors from any payload decoder are reported as `PayloadError`, which
//! wraps into `DecodeError::InvalidPayload`.

pub mod aoc;
pub mod arinc620;
pub mod arinc622;
pub mod arinc623;
pub mod atn_b1;
pub mod boeing;
pub mod miam;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use self::aoc::label16::Label16Message;
use self::aoc::label32::Label32Message;
use self::aoc::label37::Label37Message;
use self::aoc::label5z::Label5zMessage;
use self::aoc::label80::AocMessage;
use self::aoc::oooi::{OooiOffDestination, OooiOffReport};
use self::aoc::position::AocPositionMessage;
use self::aoc::weather::WeatherBundle;
use self::arinc620::media_advisory::MediaAdvisory;
use self::arinc620::squitter::SquitterMessage;
use self::arinc622::afn::AfnMessage;
use self::arinc622::oceanic::OceanicClearance;
use self::arinc622::Message as Arinc622Message;
pub use self::arinc622::{Imi, Payload as Arinc622Payload};
use self::arinc623::atis::{AtisDelivery, AtisRequest};
use self::boeing::ohma::OhmaMessage;
use self::miam::MiamMessage;

/// Errors produced by payload-layer decoders (everything under `payload/`).
///
/// Wraps into `DecodeError::InvalidPayload`.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PayloadError {
    #[error("invalid ADS-C payload")]
    Adsc,
    #[error("invalid ARINC 622 envelope: {0}")]
    Arinc622(String),
    #[error("invalid Media Advisory: {0}")]
    MediaAdvisory(String),
    #[error("invalid MIAM frame: {0}")]
    Miam(String),
    #[error("invalid OHMA message: {0}")]
    Ohma(String),
    #[error("invalid SQ squitter: {0}")]
    Squitter(String),
}

/// All possible decoded application payloads for an ACARS message.
///
/// ARINC 622 messages keep their standards-defined header and IMI-dispatched
/// payload together. Other variants are inferred from ACARS labels/sublabels.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "data")]
pub enum AcarsAppPayload {
    /// Standards-defined ARINC 622 envelope and decoded payload.
    Arinc622(Arinc622Message),

    /// MIAM (Management of Integrated Avionics Maintenance) — label `MA` or `H1/T1`.
    #[serde(rename = "MIAM")]
    Miam(MiamMessage),

    /// OHMA (Boeing 737 MAX health monitoring) — label `H1` sublabel `T1`.
    #[serde(rename = "OHMA")]
    Ohma(OhmaMessage),

    /// ACARS `SA` Media Advisory — link established/lost notification.
    #[serde(rename = "SA")]
    MediaAdvisory(MediaAdvisory),

    /// ACARS `SQ` squitter / ground-station broadcast.
    #[serde(rename = "SQ")]
    Squitter(SquitterMessage),

    /// ACARS label `80` AOC position/event report.
    #[serde(rename = "AOC80")]
    AocReport(AocMessage),

    /// AOC weather/METAR bundle.
    Weather(WeatherBundle),

    /// ACARS label `5Z` slash-field AOC message.
    #[serde(rename = "5Z")]
    Label5z(Label5zMessage),

    /// AOC position/telemetry report.
    AocPosition(AocPositionMessage),

    /// ACARS label `32` CSV telemetry.
    #[serde(rename = "32")]
    Label32(Label32Message),

    /// ACARS label `16` heterogeneous telemetry classifier.
    #[serde(rename = "16")]
    Label16(Label16Message),

    /// ACARS label `37` obfuscated/encoded ops classifier.
    #[serde(rename = "37")]
    Label37(Label37Message),

    /// ACARS label `Q0` — ACARS link test / keepalive. Payload is always empty.
    #[serde(rename = "Q0")]
    LinkTest,

    /// ACARS label `QF` — OFF Destination Report (sent shortly after takeoff).
    #[serde(rename = "QF")]
    OooiOffDestination(OooiOffDestination),

    /// ACARS label `QQ` — OFF Report, extended form with ICAO codes.
    #[serde(rename = "QQ")]
    OooiOffReport(OooiOffReport),

    /// ACARS label `B9` — ATIS request (TI2 protocol, aircraft → ground).
    #[serde(rename = "B9")]
    AtisRequest(AtisRequest),

    /// ACARS label `A9` — ATIS delivery (TI2 protocol, ground → aircraft).
    #[serde(rename = "A9")]
    AtisDelivery(AtisDelivery),

    /// ACARS labels `A0`/`B0` — AFN CONTACT / logon.
    #[serde(rename = "AFN")]
    Afn(AfnMessage),

    /// ACARS label `B1` — oceanic clearance / OC1.
    #[serde(rename = "OC1")]
    OceanicClearance(OceanicClearance),

    /// Non-empty `text` with no structured decoder.
    Text(String),

    /// Empty `text` or label `_d`.
    None,
}
