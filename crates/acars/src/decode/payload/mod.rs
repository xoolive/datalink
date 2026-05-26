//! ACARS application-layer payload modules.
//!
//! All app-layer decoders live under this module. The top-level enum
//! `AcarsAppPayload` is defined here and references the sub-modules.
//!
//! Errors from any payload decoder are reported as `PayloadError`, which
//! wraps into `DecodeError::InvalidPayload`.

pub mod aoc80;
pub mod arinc622;
pub mod media_advisory;
pub mod miam;
pub mod ohma;
pub mod sq;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use self::aoc80::AocMessage;
use self::arinc622::Message as Arinc622Message;
pub use self::arinc622::{Imi, Payload as Arinc622Payload};
use self::media_advisory::MediaAdvisory;
use self::miam::MiamMessage;
use self::ohma::OhmaMessage;
use self::sq::SquitterMessage;

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

    /// Non-empty `text` with no structured decoder.
    Text(String),

    /// Empty `text` or label `_d`.
    None,
}
