//! ARINC 622 AFN (ATS Facilities Notification) text payload parser.
//!
//! Observed on ACARS labels A0/B0 as slash/dot messages such as:
//! `/EGTT.AFN/FMHBAW50G,.G-YMMG,4007F2,154626/FPON51283W000285,1/FCOADS,01/FCOATC,01CB69`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AfnMessage {
    pub facility: String,
    pub message_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flight_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icao24: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<AfnPosition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub applications: Vec<AfnApplication>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AfnPosition {
    pub raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AfnApplication {
    pub code: String,
    pub value: String,
}

pub fn parse_afn(text: &str) -> Option<AfnMessage> {
    let raw = text.trim().to_string();
    let after_slash = raw.strip_prefix('/')?;
    let (facility, rest) = after_slash.split_once(".AFN/")?;
    if facility.is_empty() {
        return None;
    }

    let mut checksum = None;
    let mut parts: Vec<&str> = rest.split('/').collect();
    if let Some(last) = parts.last_mut() {
        if last.len() >= 4 {
            let suffix = &last[last.len() - 4..];
            if suffix.chars().all(|c| c.is_ascii_hexdigit()) {
                checksum = Some(suffix.to_ascii_uppercase());
                *last = &last[..last.len() - 4];
            }
        }
    }

    let first = parts.first().copied().unwrap_or_default();
    let mut first_fields = first.split(',');
    let first0 = first_fields.next().unwrap_or_default();
    let message_type = first0.get(..3).unwrap_or(first0).to_string();
    let flight_id = first0
        .get(3..)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let registration = first_fields
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_start_matches('.').to_string());
    let icao24 = first_fields
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_uppercase);
    let timestamp = first_fields
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let mut position = None;
    let mut applications = Vec::new();
    for part in parts.iter().skip(1).copied().filter(|p| !p.is_empty()) {
        if let Some(rest) = part.strip_prefix("FPO") {
            let mut fields = rest.split(',');
            position = Some(AfnPosition {
                raw: fields.next().unwrap_or_default().to_string(),
                sequence: fields.next().filter(|s| !s.is_empty()).map(str::to_string),
            });
        } else if let Some(rest) = part.strip_prefix("FAR") {
            if rest.len() >= 3 {
                let code = rest[..3].to_string();
                let value = rest[3..].trim_start_matches(',').to_string();
                applications.push(AfnApplication { code, value });
            }
        } else if let Some(rest) = part.strip_prefix("FCO") {
            if rest.len() >= 3 {
                let code = rest[..3].to_string();
                let value = rest[3..].trim_start_matches(',').to_string();
                applications.push(AfnApplication { code, value });
            }
        }
    }

    Some(AfnMessage {
        facility: facility.to_string(),
        message_type,
        flight_id,
        registration,
        icao24,
        timestamp,
        position,
        applications,
        checksum,
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_afn_contact() {
        let msg = parse_afn(
            "/EGTT.AFN/FMHBAW50G,.G-YMMG,4007F2,154626/FPON51283W000285,1/FCOADS,01/FCOATC,01CB69",
        )
        .unwrap();
        assert_eq!(msg.facility, "EGTT");
        assert_eq!(msg.message_type, "FMH");
        assert_eq!(msg.flight_id.as_deref(), Some("BAW50G"));
        assert_eq!(msg.registration.as_deref(), Some("G-YMMG"));
        assert_eq!(msg.icao24.as_deref(), Some("4007F2"));
        assert_eq!(msg.applications.len(), 2);
        assert_eq!(msg.checksum.as_deref(), Some("CB69"));
    }
}
