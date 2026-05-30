//! OHMA (On-board Health Monitoring Architecture) decoder.
//!
//! Boeing 737 MAX aircraft transmit OHMA messages over ACARS label `H1` sublabel `T1`.
//! Each OHMA message carries diagnostic and monitoring data compressed as zlib(JSON).
//!
//! ## Wire format
//!
//! ```text
//! [OHMA|RYKO][base64(zlib(outer_json))]
//! ```
//!
//! The prefix `OHMA` or `RYKO` identifies the message family. The rest is
//! standard Base64 (RFC 4648) encoding of a zlib-compressed JSON object.
//!
//! Long-form prefixes also exist for uplinks:
//! - `/<7-char GSID>.OHMA...` (uplink long form)
//! - `/<2-char>.OHMA...` (uplink short form)
//!
//! ## Outer JSON structure
//!
//! ```json
//! {
//!   "version": "2.0",
//!   "message": "<inner JSON string, double-encoded>",
//!   "convo_id": "<optional: multi-part message key>",
//!   "msg_seq": <optional: 1-based part number>,
//!   "msg_total": <optional: total number of parts>,
//!   "sym_key": "<optional: base64 encryption key>",
//!   "iv": "<optional: base64 IV>",
//!   "signature": "<optional: base64 signature>"
//! }
//! ```
//!
//! When `msg_seq` is present and > 0, the message is a multi-part OHMA
//! transmission. Parts are reassembled by `(reg, convo_id)` key before the
//! inner JSON is decoded. This is distinct from ACARS-level multi-block
//! reassembly (ETB/ETX), which must be performed first.
//!
//! ## Inner JSON structure (`message` field, double-encoded)
//!
//! ```json
//! {
//!   "clientId": "OHMA",
//!   "messageDate": "2026-05-18T11:20:51.240Z",
//!   "data": {
//!     "airplanes": [{
//!       "tailNumber": "OK-SWN",
//!       "model": "",
//!       "flights": [{
//!         "departureAirportCode": "LKPR",
//!         "arrivalAirportCode": "GCTS",
//!         "flightNumber": "TVS1BC",
//!         "flightLegStartTime": "2026-05-18T09:59:44.210Z",
//!         "events": [...]
//!       }]
//!     }]
//!   }
//! }
//! ```
//!
//! ## Observed fixture
//!
//! Aircraft OK-SWN (Boeing 737 MAX), Smartwings flight TVS1BC, LKPR→GCTS.
//! 4-block ACARS reassembly (blocks A–D, msg_num=102), then single-part OHMA decode.
//! Events: `OHMA_META` (ruleset metadata) + `TM1_CLIPMEDIAN_CRUISE_RPT_A`
//! (cruise thermal/pressure pack report).
//! Source: `gqrx_20260518_114025_136500000_1800000_fc.raw`, t=31.59–37.62s.

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::decode::payload::PayloadError;
use crate::decode::{DecodeError, DecodeResult};

/// Decoded OHMA message parameter instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OhmaInstance {
    /// Parameter name (e.g. `"TM1TEMP1_MED"`)
    pub name: String,
    /// Parameter values (usually a single numeric or string value)
    pub values: Vec<serde_json::Value>,
}

/// One OHMA event record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OhmaEvent {
    /// Event class identifier (e.g. `"parametric"`)
    pub event_class_id: String,
    /// ISO 8601 UTC timestamp of the event
    pub event_time: String,
    /// Event type (e.g. `"TM1_CLIPMEDIAN_CRUISE_RPT_A"`, `"OHMA_META"`)
    pub event_type: String,
    /// Named parameter instances for this event
    pub instances: Vec<OhmaInstance>,
}

/// One OHMA flight record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OhmaFlight {
    /// ICAO/IATA departure airport code
    pub departure_airport: String,
    /// ICAO/IATA arrival airport code
    pub arrival_airport: String,
    /// Flight number (e.g. `"TVS1BC"`)
    pub flight_number: String,
    /// UTC ISO 8601 leg start time
    pub leg_start_time: String,
    /// Events recorded for this flight leg
    pub events: Vec<OhmaEvent>,
}

