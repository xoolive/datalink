//! Generic AOC position/telemetry text parsers for several ACARS labels.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AocPositionMessage {
    pub format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub altitude_ft: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_deg: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub departure: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
    pub raw: String,
}

pub fn parse_aoc_position(label: &str, text: &str) -> Option<AocPositionMessage> {
    let raw = text.trim().to_string();
    if raw.is_empty() {
        return None;
    }
    match label {
        "21" => parse_label21(&raw),
        "22" | "31" | "36" => parse_latlon_text(label, &raw),
        "44" => parse_label44(&raw),
        "83" => parse_label83(&raw),
        _ => None,
    }
}

fn parse_label21(raw: &str) -> Option<AocPositionMessage> {
    let rest = raw.strip_prefix("POSN ")?;
    let parts: Vec<&str> = rest.split(',').map(str::trim).collect();
    let mut first_fields = parts.first()?.split_whitespace();
    let _posn_lat = first_fields.next();
    let lon_token = first_fields.next();
    let (lat, lon) = if let (Some(lat_token), Some(lon_token)) = (_posn_lat, lon_token) {
        if lat_token
            .chars()
            .last()
            .is_some_and(|c| matches!(c, 'N' | 'S'))
            && lon_token
                .chars()
                .last()
                .is_some_and(|c| matches!(c, 'E' | 'W'))
        {
            (
                parse_trailing_hemi(lat_token)?,
                parse_trailing_hemi(lon_token)?,
            )
        } else {
            parse_decimal_pair(parts.first()?)?
        }
    } else {
        parse_decimal_pair(parts.first()?)?
    };
    Some(AocPositionMessage {
        format: "label21".into(),
        latitude: Some(lat),
        longitude: Some(lon),
        heading_deg: parts.get(1).and_then(|s| s.parse().ok()),
        timestamp: parts
            .get(2)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        altitude_ft: parts.get(3).and_then(|s| s.parse().ok()),
        departure: None,
        destination: parts.get(8).filter(|s| s.len() >= 3).map(|s| s.to_string()),
        raw: raw.to_string(),
    })
}

fn parse_latlon_text(label: &str, raw: &str) -> Option<AocPositionMessage> {
    let (lat, lon) = find_latlon(raw)?;
    let parts: Vec<&str> = raw.split(&[',', '\n', '\r'][..]).map(str::trim).collect();
    Some(AocPositionMessage {
        format: format!("label{label}"),
        latitude: Some(lat),
        longitude: Some(lon),
        timestamp: parts
            .iter()
            .find(|s| s.len() == 6 && s.chars().all(|c| c.is_ascii_digit()))
            .map(|s| (*s).to_string()),
        altitude_ft: parts
            .iter()
            .find_map(|s| s.parse::<i32>().ok().filter(|v| *v > 1000 && *v < 60000)),
        heading_deg: None,
        departure: None,
        destination: None,
        raw: raw.to_string(),
    })
}

fn parse_label44(raw: &str) -> Option<AocPositionMessage> {
    let parts: Vec<&str> = raw.split(',').map(str::trim).collect();
    let coord = parts.get(1)?;
    let (lat, lon) = parse_compact_ns_ew(coord)?;
    Some(AocPositionMessage {
        format: "label44".into(),
        latitude: Some(lat),
        longitude: Some(lon),
        heading_deg: parts.get(2).and_then(|s| s.parse().ok()),
        departure: parts
            .get(3)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        destination: parts
            .get(4)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        timestamp: parts
            .get(6)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        altitude_ft: None,
        raw: raw.to_string(),
    })
}

fn parse_label83(raw: &str) -> Option<AocPositionMessage> {
    let ns = raw.find('N').or_else(|| raw.find('S'))?;
    let ew_rel = raw[ns + 1..]
        .find('W')
        .or_else(|| raw[ns + 1..].find('E'))?;
    let ew = ns + 1 + ew_rel;
    let lat = parse_deg_min_decimal(&raw[ns..ew], 2)?;
    let end = raw[ew + 1..]
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .map(|i| ew + 1 + i)
        .unwrap_or(raw.len());
    let lon = parse_deg_min_decimal(&raw[ew..end], 3)?;
    Some(AocPositionMessage {
        format: "label83".into(),
        latitude: Some(lat),
        longitude: Some(lon),
        timestamp: raw
            .get(5..11)
            .filter(|s| s.chars().all(|c| c.is_ascii_digit()))
            .map(str::to_string),
        altitude_ft: None,
        heading_deg: None,
        departure: None,
        destination: None,
        raw: raw.to_string(),
    })
}

