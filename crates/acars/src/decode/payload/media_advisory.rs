//! ACARS label `SA` — Media Advisory decoder.
//!
//! Media Advisory messages inform the ground about which communication link
//! an aircraft is currently using and what links are available.
//!
//! ## Wire format (ARINC 620 / libacars `media-adv.c`)
//!
//! ```text
//! ┌──────────┬──────────┬──────────────┬──────────────────┬─────────────────────────┬──────────────┐
//! │ version  │  state   │ current_link │  time (HHMMSS)   │  available_links (0..n) │  text (opt)  │
//! │  1 byte  │  1 byte  │   1 byte     │    6 bytes       │   1 byte each, valid    │   after '/'  │
//! │  ASCII   │  E | L   │  link code   │  ASCII digits    │   link codes            │              │
//! └──────────┴──────────┴──────────────┴──────────────────┴─────────────────────────┴──────────────┘
//! ```
//!
//! The 9-byte fixed header is decoded with deku. The variable-length available-links
//! tail is parsed manually (one char per link until `/` or end of string).
//!
//! ## Link codes
//!
//! | Code | Link |
//! |---|---|
//! | `V` | VHF ACARS |
//! | `S` | Default SATCOM |
//! | `H` | HF |
//! | `G` | Global Star Satcom |
//! | `C` | ICO Satcom |
//! | `2` | VDL2 |
//! | `X` | Inmarsat Aero H/H+/I/L |
//! | `I` | Iridium Satcom |
//!
//! ## Observed fixture
//!
//! ```text
//! 0EV114038V
//! ```
//! Decoded: version 0, VHF ACARS **established** at **11:40:38 UTC**,
//! available: VHF ACARS. Aircraft OE-IVR.
//! Source: `gqrx_20260518_114025_136500000_1800000_fc.raw`, t=14.49s, ch=136.825 MHz.

use deku::prelude::*;
use serde::{Deserialize, Serialize};

use crate::decode::payload::PayloadError;
use crate::decode::{DecodeError, DecodeResult};

/// Link state: established or lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, DekuRead, Serialize, Deserialize)]
#[deku(id_type = "u8")]
pub enum LinkState {
    /// Link established (`E` = 0x45)
    #[deku(id = 0x45)]
    Established,
    /// Link lost (`L` = 0x4C)
    #[deku(id = 0x4C)]
    Lost,
}

/// Communication link type, identified by its single ASCII character code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, DekuRead, Serialize, Deserialize)]
#[deku(id_type = "u8")]
pub enum LinkType {
    /// VHF ACARS (`V` = 0x56)
    #[deku(id = 0x56)]
    VhfAcars,
    /// Default SATCOM (`S` = 0x53)
    #[deku(id = 0x53)]
    Satcom,
    /// HF (`H` = 0x48)
    #[deku(id = 0x48)]
    Hf,
    /// Global Star Satcom (`G` = 0x47)
    #[deku(id = 0x47)]
    GlobalStar,
    /// ICO Satcom (`C` = 0x43)
    #[deku(id = 0x43)]
    IcoSatcom,
    /// VDL2 (`2` = 0x32)
    #[deku(id = 0x32)]
    Vdl2,
    /// Inmarsat Aero H/H+/I/L (`X` = 0x58)
    #[deku(id = 0x58)]
    Inmarsat,
    /// Iridium Satcom (`I` = 0x49)
    #[deku(id = 0x49)]
    Iridium,
}

impl LinkType {
    pub fn description(&self) -> &'static str {
        match self {
            Self::VhfAcars => "VHF ACARS",
            Self::Satcom => "Default SATCOM",
            Self::Hf => "HF",
            Self::GlobalStar => "Global Star Satcom",
            Self::IcoSatcom => "ICO Satcom",
            Self::Vdl2 => "VDL2",
            Self::Inmarsat => "Inmarsat Aero H/H+/I/L",
            Self::Iridium => "Iridium Satcom",
        }
    }

    fn try_from_byte(b: u8) -> Option<Self> {
        match b {
            b'V' => Some(Self::VhfAcars),
            b'S' => Some(Self::Satcom),
            b'H' => Some(Self::Hf),
            b'G' => Some(Self::GlobalStar),
            b'C' => Some(Self::IcoSatcom),
            b'2' => Some(Self::Vdl2),
            b'X' => Some(Self::Inmarsat),
            b'I' => Some(Self::Iridium),
            _ => None,
        }
    }
}

/// Fixed 9-byte Media Advisory header, decoded with deku.
#[derive(Debug, DekuRead)]
struct MediaAdvisoryHeader {
    /// Version digit as raw ASCII byte (`b'0'` = version 0)
    version_byte: u8,
    /// Link state
    state: LinkState,
    /// Current active link
    current_link: LinkType,
    /// UTC time as 6 ASCII digit bytes `[H, H, M, M, S, S]`
    time_bytes: [u8; 6],
}

/// Decoded ACARS label `SA` Media Advisory message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaAdvisory {
    /// Message version (0 = v1)
    pub version: u8,
    /// Link state
    pub state: LinkState,
    /// Current active link
    pub current_link: LinkType,
    /// UTC time as `HHMMSS` string (e.g. `"114038"`)
    pub time_utc: String,
    /// All available links at time of message
    pub available_links: Vec<LinkType>,
    /// Optional free text after `/`
    pub text: Option<String>,
}

