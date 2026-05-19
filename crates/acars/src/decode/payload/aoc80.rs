//! ACARS label `80` — AOC (Airline Operations Control) text messages.
//!
//! Label `80` carries free-text operational messages from aircraft to ground.
//! The most common form is a position/event report in a fixed multiline text format.
//!
//! ## Message format
//!
//! ```text
//! <value> <msg_type> <flight_number>/<date> <dep>/<dest> <reg>\r\n
//! /ETA <hhmm>[/ERT <hhmm>]\r\n
//! <flight_level><sign><vrate>\r\n
//! <remarks>
//! ```
//!
//! Where:
//! - `value`         — leading numeric field (distance nm, time, or other)
//! - `msg_type`      — message type keyword: `INRANG`, `POSREP`, `OFFTIME`, `ONTIME`, `OUTTIME`, `INTIME`, etc.
//! - `flight_number` — airline flight number (e.g. `0804`)
//! - `date`          — date (day of month, e.g. `18`)
//! - `dep` / `dest`  — ICAO or IATA departure/destination airport codes
//! - `reg`           — aircraft registration (e.g. `.G-SUNF`)
//! - `eta`           — Estimated Time of Arrival (HHMM UTC)
//! - `ert`           — Estimated (runway/gate) Time (HHMM UTC)
//! - `flight_level`  — current flight level (e.g. `208` = FL208)
//! - `sign` / `vrate`— vertical rate direction and value (`+0` = level)
//! - `remarks`       — free-text loads or additional data (optional)
//!
//! ## Known message types
//!
//! | Type | Meaning |
//! |---|---|
//! | `INRANG` | In-range report (aircraft approaching destination) |
//! | `POSREP` | En-route position report |
//! | `OFFTIME` | Off-blocks / takeoff time report |
//! | `OUTTIME` | Pushback / out-from-gate time |
//! | `ONTIME` | Landing time |
//! | `INTIME` | On-blocks / gate-in time |
//! | `GATEOUT` | Gate departure |
//! | `DIVERTED` | Diversion notification |
//!
//! ## Observed fixture
//!
//! ```text
//! 3701 INRANG 0804/18 LEBL/EGCC .G-SUNF\r\n
//! /ETA 1322/ERT 1326\r\n
//! 208+0\r\n
//! 4R 4S 1C 0DPNA
//! ```
//! Decoded: Jet2 LS0804, G-SUNF, 37nm from EGCC (Barcelona→Manchester),
//! FL208 level, ETA 13:22, ERT 13:26.

use serde::{Deserialize, Serialize};

/// Decoded ACARS label `80` AOC message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AocMessage {
    /// Raw message type keyword (e.g. `"INRANG"`, `"POSREP"`)
    pub msg_type: String,
    /// Human-readable description of message type
    pub msg_type_description: String,
    /// Leading numeric value (distance nm, time, or other — interpretation
    /// depends on `msg_type`)
    pub leading_value: Option<String>,
    /// Flight number (e.g. `"0804"`)
    pub flight_number: Option<String>,
    /// Date, day of month (e.g. `"18"`)
    pub date: Option<String>,
    /// Departure airport code (e.g. `"LEBL"`)
    pub departure: Option<String>,
    /// Destination airport code (e.g. `"EGCC"`)
    pub destination: Option<String>,
    /// Aircraft registration (e.g. `".G-SUNF"`)
    pub registration: Option<String>,
    /// Estimated Time of Arrival, HHMM UTC (e.g. `"1322"`)
    pub eta: Option<String>,
    /// Estimated Runway/gate Time, HHMM UTC (e.g. `"1326"`)
    pub ert: Option<String>,
    /// Current flight level (e.g. `208`)
    pub flight_level: Option<u32>,
    /// Vertical rate direction: `"+"` climbing, `"-"` descending, `"0"` level
    pub vertical_trend: Option<String>,
    /// Vertical rate value (e.g. `"0"`)
    pub vertical_rate: Option<String>,
    /// Free-text remarks / loads field (e.g. `"4R 4S 1C 0DPNA"`)
    pub remarks: Option<String>,
}

