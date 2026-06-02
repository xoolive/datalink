//! ACARS label 32 CSV telemetry parser.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Label32Message {
    pub fields: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub altitude_ft: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_deg: Option<i32>,
    pub raw: String,
}

pub fn parse_label32(text: &str) -> Option<Label32Message> {
    let raw = text.trim().to_string();
    let fields: Vec<String> = raw.split(',').map(|s| s.trim().to_string()).collect();
    if fields.len() < 6 {
        return None;
    }
    let timestamp = fields
        .iter()
        .find(|f| f.len() >= 19 && f.as_bytes().get(4) == Some(&b'-'))
        .cloned();
    let (latitude, longitude) = fields
        .iter()
        .find_map(|f| parse_latlon_field(f))
        .unwrap_or((None, None));
    let pos_idx = fields.iter().position(|f| parse_latlon_field(f).is_some());
    let altitude_ft = pos_idx
        .and_then(|i| fields.get(i + 1))
        .and_then(|s| s.parse::<i32>().ok());
    let heading_deg = pos_idx
        .and_then(|i| fields.get(i + 2))
        .and_then(|s| s.parse::<i32>().ok());
    Some(Label32Message {
        fields,
        timestamp,
        latitude,
        longitude,
        altitude_ft,
        heading_deg,
        raw,
    })
}

fn parse_latlon_field(s: &str) -> Option<(Option<f64>, Option<f64>)> {
    let mut parts = s.split_whitespace();
    let lat = parse_hemi_decimal(parts.next()?)?;
    let lon = parse_hemi_decimal(parts.next()?)?;
    Some((Some(lat), Some(lon)))
}

fn parse_hemi_decimal(s: &str) -> Option<f64> {
    let hemi = s.chars().next()?;
    let sign = match hemi {
        'N' | 'E' => 1.0,
        'S' | 'W' => -1.0,
        _ => return None,
    };
    s[1..].parse::<f64>().ok().map(|v| sign * v)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_label32_csv() {
        let msg = parse_label32(
            "P,R,2026-05-30 15:29:16,4276,68100,N40.706 W087.395,24350,349,12,-21,474,462,747,,",
        )
        .unwrap();
        assert_eq!(msg.timestamp.as_deref(), Some("2026-05-30 15:29:16"));
        assert!((msg.latitude.unwrap() - 40.706).abs() < 0.001);
        assert!((msg.longitude.unwrap() + 87.395).abs() < 0.001);
        assert_eq!(msg.altitude_ft, Some(24350));
        assert_eq!(msg.heading_deg, Some(349));
    }
}
