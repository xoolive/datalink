//! ACARS ATIS (Automatic Terminal Information Service) label decoders.
//!
//! Two labels carry ATIS data over the ARINC 623 TI2 datalink:
//!
//! - **B9** (`AtisRequest`): aircraft-to-ground ATIS request.
//!   Format: `/<ICAO4>.TI2/<offset3><ICAO4><atis_letter><crc4>`
//!
//! - **A9** (`AtisDelivery`): ground-to-aircraft ATIS broadcast/response.
//!   Contains ATIS text, often prefixed by slash/dot ARINC 623 TI2 syntax
//!   (`/ATSU.TI2/ICAO ARR ATIS X\n...`).
//!   Surfaced as `Text` payload; the ATIS text itself is the value.

use serde::{Deserialize, Serialize};

/// B9: ATIS request from aircraft to ground station (TI2 protocol).
///
/// The aircraft requests the current ATIS for the specified airport,
/// optionally at a byte offset (for continuation requests).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AtisRequest {
    /// ICAO 4-character airport code for which ATIS is requested.
    pub airport: String,
    /// ATIS information letter currently held by the aircraft (`"A"` through `"Z"`).
    pub current_atis: String,
    /// Byte offset into the ATIS message (0 = start; non-zero = continuation).
    pub offset: u16,
    /// 4-hex-digit checksum of the request payload.
    pub crc: String,
}

/// Parse a B9 (ATIS request) message.
///
/// Expected format: `/<ICAO4>.TI2/<3-digit-offset><ICAO4><atis_letter><4-hex-crc>`
///
/// Strips any leading message-number prefix (e.g. `J41ATK0059/`) before the
/// ARINC 623 TI2 pattern.
pub fn parse_b9(txt: &str) -> Option<AtisRequest> {
    // Find the `.TI2/` marker — there may be a message-number prefix before it.
    let ti2_pos = txt.find(".TI2/")?;

    // Airport code is the 4 chars before ".TI2/"
    let before = &txt[..ti2_pos];
    let slash_pos = before.rfind('/')?;
    let airport = before[slash_pos + 1..].to_ascii_uppercase();
    if airport.len() != 4 || !airport.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }

    // After ".TI2/"
    let after = &txt[ti2_pos + 5..];
    if after.len() < 12 {
        return None;
    }
    let offset_str = &after[..3];
    let airport2 = after[3..7].to_ascii_uppercase();
    if airport2 != airport {
        return None;
    }
    let atis = after[7..8].to_ascii_uppercase();
    let crc = after[8..12].to_ascii_uppercase();

    let offset = offset_str.parse::<u16>().ok()?;

    Some(AtisRequest {
        airport,
        current_atis: atis,
        offset,
        crc,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_b9_simple() {
        let r = parse_b9("/EHAM.TI2/000EHAMA0E3C").unwrap();
        assert_eq!(r.airport, "EHAM");
        assert_eq!(r.current_atis, "A");
        assert_eq!(r.offset, 0);
        assert_eq!(r.crc, "0E3C");
    }

    #[test]
    fn parse_b9_with_offset() {
        let r = parse_b9("/EGCC.TI2/040EGCCA567B").unwrap();
        assert_eq!(r.airport, "EGCC");
        assert_eq!(r.current_atis, "A");
        assert_eq!(r.offset, 40);
    }

    #[test]
    fn parse_b9_with_msg_prefix() {
        // Full message with ACARS message-number prefix before the ARINC 623 TI2 part
        let r = parse_b9("J41ATK0059/LTFM.TI2/000LTFMA7745").unwrap();
        assert_eq!(r.airport, "LTFM");
        assert_eq!(r.current_atis, "A");
        assert_eq!(r.offset, 0);
        assert_eq!(r.crc, "7745");
    }

    #[test]
    fn parse_b9_atis_d() {
        let r = parse_b9("/ZHHH.TI2/040ZHHHD9F98").unwrap();
        assert_eq!(r.airport, "ZHHH");
        assert_eq!(r.current_atis, "D");
        assert_eq!(r.offset, 40);
    }

    #[test]
    fn parse_b9_invalid_returns_none() {
        assert!(parse_b9("SOMEGARBAGETEXT").is_none());
        assert!(parse_b9("/EHA.TI2/000EHAMA0E3C").is_none()); // 3-char airport
    }
}