fn msg_type_description(t: &str) -> &'static str {
    match t {
        "INRANG" => "In-range report",
        "POSREP" => "Position report",
        "OFFTIME" => "Off-blocks time",
        "OUTTIME" => "Pushback time",
        "ONTIME" => "Landing time",
        "INTIME" => "On-blocks time",
        "GATEOUT" => "Gate departure",
        "DIVERTED" => "Diversion notification",
        "FUELREP" => "Fuel report",
        "SEATREP" => "Seat report",
        _ => "AOC message",
    }
}

/// Parse an ACARS label `80` AOC text message.
///
/// Returns `None` if the text does not match the expected format.
pub fn parse_label80(txt: &str) -> Option<AocMessage> {
    let txt = txt.trim();
    if txt.is_empty() {
        return None;
    }

    let lines: Vec<&str> = txt.split("\r\n").collect();
    let line1 = lines.first()?;

    // Parse line 1: "<value> <msg_type> <flight>/<date> <dep>/<dest> <reg>"
    let parts: Vec<&str> = line1.splitn(5, ' ').collect();
    if parts.len() < 2 {
        return None;
    }

    // First token may be a numeric leading value; second is msg_type.
    // Handle both: "<value> <msg_type> ..." and "<msg_type> ..."
    let (leading_value, msg_type, rest_parts) = if parts[0].chars().next()?.is_ascii_digit() {
        (
            Some(parts[0].to_string()),
            parts[1].to_string(),
            &parts[2..],
        )
    } else {
        (None, parts[0].to_string(), &parts[1..])
    };

    let description = msg_type_description(&msg_type).to_string();

    // Parse flight/date: "0804/18"
    let mut flight_number = None;
    let mut date = None;
    let mut departure = None;
    let mut destination = None;
    let mut registration = None;

    if let Some(flt_date) = rest_parts.first() {
        if let Some((flt, dt)) = flt_date.split_once('/') {
            flight_number = Some(flt.to_string());
            date = Some(dt.to_string());
        }
    }

    // Parse dep/dest: "LEBL/EGCC"
    if let Some(dep_dest) = rest_parts.get(1) {
        if let Some((dep, dest)) = dep_dest.split_once('/') {
            departure = Some(dep.to_string());
            destination = Some(dest.to_string());
        }
    }

    // Parse registration
    if let Some(reg) = rest_parts.get(2) {
        if !reg.is_empty() {
            registration = Some(reg.to_string());
        }
    }

    // Parse line 2: "/ETA <hhmm>[/ERT <hhmm>]"
    let mut eta = None;
    let mut ert = None;
    if let Some(line2) = lines.get(1) {
        let line2 = line2.trim_start_matches('/');
        for segment in line2.split('/') {
            let segment = segment.trim();
            if let Some(val) = segment.strip_prefix("ETA").map(|s| s.trim()) {
                eta = Some(val.to_string());
            } else if let Some(val) = segment.strip_prefix("ERT").map(|s| s.trim()) {
                ert = Some(val.to_string());
            }
        }
    }

    // Parse line 3: "<flight_level><sign><vrate>" e.g. "208+0" or "350-5"
    let mut flight_level = None;
    let mut vertical_trend = None;
    let mut vertical_rate = None;
    if let Some(line3) = lines.get(2) {
        let line3 = line3.trim();
        // Find the sign character
        if let Some(sign_pos) = line3.find(['+', '-']) {
            if let Ok(fl) = line3[..sign_pos].parse::<u32>() {
                flight_level = Some(fl);
            }
            let sign = &line3[sign_pos..sign_pos + 1];
            vertical_trend = Some(sign.to_string());
            vertical_rate = Some(line3[sign_pos + 1..].to_string());
        } else if let Ok(fl) = line3.parse::<u32>() {
            flight_level = Some(fl);
        }
    }

    // Line 4+: remarks (join remaining lines)
    let remarks = if lines.len() > 3 {
        let r = lines[3..].join("\r\n");
        if r.trim().is_empty() {
            None
        } else {
            Some(r)
        }
    } else {
        None
    };

    Some(AocMessage {
        msg_type,
        msg_type_description: description,
        leading_value,
        flight_number,
        date,
        departure,
        destination,
        registration,
        eta,
        ert,
        flight_level,
        vertical_trend,
        vertical_rate,
        remarks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Observed fixture: G-SUNF Jet2 LS0804, in-range LEBL→EGCC
    /// Source: gqrx_20260518_114025_136500000_1800000_fc.raw, t=3.62s, ch=136.725 MHz
    const FIXTURE_INRANG: &str =
        "3701 INRANG 0804/18 LEBL/EGCC .G-SUNF\r\n/ETA 1322/ERT 1326\r\n208+0\r\n4R 4S 1C 0DPNA";

    #[test]
    fn test_parse_inrang() {
        let msg = parse_label80(FIXTURE_INRANG).expect("should parse INRANG");
        assert_eq!(msg.msg_type, "INRANG");
        assert_eq!(msg.msg_type_description, "In-range report");
        assert_eq!(msg.leading_value.as_deref(), Some("3701"));
        assert_eq!(msg.flight_number.as_deref(), Some("0804"));
        assert_eq!(msg.date.as_deref(), Some("18"));
        assert_eq!(msg.departure.as_deref(), Some("LEBL"));
        assert_eq!(msg.destination.as_deref(), Some("EGCC"));
        assert_eq!(msg.registration.as_deref(), Some(".G-SUNF"));
        assert_eq!(msg.eta.as_deref(), Some("1322"));
        assert_eq!(msg.ert.as_deref(), Some("1326"));
        assert_eq!(msg.flight_level, Some(208));
        assert_eq!(msg.vertical_trend.as_deref(), Some("+"));
        assert_eq!(msg.vertical_rate.as_deref(), Some("0"));
        assert_eq!(msg.remarks.as_deref(), Some("4R 4S 1C 0DPNA"));
    }

    #[test]
    fn test_parse_inrang_json() {
        let msg = parse_label80(FIXTURE_INRANG).unwrap();
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["msg_type"], "INRANG");
        assert_eq!(json["departure"], "LEBL");
        assert_eq!(json["destination"], "EGCC");
        assert_eq!(json["flight_level"], 208);
        assert_eq!(json["eta"], "1322");
    }

    #[test]
    fn test_parse_no_leading_value() {
        // Message without leading numeric value
        let txt = "POSREP FLT123/20 EDDM/EGLL .D-AIBL\r\n/ETA 1445\r\n350+0";
        let msg = parse_label80(txt).expect("should parse POSREP");
        assert_eq!(msg.msg_type, "POSREP");
        assert_eq!(msg.msg_type_description, "Position report");
        assert!(msg.leading_value.is_none());
        assert_eq!(msg.flight_number.as_deref(), Some("FLT123"));
        assert_eq!(msg.eta.as_deref(), Some("1445"));
        assert!(msg.ert.is_none());
        assert_eq!(msg.flight_level, Some(350));
        assert_eq!(msg.vertical_trend.as_deref(), Some("+"));
    }

    #[test]
    fn test_parse_empty_returns_none() {
        assert!(parse_label80("").is_none());
    }

    #[test]
    fn test_parse_minimal_no_eta_no_level() {
        let txt = "ONTIME 0505/12 EGCC/LEBL .G-SUNF";
        let msg = parse_label80(txt).expect("should parse ONTIME");
        assert_eq!(msg.msg_type, "ONTIME");
        assert_eq!(msg.msg_type_description, "Landing time");
        assert!(msg.leading_value.is_none());
        assert_eq!(msg.departure.as_deref(), Some("EGCC"));
        assert_eq!(msg.destination.as_deref(), Some("LEBL"));
        assert!(msg.eta.is_none());
        assert!(msg.flight_level.is_none());
    }
}