fn parse_decimal_pair(s: &str) -> Option<(f64, f64)> {
    let s = s.trim();
    let ew = s[1..].find(['E', 'W']).map(|i| i + 1)?;
    let sep = s[..ew]
        .rfind(|c: char| !c.is_ascii_digit() && c != '.')
        .map(|i| i + 1)
        .unwrap_or(0);
    let lat_part = s[..sep].trim();
    let lon_part = s[sep..].trim();
    let lat = if lat_part.is_empty() {
        None
    } else {
        parse_signed_decimal_token(lat_part)
    };
    let lon = parse_signed_decimal_token(lon_part)?;
    Some((lat.unwrap_or(0.0), lon))
}

fn find_latlon(s: &str) -> Option<(f64, f64)> {
    let tokens: Vec<&str> = s.split_whitespace().collect();
    for w in tokens.windows(2) {
        if let (Some(lat), Some(lon)) = (
            parse_signed_decimal_token(w[0]),
            parse_signed_decimal_token(w[1]),
        ) {
            return Some((lat, lon));
        }
    }
    None
}

fn parse_trailing_hemi(tok: &str) -> Option<f64> {
    let hemi = tok.chars().last()?;
    let sign = match hemi {
        'N' | 'E' => 1.0,
        'S' | 'W' => -1.0,
        _ => return None,
    };
    tok[..tok.len() - 1].parse::<f64>().ok().map(|v| sign * v)
}

fn parse_signed_decimal_token(tok: &str) -> Option<f64> {
    let tok = tok.trim_matches(',');
    let hemi = tok.chars().next()?;
    let sign = match hemi {
        'N' | 'E' => 1.0,
        'S' | 'W' => -1.0,
        _ => return None,
    };
    tok[1..].parse::<f64>().ok().map(|v| sign * v)
}

fn parse_compact_ns_ew(s: &str) -> Option<(f64, f64)> {
    let ew = s[1..].find(['E', 'W']).map(|i| i + 1)?;
    let lat = parse_deg_min_decimal(&s[..ew], 2)?;
    let lon = parse_deg_min_decimal(&s[ew..], 3)?;
    Some((lat, lon))
}

fn parse_deg_min_decimal(s: &str, deg_digits: usize) -> Option<f64> {
    let hemi = s.chars().next()?;
    let sign = match hemi {
        'N' | 'E' => 1.0,
        'S' | 'W' => -1.0,
        _ => return None,
    };
    let rest = &s[1..];
    if rest.len() <= deg_digits {
        return None;
    }
    let deg: f64 = rest[..deg_digits].parse().ok()?;
    let min: f64 = rest[deg_digits..].parse().ok()?;
    Some(sign * (deg + min / 60.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_label21() {
        let msg = parse_aoc_position(
            "21",
            "POSN N28.282 W82.571, 266,154806,10434,25363,  29,  6,174854,KCLE",
        )
        .unwrap();
        assert!((msg.latitude.unwrap() - 28.282).abs() < 0.001);
        assert!((msg.longitude.unwrap() + 82.571).abs() < 0.001);
        assert_eq!(msg.destination.as_deref(), Some("KCLE"));
    }

    #[test]
    fn parses_label44() {
        let msg = parse_aoc_position(
            "44",
            "00POS03,N39436W075032,430,KSRQ,KLCI,0530,1548,1641,004.2",
        )
        .unwrap();
        assert!(msg.latitude.unwrap() > 39.0);
        assert!(msg.longitude.unwrap() < -75.0);
        assert_eq!(msg.departure.as_deref(), Some("KSRQ"));
    }
}
