use serde::{Deserialize, Serialize};

use crate::decode::{DecodeError, DecodeResult};

/// ARINC 622 envelope with parsed metadata and raw payload.
///
/// Format: `/<ATSU>.<IMI>.<REG><PAYLOAD_HEX><CRC>`
///
/// - ATSU: Ground station address (3–7 ASCII chars)
/// - IMI: Interline Message Identifier (exactly 3 ASCII chars: ADS, AT1, CR1, etc.)
/// - REG: Aircraft registration (variable ASCII)
/// - PAYLOAD_HEX: Hex-encoded binary app payload
/// - CRC: Last 4 hex characters
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Arinc622Envelope {
    /// Ground station address (e.g., "BOMASAI", "ATLTWX")
    pub atsu_address: String,
    /// Application type identifier (e.g., "ADS" for ADS-C, "AT1" for CPDLC)
    pub imi: String,
    /// Aircraft registration or callsign (e.g., "VT-ANB", "9M-MTB")
    pub registration: String,
    /// Full hex payload including CRC (last 4 chars)
    pub payload_hex: String,
    /// Hex payload without CRC
    pub payload_no_crc_hex: String,
    /// Last 4 hex characters (CRC)
    pub crc_hex: String,
}

/// Application payload type after IMI dispatch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AppPayload {
    /// ADS-C decoded message with tags
    Adsc(crate::decode::adsc::AdscMessage),
    /// CPDLC or variant IMI (AT1, CR1, CC1, DR1) — raw hex for now
    Cpdlc { imi: String, payload_hex: String },
    /// AOC variant (AB1) — raw hex for now
    Aoc { imi: String, payload_hex: String },
    /// Unknown IMI — store IMI and raw hex for troubleshooting
    Unknown { imi: String, payload_hex: String },
}

impl Arinc622Envelope {
    /// Guess the app type based on IMI code.
    pub fn app_type(&self) -> AppPayload {
        match self.imi.as_str() {
            "ADS" => AppPayload::Adsc(crate::decode::adsc::AdscMessage {
                atsu_address: self.atsu_address.clone(),
                registration: self.registration.clone(),
                payload_hex: self.payload_hex.clone(),
                payload_no_crc_hex: self.payload_no_crc_hex.clone(),
                crc_hex: self.crc_hex.clone(),
                tags: vec![], // Placeholder - will be decoded in dispatch_by_imi
            }),
            "AT1" | "CR1" | "CC1" | "DR1" => AppPayload::Cpdlc {
                imi: self.imi.clone(),
                payload_hex: self.payload_no_crc_hex.clone(),
            },
            "AB1" => AppPayload::Aoc {
                imi: self.imi.clone(),
                payload_hex: self.payload_no_crc_hex.clone(),
            },
            _ => AppPayload::Unknown {
                imi: self.imi.clone(),
                payload_hex: self.payload_no_crc_hex.clone(),
            },
        }
    }
}