/// Parse an ACARS label `SA` Media Advisory text payload.
///
/// Returns `Err` if the text is too short, the fixed header fails to decode,
/// or any field contains an invalid value.
pub fn parse_media_advisory(txt: &str) -> DecodeResult<MediaAdvisory> {
    let bytes = txt.trim().as_bytes();

    // Need at least 9 bytes for the fixed header
    if bytes.len() < 9 {
        return Err(DecodeError::InvalidPayload(PayloadError::MediaAdvisory(
            format!("too short: {} bytes (need 9)", bytes.len()),
        )));
    }

    let ((_, _), header) = MediaAdvisoryHeader::from_bytes((bytes, 0))
        .map_err(|e| DecodeError::Deku(e.to_string()))?;

    let version = header.version_byte.wrapping_sub(b'0');
    let time_utc = std::str::from_utf8(&header.time_bytes)
        .map_err(|_| {
            DecodeError::InvalidPayload(PayloadError::MediaAdvisory(
                "invalid UTF-8 in time field".into(),
            ))
        })?
        .to_string();

    if !time_utc.chars().all(|c| c.is_ascii_digit()) {
        return Err(DecodeError::InvalidPayload(PayloadError::MediaAdvisory(
            format!("non-digit in time field: {:?}", time_utc),
        )));
    }

    // Parse variable-length tail: available links until '/' or end
    let tail = &bytes[9..];
    let slash_pos = tail.iter().position(|&b| b == b'/');
    let avail_bytes = &tail[..slash_pos.unwrap_or(tail.len())];
    let text_bytes = slash_pos.map(|p| &tail[p + 1..]);

    let available_links: Vec<LinkType> = avail_bytes
        .iter()
        .filter_map(|&b| LinkType::try_from_byte(b))
        .collect();

    let text = text_bytes
        .and_then(|b| std::str::from_utf8(b).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    Ok(MediaAdvisory {
        version,
        state: header.state,
        current_link: header.current_link,
        time_utc,
        available_links,
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Observed fixture: OE-IVR, gqrx recording t=14.49s, ch=136.825 MHz
    const FIXTURE: &str = "0EV114038V";

    #[test]
    fn test_parse_fixture() {
        let msg = parse_media_advisory(FIXTURE).expect("should parse");
        assert_eq!(msg.version, 0);
        assert_eq!(msg.state, LinkState::Established);
        assert_eq!(msg.current_link, LinkType::VhfAcars);
        assert_eq!(msg.time_utc, "114038");
        assert_eq!(msg.available_links, vec![LinkType::VhfAcars]);
        assert!(msg.text.is_none());
    }

    #[test]
    fn test_link_state_enum() {
        assert_eq!(msg("0EV114038V").state, LinkState::Established);
        assert_eq!(msg("0L2120000S").state, LinkState::Lost);
    }

    #[test]
    fn test_link_type_enum() {
        assert_eq!(msg("0EV114038V").current_link, LinkType::VhfAcars);
        assert_eq!(msg("0L2120000S").current_link, LinkType::Vdl2);
        assert_eq!(msg("0EX093015V").current_link, LinkType::Inmarsat);
    }

    #[test]
    fn test_parse_link_lost_vdl2() {
        // version=0, state=L (lost), current=2 (VDL2), time=120000, available=S
        let msg = msg("0L2120000S");
        assert_eq!(msg.version, 0);
        assert_eq!(msg.state, LinkState::Lost);
        assert_eq!(msg.current_link, LinkType::Vdl2);
        assert_eq!(msg.time_utc, "120000");
        assert_eq!(msg.available_links, vec![LinkType::Satcom]);
    }

    #[test]
    fn test_multiple_available_links() {
        let msg = msg("0EV093015VS2");
        assert_eq!(
            msg.available_links,
            vec![LinkType::VhfAcars, LinkType::Satcom, LinkType::Vdl2]
        );
    }

    #[test]
    fn test_with_free_text() {
        let msg = msg("0EV120000V/some advisory text");
        assert_eq!(msg.available_links, vec![LinkType::VhfAcars]);
        assert_eq!(msg.text.as_deref(), Some("some advisory text"));
    }

    #[test]
    fn test_too_short_returns_err() {
        assert!(parse_media_advisory("0EV").is_err());
        assert!(parse_media_advisory("").is_err());
    }

    #[test]
    fn test_invalid_state_returns_err() {
        // 'X' is not a valid state (must be E or L)
        assert!(parse_media_advisory("0XV114038V").is_err());
    }

    #[test]
    fn test_link_type_description() {
        assert_eq!(LinkType::VhfAcars.description(), "VHF ACARS");
        assert_eq!(LinkType::Vdl2.description(), "VDL2");
        assert_eq!(LinkType::Inmarsat.description(), "Inmarsat Aero H/H+/I/L");
    }

    #[test]
    fn test_json_serializes() {
        let msg = parse_media_advisory(FIXTURE).unwrap();
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["state"], "Established");
        assert_eq!(json["current_link"], "VhfAcars");
        assert_eq!(json["time_utc"], "114038");
        assert_eq!(json["available_links"][0], "VhfAcars");
    }

    fn msg(txt: &str) -> MediaAdvisory {
        parse_media_advisory(txt).expect("should parse")
    }
}
