//! Oceanic clearance style ARINC 622 text payloads (observed IMI `OC1`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OceanicClearance {
    pub facility: String,
    pub protocol: String,
    pub clearance_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clearance_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flight_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_point: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mach: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flight_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remarks: Option<String>,
    pub raw: String,
}

pub fn parse_oceanic(text: &str) -> Option<OceanicClearance> {
    let raw = text.trim().to_string();
    let after_slash = raw.strip_prefix('/')?;
    let (facility, rest) = after_slash.split_once(".OC1/")?;
    let mut lines = rest.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next()?;
    let mut first_parts = first.split_whitespace();
    let clearance_type = first_parts.next()?.to_string();
    let clearance_number = first_parts.next().map(str::to_string);

    let second = lines.next().unwrap_or_default();
    let mut flight_id = None;
    let mut entry_point = None;
    let mut entry_time = None;
    let mut mach = None;
    let mut flight_level = None;
    if let Some((lhs, rest)) = second.split_once('/') {
        if let Some((flt, entry)) = lhs.split_once('-') {
            flight_id = Some(flt.to_string());
            entry_point = Some(entry.to_string());
        }
        let fields: Vec<&str> = rest.split_whitespace().collect();
        if let Some(first) = fields.first() {
            entry_time = Some((*first).to_string());
        }
        for field in fields.iter().skip(1) {
            if field.starts_with('M') {
                mach = Some((*field).to_string());
            } else if field.starts_with('F') {
                flight_level = Some((*field).to_string());
            }
        }
    }

    let remarks = lines
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .strip_prefix("-RMK/")
        .or_else(|| raw.split("\n-RMK/").nth(1))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    Some(OceanicClearance {
        facility: facility.to_string(),
        protocol: "OC1".to_string(),
        clearance_type,
        clearance_number,
        flight_id,
        entry_point,
        entry_time,
        mach,
        flight_level,
        remarks,
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_oc1_rcl() {
        let msg =
            parse_oceanic("/EGGX.OC1/RCL 030\r\nBAW71K-BALIX/1628 M084F360\r\n-RMK/MAX F3805082")
                .unwrap();
        assert_eq!(msg.facility, "EGGX");
        assert_eq!(msg.clearance_type, "RCL");
        assert_eq!(msg.flight_id.as_deref(), Some("BAW71K"));
        assert_eq!(msg.entry_point.as_deref(), Some("BALIX"));
        assert_eq!(msg.mach.as_deref(), Some("M084F360"));
    }
}