/// One airplane entry within an OHMA message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OhmaAirplane {
    /// Aircraft tail number / registration (e.g. `"OK-SWN"`)
    pub tail_number: String,
    /// Aircraft model string (may be empty)
    pub model: String,
    /// Flight records
    pub flights: Vec<OhmaFlight>,
}

/// Decoded OHMA message payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OhmaMessage {
    /// OHMA protocol version string (e.g. `"2.0"`)
    pub version: String,
    /// Client identifier from inner JSON (typically `"OHMA"`)
    pub client_id: String,
    /// ISO 8601 UTC message generation timestamp
    pub message_date: String,
    /// Airplane records (usually one entry per message)
    pub airplanes: Vec<OhmaAirplane>,
    /// Multi-part conversation ID (present when `msg_seq > 0`)
    pub convo_id: Option<String>,
    /// 1-based part number within a multi-part transmission
    pub msg_seq: Option<i32>,
    /// Total number of parts in a multi-part transmission
    pub msg_total: Option<i32>,
}

/// Recognised OHMA/RYKO prefix lengths (for stripping envelope headers).
const OHMA_SHORT_PREFIX: &str = "OHMA";
const RYKO_SHORT_PREFIX: &str = "RYKO";

/// Strip any OHMA/RYKO envelope prefix and return the base64 payload.
fn strip_prefix(txt: &str) -> Option<&str> {
    let t = txt.trim();
    // Long-form uplink: "/XXXXXXX.OHMA..." or "XX.OHMA..."
    let t = if t.starts_with('/') && t.len() >= 13 && t.as_bytes().get(8) == Some(&b'.') {
        &t[9..]
    } else if t.starts_with('/') && t.len() >= 8 && t.as_bytes().get(3) == Some(&b'.') {
        &t[4..]
    } else {
        t
    };
    // Now expect OHMA or RYKO
    if t.starts_with(OHMA_SHORT_PREFIX) || t.starts_with(RYKO_SHORT_PREFIX) {
        Some(&t[4..])
    } else {
        None
    }
}

/// Check whether `txt` looks like an OHMA/RYKO message.
pub fn is_ohma(txt: &str) -> bool {
    strip_prefix(txt).is_some()
}

