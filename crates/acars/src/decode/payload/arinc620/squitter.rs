//! ARINC 620 / ACARS `SQ` squitter messages.
//!
//! Squitters are ground-station broadcasts that advertise station identity,
//! airport, provider, link, frequency, and sometimes station coordinates. They
//! are useful as link metadata rather than aircraft application traffic.

use serde::{Deserialize, Serialize};

use crate::decode::payload::PayloadError;
use crate::decode::{DecodeError, DecodeResult};

/// ACARS `SQ` squitter / ground-station broadcast.
///
/// Observed formats include:
/// - `00XS` — short heartbeat/status form.
/// - `01XA<station><airport><status><provider>` — station/airport without coordinates.
/// - `02X<station><airport><status><lat><N/S><lon><E/W><link><freq>/<provider>`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SquitterMessage {
    pub version: u8,
    pub kind: char,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub airport: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<SquitterLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_mhz: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub raw: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SquitterLink {
    #[serde(rename = "VHF")]
    Vhf,
    #[serde(rename = "SAT")]
    Satellite,
    Other(char),
}

pub fn parse_squitter(text: &str) -> DecodeResult<SquitterMessage> {
    let raw = text.trim().to_string();
    let bytes = raw.as_bytes();
    if bytes.len() < 4 || !bytes[0].is_ascii_digit() || !bytes[1].is_ascii_digit() {
        return Err(invalid("SQ squitter must start with two version digits"));
    }
    let version = parse_u8(&raw[0..2], "version")?;
    let kind = bytes[2] as char;

    match version {
        0 => parse_v00(raw, version, kind),
        1 => parse_v01(raw, version, kind),
        2 => parse_v02(raw, version, kind),
        _ => Err(invalid(format!(
            "unsupported SQ squitter version {version:02}"
        ))),
    }
}

fn parse_v00(raw: String, version: u8, kind: char) -> DecodeResult<SquitterMessage> {
    Ok(SquitterMessage {
        version,
        kind,
        station: raw.get(3..).filter(|s| !s.is_empty()).map(str::to_string),
        airport: None,
        status: None,
        latitude: None,
        longitude: None,
        link: None,
        frequency_mhz: None,
        provider: None,
        raw,
    })
}

fn parse_v01(raw: String, version: u8, kind: char) -> DecodeResult<SquitterMessage> {
    if raw.len() < 11 {
        return Err(invalid("SQ v01 too short"));
    }
    let station = raw[3..7].to_string();
    let airport = raw[7..11].to_string();
    let rest = &raw[11..];
    let (status, provider) = split_status_provider(rest)?;
    Ok(SquitterMessage {
        version,
        kind,
        station: Some(station),
        airport: Some(airport),
        status,
        latitude: None,
        longitude: None,
        link: None,
        frequency_mhz: None,
        provider,
        raw,
    })
}

fn parse_v02(raw: String, version: u8, kind: char) -> DecodeResult<SquitterMessage> {
    if raw.len() < 30 {
        return Err(invalid("SQ v02 too short"));
    }
    let station = raw[3..7].to_string();
    let airport = raw[7..11].to_string();
    let status = parse_u8(&raw[11..12], "status")?;
    let lat = parse_deg_min(&raw[12..16], raw.as_bytes()[16] as char, 2)?;
    let lon = parse_deg_min(&raw[17..22], raw.as_bytes()[22] as char, 3)?;
    let link = match raw.as_bytes()[23] as char {
        'V' => SquitterLink::Vhf,
        'B' => SquitterLink::Satellite,
        other => SquitterLink::Other(other),
    };
    let freq = parse_u32(&raw[24..30], "frequency")? as f64 / 1000.0;
    let provider = raw.get(30..).and_then(|rest| {
        rest.strip_prefix('/')
            .map(|p| p.to_string())
            .filter(|p| !p.is_empty())
    });

    Ok(SquitterMessage {
        version,
        kind,
        station: Some(station),
        airport: Some(airport),
        status: Some(status),
        latitude: Some(lat),
        longitude: Some(lon),
        link: Some(link),
        frequency_mhz: Some(freq),
        provider,
        raw,
    })
}

fn split_status_provider(rest: &str) -> DecodeResult<(Option<u8>, Option<String>)> {
    if rest.is_empty() {
        return Ok((None, None));
    }
    let mut chars = rest.chars();
    let first = chars.next().expect("non-empty rest");
    let status = if first.is_ascii_digit() {
        Some(first.to_digit(10).expect("ascii digit") as u8)
    } else {
        return Ok((None, Some(rest.to_string())));
    };
    let provider = chars.as_str();
    Ok((
        status,
        if provider.is_empty() {
            None
        } else {
            Some(provider.to_string())
        },
    ))
}

fn parse_deg_min(value: &str, hemi: char, deg_digits: usize) -> DecodeResult<f64> {
    let deg = parse_u32(&value[..deg_digits], "degrees")? as f64;
    let min = parse_u32(&value[deg_digits..], "minutes")? as f64;
    let mut out = deg + min / 60.0;
    match hemi {
        'S' | 'W' => out = -out,
        'N' | 'E' => {}
        _ => return Err(invalid(format!("invalid hemisphere {hemi}"))),
    }
    Ok(out)
}

fn parse_u8(value: &str, field: &str) -> DecodeResult<u8> {
    value
        .parse::<u8>()
        .map_err(|_| invalid(format!("invalid SQ {field}: {value}")))
}

fn parse_u32(value: &str, field: &str) -> DecodeResult<u32> {
    value
        .parse::<u32>()
        .map_err(|_| invalid(format!("invalid SQ {field}: {value}")))
}

fn invalid(message: impl Into<String>) -> DecodeError {
    DecodeError::InvalidPayload(PayloadError::Squitter(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v02_arinc_provider() {
        let msg = parse_squitter("02XAMSNKMSN14308N08920WV136975/ARINC").unwrap();
        assert_eq!(msg.version, 2);
        assert_eq!(msg.station.as_deref(), Some("AMSN"));
        assert_eq!(msg.airport.as_deref(), Some("KMSN"));
        assert_eq!(msg.status, Some(1));
        assert_eq!(msg.link, Some(SquitterLink::Vhf));
        assert_eq!(msg.frequency_mhz, Some(136.975));
        assert_eq!(msg.provider.as_deref(), Some("ARINC"));
        assert!((msg.latitude.unwrap() - 43.133333).abs() < 0.00001);
        assert!((msg.longitude.unwrap() + 89.333333).abs() < 0.00001);
    }

    #[test]
    fn parses_v02_empty_provider() {
        let msg = parse_squitter("02XSSANKSAN03273N11718WV136975/").unwrap();
        assert_eq!(msg.station.as_deref(), Some("SSAN"));
        assert_eq!(msg.airport.as_deref(), Some("KSAN"));
        assert_eq!(msg.status, Some(0));
        assert_eq!(msg.provider, None);
    }

    #[test]
    fn parses_v01() {
        let msg = parse_squitter("01XAYOWKYOW1ARINC").unwrap();
        assert_eq!(msg.version, 1);
        assert_eq!(msg.station.as_deref(), Some("AYOW"));
        assert_eq!(msg.airport.as_deref(), Some("KYOW"));
        assert_eq!(msg.provider.as_deref(), Some("ARINC"));
    }

    #[test]
    fn parses_v00() {
        let msg = parse_squitter("00XS").unwrap();
        assert_eq!(msg.version, 0);
        assert_eq!(msg.kind, 'X');
        assert_eq!(msg.station.as_deref(), Some("S"));
    }
}