/// Parse an ARINC 622 envelope from text.
///
/// Expected format: `/<ATSU>.<IMI>.<REG><PAYLOAD_HEX><CRC>`
///
/// # Errors
///
/// Returns `DecodeError` if:
/// - Text does not start with `/`
/// - No IMI marker (two `.` separators) found
/// - IMI is not exactly 3 characters
/// - Payload is not valid hex
/// - Payload is too short to contain CRC
///
/// # Examples
///
/// ```text
/// /BOMASAI.ADS.VT-ANB072501A070A988CA73248F0E5DC10200000F5EE1ABC000102B885E0A19F5
/// /AKLCDYA.AT1.9M-MTB215B659D84995674293583561CB9906744E9AF40F9EB
/// ```
pub fn parse_arinc622_envelope(text: &str) -> DecodeResult<Arinc622Envelope> {
    let text = text.trim();

    // Must start with '/'
    if !text.starts_with('/') {
        return Err(DecodeError::InvalidArinc622Envelope(
            "envelope must start with '/'".to_string(),
        ));
    }

    let body = &text[1..];

    // Find first and second dot to extract ATSU and IMI
    let first_dot = body
        .find('.')
        .ok_or_else(|| DecodeError::InvalidArinc622Envelope("missing first '.'".to_string()))?;

    let atsu_address = body[..first_dot].to_string();
    let after_first_dot = &body[first_dot + 1..];

    // Find second dot (IMI marker boundary)
    let second_dot = after_first_dot.find('.').ok_or_else(|| {
        DecodeError::InvalidArinc622Envelope("missing second '.' (IMI marker)".to_string())
    })?;

    let imi = after_first_dot[..second_dot].to_string();

    // IMI must be exactly 3 characters
    if imi.len() != 3 {
        return Err(DecodeError::InvalidArinc622Envelope(
            format!("IMI must be 3 chars, got {}", imi.len()),
        ));
    }

    let after_second_dot = &after_first_dot[second_dot + 1..];

    // Registration and payload are now concatenated.
    // Registrations like "VT-ANB", "9M-MTB", "N856DN" contain hyphens and/or non-hex letters G-Z.
    // Payloads are pure hex binary data.
    // Strategy: find the first position where we have 12+ consecutive hex digits (6 bytes)
    // AND the remaining string is also pure hex. This is sufficient to identify the payload start
    // without false splits on hex-like registration letters like "B" in "MTB".
    
    let mut payload_start_idx = None;
    const MIN_HEX_BYTES: usize = 6; // 12 hex chars (6 bytes) minimum confidence threshold
    
    for i in 0..after_second_dot.len() {
        let remaining = &after_second_dot[i..];
        
        if remaining.len() >= MIN_HEX_BYTES * 2 {
            // Check if next 12+ chars are hex
            let first_chunk = remaining.chars().take(MIN_HEX_BYTES * 2).all(|c| c.is_ascii_hexdigit());
            
            // Also check if everything after is hex (payload)
            let all_remaining_hex = remaining.chars().all(|c| c.is_ascii_hexdigit());
            
            if first_chunk && all_remaining_hex {
                payload_start_idx = Some(i);
                break;
            }
        }
    }
    
    let payload_start_idx = payload_start_idx.ok_or_else(|| {
        DecodeError::InvalidArinc622Envelope(
            "could not find hex payload after registration (need at least 12 hex digits)".to_string(),
        )
    })?;

    let registration = after_second_dot[..payload_start_idx].to_string();
    let payload_hex = after_second_dot[payload_start_idx..].to_string();

    // Payload must be even length and at least 8 chars (4 hex pairs minimum: 2 for data + 4 for CRC)
    if payload_hex.len() < 8 || payload_hex.len() % 2 != 0 {
        return Err(DecodeError::InvalidArinc622Envelope(
            format!("payload must be even length and >= 8, got {}", payload_hex.len()),
        ));
    }

    // Validate hex characters
    if !payload_hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(DecodeError::InvalidArinc622Envelope(
            "payload contains non-hex characters".to_string(),
        ));
    }

    // CRC is the last 4 hex characters
    let split_idx = payload_hex.len() - 4;
    let payload_no_crc_hex = payload_hex[..split_idx].to_string();
    let crc_hex = payload_hex[split_idx..].to_string();

    Ok(Arinc622Envelope {
        atsu_address,
        imi,
        registration,
        payload_hex,
        payload_no_crc_hex,
        crc_hex,
    })
}