/// Parse an OHMA/RYKO message from assembled ACARS `H1/T1` text.
///
/// ## Steps
///
/// 1. Strip the `OHMA`/`RYKO` prefix (and any envelope header).
/// 2. Base64-decode the remaining bytes.
/// 3. zlib-decompress (standard zlib/DEFLATE, RFC 1950).
/// 4. JSON-parse the outer object.
/// 5. JSON-parse the double-encoded `message` string.
/// 6. Map the inner JSON into `OhmaMessage`.
///
/// Multi-part OHMA reassembly (when `msg_seq > 0`) is not handled here —
/// the `convo_id`, `msg_seq`, and `msg_total` fields are preserved so the
/// caller can implement reassembly if needed.
pub fn parse_ohma(txt: &str) -> DecodeResult<OhmaMessage> {
    let b64 = strip_prefix(txt).ok_or_else(|| {
        DecodeError::InvalidPayload(PayloadError::Ohma("missing OHMA/RYKO prefix".into()))
    })?;

    // Strip trailing CR/LF (common in uplink messages, invalid base64)
    let b64 = b64.trim_end_matches(['\r', '\n']);

    // Base64-decode (add padding if needed)
    let pad = (4 - b64.len() % 4) % 4;
    let b64_padded = format!("{b64}{}", "=".repeat(pad));
    let compressed = base64::engine::general_purpose::STANDARD
        .decode(b64_padded.as_bytes())
        .map_err(|e| {
            DecodeError::InvalidPayload(PayloadError::Ohma(format!("base64 decode: {e}")))
        })?;

    // zlib-decompress (standard zlib RFC 1950 with CMF+FLG header)
    use std::io::Read;
    let mut decoder = flate2::read::ZlibDecoder::new(&compressed[..]);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed).map_err(|e| {
        DecodeError::InvalidPayload(PayloadError::Ohma(format!("zlib decompress: {e}")))
    })?;

    // Parse outer JSON
    #[derive(Deserialize)]
    struct OuterJson {
        version: String,
        message: String,
        convo_id: Option<String>,
        msg_seq: Option<i32>,
        msg_total: Option<i32>,
    }
    let outer: OuterJson = serde_json::from_slice(&decompressed)
        .map_err(|e| DecodeError::InvalidPayload(PayloadError::Ohma(format!("outer JSON: {e}"))))?;

    // Parse inner JSON (double-encoded string in `message`)
    #[derive(Deserialize)]
    struct InnerJson {
        #[serde(rename = "clientId")]
        client_id: String,
        #[serde(rename = "messageDate")]
        message_date: String,
        data: InnerData,
    }
    #[derive(Deserialize)]
    struct InnerData {
        airplanes: Vec<InnerAirplane>,
    }
    #[derive(Deserialize)]
    struct InnerAirplane {
        #[serde(rename = "tailNumber")]
        tail_number: String,
        #[serde(default)]
        model: String,
        flights: Vec<InnerFlight>,
    }
    #[derive(Deserialize)]
    struct InnerFlight {
        #[serde(rename = "departureAirportCode")]
        departure_airport_code: String,
        #[serde(rename = "arrivalAirportCode")]
        arrival_airport_code: String,
        #[serde(rename = "flightNumber")]
        flight_number: String,
        #[serde(rename = "flightLegStartTime")]
        flight_leg_start_time: String,
        #[serde(default)]
        events: Vec<InnerEvent>,
    }
    #[derive(Deserialize)]
    struct InnerEvent {
        #[serde(rename = "eventClassId")]
        event_class_id: String,
        #[serde(rename = "eventTime")]
        event_time: String,
        #[serde(rename = "eventType")]
        event_type: String,
        #[serde(default)]
        instances: Vec<InnerInstance>,
    }
    #[derive(Deserialize)]
    struct InnerInstance {
        name: String,
        values: Vec<serde_json::Value>,
    }

    let inner: InnerJson = serde_json::from_str(&outer.message)
        .map_err(|e| DecodeError::InvalidPayload(PayloadError::Ohma(format!("inner JSON: {e}"))))?;

    let airplanes = inner
        .data
        .airplanes
        .into_iter()
        .map(|a| OhmaAirplane {
            tail_number: a.tail_number,
            model: a.model,
            flights: a
                .flights
                .into_iter()
                .map(|f| OhmaFlight {
                    departure_airport: f.departure_airport_code,
                    arrival_airport: f.arrival_airport_code,
                    flight_number: f.flight_number,
                    leg_start_time: f.flight_leg_start_time,
                    events: f
                        .events
                        .into_iter()
                        .map(|e| OhmaEvent {
                            event_class_id: e.event_class_id,
                            event_time: e.event_time,
                            event_type: e.event_type,
                            instances: e
                                .instances
                                .into_iter()
                                .map(|i| OhmaInstance {
                                    name: i.name,
                                    values: i.values,
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect();

    Ok(OhmaMessage {
        version: outer.version,
        client_id: inner.client_id,
        message_date: inner.message_date,
        airplanes,
        convo_id: outer.convo_id,
        msg_seq: outer.msg_seq,
        msg_total: outer.msg_total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reassembled 4-block OHMA fixture: OK-SWN, TVS1BC, LKPR→GCTS.
    /// Source: `gqrx_20260518_114025_136500000_1800000_fc.raw`, H1/T1, msg_num=102.
    const FIXTURE: &str = concat!(
        "OHMAeJy1klFvmzAUhf/K5OeAbBMa4I0Q0qJAioC208aEvMTLkAyJjBNpqvLfewtJlxGyPQ2eL",
        "j7fufce84oOXDbltkYOojpGI1TxpmEbDvVrjlai5LUK1jlycvT4ELk5GuVnyYwp3h5QTO80bGrEyghxK",
        "HZMotMx/tKK10wxUIEZK+VOsJo3UH6FWrFSLPfVdy47+4WWviy7Bts1F+3Htvwhys1PdcbWfMek2kvugt",
        "9WKg/ErTZcxEmrZ1KWByb65/dell74XbTOnlMy9S7OQr5JFXTJyupqRWw7pu2MxzolpxX5AUI6j9cWnmB",
        "Nc4oNpmUVV7Jc/RYP+V5H10l/7fhH/EXkZ90dlHWjWL36CLNmJ8dKxTD4eTlQQhL7TpajqXdvUA2Dk4Y",
        "xITn6dhz9CXtMiuful7iix7o5QCR7wRuubkETY/Ipcj87juM+RJoxiaahBc3hmbjwsZ2h8/xvyWURKbwwi",
        "CN/FrjLwkuegtQvkjgr/pElgJkfxQRin/UWMwysG5T20jgBdAgglm6b9jAwC+bzAYZg3b7rI3FE4sRP06f",
        "EHxwMbskgt5nB2Uzd6q8Su95iHj6+3BgNtretASRYvjf6C2S+X3f3HtHxDenfYyE="
    );

    #[test]
    fn test_is_ohma() {
        assert!(is_ohma("OHMAeJy1..."));
        assert!(is_ohma("RYKOeJy1..."));
        assert!(is_ohma("/RTNBOCR.OHMAeJy1..."));
        assert!(is_ohma("/O2.OHMAeJy1..."));
        assert!(!is_ohma("MIAM..."));
        assert!(!is_ohma(""));
    }

    #[test]
    fn test_parse_fixture() {
        let msg = parse_ohma(FIXTURE).expect("should parse");
        assert_eq!(msg.version, "2.0");
        assert_eq!(msg.client_id, "OHMA");
        assert!(msg.message_date.starts_with("2026-05-18"));
        assert!(msg.convo_id.is_none());
        assert!(msg.msg_seq.is_none());

        assert_eq!(msg.airplanes.len(), 1);
        let plane = &msg.airplanes[0];
        assert_eq!(plane.tail_number, "OK-SWN");

        assert_eq!(plane.flights.len(), 1);
        let flight = &plane.flights[0];
        assert_eq!(flight.flight_number, "TVS1BC");
        assert_eq!(flight.departure_airport, "LKPR");
        assert_eq!(flight.arrival_airport, "GCTS");

        assert_eq!(flight.events.len(), 2);
        assert_eq!(flight.events[0].event_type, "OHMA_META");
        assert_eq!(flight.events[1].event_type, "TM1_CLIPMEDIAN_CRUISE_RPT_A");

        let meta = &flight.events[0];
        let part_num = meta
            .instances
            .iter()
            .find(|i| i.name == "mtPartNumber")
            .unwrap();
        assert_eq!(part_num.values[0], "BCG32-0HMA-0011");

        let cruise = &flight.events[1];
        let temp1 = cruise
            .instances
            .iter()
            .find(|i| i.name == "TM1TEMP1_MED")
            .unwrap();
        assert!((temp1.values[0].as_f64().unwrap() - 330.322).abs() < 0.001);
    }

    #[test]
    fn test_json_serializes() {
        let msg = parse_ohma(FIXTURE).unwrap();
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["version"], "2.0");
        assert_eq!(json["airplanes"][0]["tail_number"], "OK-SWN");
        assert_eq!(
            json["airplanes"][0]["flights"][0]["departure_airport"],
            "LKPR"
        );
        assert_eq!(
            json["airplanes"][0]["flights"][0]["events"][0]["event_type"],
            "OHMA_META"
        );
    }

    #[test]
    fn test_invalid_prefix_returns_err() {
        assert!(parse_ohma("MIAM...").is_err());
        assert!(parse_ohma("").is_err());
    }
}
