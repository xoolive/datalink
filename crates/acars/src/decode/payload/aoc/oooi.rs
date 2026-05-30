//! ACARS OOOI (Out/Off/On/In) operational event decoders.
//!
//! Covers the common "OFF" report labels sent after takeoff:
//!
//! - **QF** (`OooiOffDestination`): departure airport + time + destination.
//!   Format: `<3-char-dep><HHMM><3-char-dest>[optional extras]`
//!
//! - **QQ** (`OooiOffReport`): extended OFF report with 4-char ICAO codes.
//!   Format: `<4-char-dep><4-char-arr><HHMM>[optional extras]`
//!   Optional extras include position fixes, fuel, times, and other AOC fields.

use serde::{Deserialize, Serialize};

/// QF: OFF Destination Report — sent shortly after takeoff.
///
/// Compact form used by carriers that transmit only the
/// origin/destination pair and departure time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OooiOffDestination {
    /// 3-character departure airport code.
    pub departure: String,
    /// Departure time UTC as `"HHMM"`.
    pub time_utc: String,
    /// 3-character arrival/destination airport code.
    pub destination: String,
    /// Optional trailing fields (airline-specific: `/FUL`, `/FB`, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extras: Option<String>,
}

/// QQ: OFF Report — extended form with 4-character ICAO codes.
///
/// Sent shortly after takeoff; may optionally include position,
/// fuel-on-board, and other operational data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OooiOffReport {
    /// 4-character ICAO departure airport code.
    pub departure: String,
    /// 4-character ICAO arrival airport code.
    pub arrival: String,
    /// Takeoff time UTC as `"HHMM"`.
    pub time_utc: String,
    /// Optional trailing fields (position, fuel, times, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extras: Option<String>,
}

/// Parse a QF (OFF Destination Report) message.
///
/// Expects at least 10 characters: `<DEP3><HHMM><DEST3>`.
/// Any remaining text is captured in `extras`.
pub fn parse_qf(txt: &str) -> Option<OooiOffDestination> {
    let txt = txt.trim();
    if txt.len() < 10 {
        return None;
    }
    let dep = &txt[..3];
    let time = &txt[3..7];
    let dest = &txt[7..10];
    // Basic sanity: time digits
    if !time.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let extras = if txt.len() > 10 {
        let rest = txt[10..].trim();
        if rest.is_empty() {
            None
        } else {
            Some(rest.to_string())
        }
    } else {
        None
    };
    Some(OooiOffDestination {
        departure: dep.to_ascii_uppercase(),
        time_utc: time.to_string(),
        destination: dest.to_ascii_uppercase(),
        extras,
    })
}

/// Parse a QQ (OFF Report) message.
///
/// Expects at least 12 characters: `<DEP4><ARR4><HHMM>`.
/// Any remaining text is captured in `extras`.
pub fn parse_qq(txt: &str) -> Option<OooiOffReport> {
    let txt = txt.trim();
    if txt.len() < 12 {
        return None;
    }
    let dep = &txt[..4];
    let arr = &txt[4..8];
    let time = &txt[8..12];
    if !time.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let extras = if txt.len() > 12 {
        let rest = txt[12..].trim();
        if rest.is_empty() {
            None
        } else {
            Some(rest.to_string())
        }
    } else {
        None
    };
    Some(OooiOffReport {
        departure: dep.to_ascii_uppercase(),
        arrival: arr.to_ascii_uppercase(),
        time_utc: time.to_string(),
        extras,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_qf_simple() {
        let m = parse_qf("BNE0406NTL").unwrap();
        assert_eq!(m.departure, "BNE");
        assert_eq!(m.time_utc, "0406");
        assert_eq!(m.destination, "NTL");
        assert!(m.extras.is_none());
    }

    #[test]
    fn parse_qf_with_extras() {
        let m = parse_qf("PHL2300BUFOS BUF/FUL0680").unwrap();
        assert_eq!(m.departure, "PHL");
        assert_eq!(m.time_utc, "2300");
        assert_eq!(m.destination, "BUF");
        assert_eq!(m.extras.as_deref(), Some("OS BUF/FUL0680"));
    }

    #[test]
    fn parse_qf_fuel_extras() {
        let m =
            parse_qf("PHL1252CVG/FB 0137/FP 118/CO 856385/FO 928445/A1 031720/A2 266135").unwrap();
        assert_eq!(m.departure, "PHL");
        assert_eq!(m.destination, "CVG");
        assert!(m.extras.is_some());
    }

    #[test]
    fn parse_qq_simple() {
        let m = parse_qq("YPPHYBRY22312220").unwrap();
        assert_eq!(m.departure, "YPPH");
        assert_eq!(m.arrival, "YBRY");
        assert_eq!(m.time_utc, "2231");
        assert_eq!(m.extras.as_deref(), Some("2220"));
    }

    #[test]
    fn parse_qq_with_position() {
        let m = parse_qq("YSCBYSCB2313\r\n001FE24231340S3517.0E14911.3026154").unwrap();
        assert_eq!(m.departure, "YSCB");
        assert_eq!(m.arrival, "YSCB");
        assert_eq!(m.time_utc, "2313");
        assert!(m
            .extras
            .as_deref()
            .map(|s| s.contains("001FE"))
            .unwrap_or(false));
    }

    #[test]
    fn parse_qq_with_fuel() {
        let m = parse_qq("CYYCCYYZ2311/FB  255").unwrap();
        assert_eq!(m.departure, "CYYC");
        assert_eq!(m.arrival, "CYYZ");
        assert_eq!(m.time_utc, "2311");
        assert_eq!(m.extras.as_deref(), Some("/FB  255"));
    }

    #[test]
    fn parse_qq_short_is_none() {
        assert!(parse_qq("KPHL").is_none());
    }
}