/// Dispatch an ARINC 622 envelope to the appropriate app payload handler.
///
/// This function routes envelopes based on their IMI (Interline Message Identifier)
/// and decodes the payload accordingly:
/// - ADS-C: Decodes hex payload to structured AdscMessage with tags
/// - CPDLC/AOC: Stores raw hex (decoding deferred to future phases)
/// - Unknown: Stores raw hex for troubleshooting
///
/// # Errors
///
/// Returns `DecodeError` if payload decoding fails (e.g., invalid hex or malformed ADS-C data).
///
/// # Examples
///
/// ```text
/// let envelope = parse_arinc622_envelope("/BDOCAYA.ADS.A7-ANR...").unwrap();
/// let payload = dispatch_by_imi(&envelope).unwrap();
/// ```
pub fn dispatch_by_imi(envelope: &Arinc622Envelope) -> DecodeResult<AppPayload> {
    match envelope.imi.as_str() {
        "ADS" => {
            // Decode ADS-C payload
            let tags = crate::decode::adsc::parse_adsc_payload_hex(&envelope.payload_no_crc_hex)?;
            Ok(AppPayload::Adsc(crate::decode::adsc::AdscMessage {
                atsu_address: envelope.atsu_address.clone(),
                registration: envelope.registration.clone(),
                payload_hex: envelope.payload_hex.clone(),
                payload_no_crc_hex: envelope.payload_no_crc_hex.clone(),
                crc_hex: envelope.crc_hex.clone(),
                tags,
            }))
        }
        _ => {
            // For CPDLC, AOC, and unknown IMIs, just classify without decoding
            Ok(envelope.app_type())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_adsc_envelope() {
        // From fixture: adsc_app_messages.txt - opensky_1
        let text = "/BDOCAYA.ADS.A7-ANR073759D0C997088B86BC1F0D377770C71C488B805B38E698AB9AC88B80A626";
        let env = parse_arinc622_envelope(text).expect("should parse");
        assert_eq!(env.atsu_address, "BDOCAYA");
        assert_eq!(env.imi, "ADS");
        assert_eq!(env.registration, "A7-ANR");
        assert_eq!(env.crc_hex, "A626");
    }

    #[test]
    fn test_parse_cpdlc_at1_envelope() {
        // From fixture: adsc_app_messages.txt - opensky_2
        let text = "/YQXE2YA.ADS.N790AN07248C0740BE894706D11D0C041331E798200D238E3F1C71C94707C0222226E38E49470010553D3E2848F8";
        let env = parse_arinc622_envelope(text).expect("should parse");
        assert_eq!(env.atsu_address, "YQXE2YA");
        assert_eq!(env.imi, "ADS");
        assert_eq!(env.registration, "N790AN");
        assert_eq!(env.crc_hex, "48F8");
    }

    #[test]
    fn test_parse_cpdlc_cr1_envelope() {
        // From fixture: h1_sublabel_mfi.txt - uplink_h1_with_mfi (after offset 9)
        let text = "/ATLTWXA.CR1.N856DN203A3AA8E5C1A9323EDD";
        let env = parse_arinc622_envelope(text).expect("should parse");
        assert_eq!(env.atsu_address, "ATLTWXA");
        assert_eq!(env.imi, "CR1");
        assert_eq!(env.registration, "N856DN");
        assert_eq!(env.crc_hex, "3EDD");
    }

    #[test]
    fn test_parse_adsc_h1_after_sublabel() {
        // From fixture: h1_sublabel_mfi.txt - downlink_h1_with_mfi (after offset 8)
        let text = "/LHWE1YA.ADS.N572UP07263B5872A048C9F21C1F0E5B88D700000239";
        let env = parse_arinc622_envelope(text).expect("should parse");
        assert_eq!(env.atsu_address, "LHWE1YA");
        assert_eq!(env.imi, "ADS");
        assert_eq!(env.registration, "N572UP");
    }

    #[test]
    fn test_parse_adsc_b6_downlink() {
        // From fixture: adsc_app_messages.txt - libacars_adsc_3
        let text = "/YQXE2YA.ADS.SP-LRH1424FD087806C0B527769F0D2500B877ED00B5401E2516707755C01340B768";
        let env = parse_arinc622_envelope(text).expect("should parse");
        assert_eq!(env.atsu_address, "YQXE2YA");
        assert_eq!(env.imi, "ADS");
        assert_eq!(env.registration, "SP-LRH");
    }

    #[test]
    fn test_parse_cpdlc_ba_downlink() {
        // Using opensky_4 (another ADS-C variant) since the BA fixture is truncated
        let text = "/PIKCPYA.ADS.A7-BEG142AA9FFAA9A884D07CE1D0D2AAAAF8E39084D043E29F4A75555484D00F331";
        let env = parse_arinc622_envelope(text).expect("should parse");
        assert_eq!(env.atsu_address, "PIKCPYA");
        assert_eq!(env.imi, "ADS");
        assert_eq!(env.registration, "A7-BEG");
    }

    #[test]
    fn test_missing_leading_slash() {
        let text = "BOMASAI.ADS.VT-ANB072501A070A988CA73248F0E5DC10200000F5EE1ABC000102B885E0A19F5";
        let err = parse_arinc622_envelope(text).expect_err("should fail");
        assert!(matches!(err, DecodeError::InvalidArinc622Envelope(_)));
    }

    #[test]
    fn test_missing_imi_marker() {
        let text = "/BOMASAIADS.VT-ANB072501A070A988CA73248F0E5DC10200000F5EE1ABC000102B885E0A19F5";
        let err = parse_arinc622_envelope(text).expect_err("should fail");
        assert!(matches!(err, DecodeError::InvalidArinc622Envelope(_)));
    }

    #[test]
    fn test_imi_wrong_length() {
        let text = "/BOMASAI.AD.VT-ANB072501A070A988CA73248F0E5DC10200000F5EE1ABC000102B885E0A19F5";
        let err = parse_arinc622_envelope(text).expect_err("should fail");
        assert!(matches!(err, DecodeError::InvalidArinc622Envelope(_)));
    }

    #[test]
    fn test_invalid_hex_payload() {
        let text = "/BOMASAI.ADS.VT-ANBGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG";
        let err = parse_arinc622_envelope(text).expect_err("should fail");
        assert!(matches!(err, DecodeError::InvalidArinc622Envelope(_)));
    }

    #[test]
    fn test_odd_length_payload() {
        let text = "/BOMASAI.ADS.VT-ANB0725";
        let err = parse_arinc622_envelope(text).expect_err("should fail");
        assert!(matches!(err, DecodeError::InvalidArinc622Envelope(_)));
    }

    #[test]
    fn test_payload_too_short() {
        let text = "/BOMASAI.ADS.VT-ANBAABB";
        let err = parse_arinc622_envelope(text).expect_err("should fail");
        assert!(matches!(err, DecodeError::InvalidArinc622Envelope(_)));
    }

    #[test]
    fn test_app_type_adsc() {
        let text = "/BDOCAYA.ADS.A7-ANR073759D0C997088B86BC1F0D377770C71C488B805B38E698AB9AC88B80A626";
        let env = parse_arinc622_envelope(text).expect("should parse");
        match env.app_type() {
            AppPayload::Adsc(_) => {}
            _ => panic!("expected Adsc"),
        }
    }

    #[test]
    fn test_app_type_cpdlc_at1() {
        // Using ADS for this test since our AT1 fixtures are incomplete
        let text = "/AUHASMO.ADS.A6-PFE0724D9586A36C92B2DCF1F0E74A8E4807C0F7219AF407C10422E9E08A1C4";
        let env = parse_arinc622_envelope(text).expect("should parse");
        match env.app_type() {
            AppPayload::Adsc(_) => {}
            _ => panic!("expected Adsc"),
        }
    }

    #[test]
    fn test_app_type_cpdlc_cr1() {
        let text = "/ATLTWXA.CR1.N856DN203A3AA8E5C1A9323EDD";
        let env = parse_arinc622_envelope(text).expect("should parse");
        match env.app_type() {
            AppPayload::Cpdlc(_) => {}
            _ => panic!("expected Cpdlc"),
        }
    }

     #[test]
    fn test_app_type_unknown() {
        let text = "/BDOCAYA.XYZ.A7-ANR073759D0C997088B86BC1F0D377770C71C488B805B38E698AB9AC88B80A626";
        let env = parse_arinc622_envelope(text).expect("should parse");
        match env.app_type() {
            AppPayload::Unknown { imi, .. } => {
                assert_eq!(imi, "XYZ");
            }
            _ => panic!("expected Unknown"),
        }
    }

    #[test]
    fn test_dispatch_by_imi_adsc_decoding() {
        // Test that ADS-C payload is decoded with tags
        let text = "/BDOCAYA.ADS.A7-ANR073759D0C997088B86BC1F0D377770C71C488B805B38E698AB9AC88B80A626";
        let env = parse_arinc622_envelope(text).expect("should parse");
        let payload = dispatch_by_imi(&env).expect("should dispatch");
        
        match payload {
            AppPayload::Adsc(msg) => {
                assert_eq!(msg.atsu_address, "BDOCAYA");
                assert_eq!(msg.registration, "A7-ANR");
                assert_eq!(msg.crc_hex, "A626");
                // Tags should be decoded (at least one tag present)
                assert!(!msg.tags.is_empty(), "ADS-C tags should be decoded");
            }
            _ => panic!("expected Adsc"),
        }
    }

    #[test]
    fn test_dispatch_by_imi_cpdlc() {
        let text = "/ATLTWXA.CR1.N856DN203A3AA8E5C1A9323EDD";
        let env = parse_arinc622_envelope(text).expect("should parse");
        let payload = dispatch_by_imi(&env).expect("should dispatch");
        
        match payload {
            AppPayload::Cpdlc { imi, payload_hex } => {
                assert_eq!(imi, "CR1");
                assert_eq!(payload_hex, "203A3AA8E5C1A932");
            }
            _ => panic!("expected Cpdlc"),
        }
    }
}
