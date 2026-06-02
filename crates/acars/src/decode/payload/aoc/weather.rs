//! AOC weather bundle parser for METAR-like ACARS text payloads.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WeatherBundle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reports: Vec<WeatherReport>,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WeatherReport {
    pub station: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub day: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    pub text: String,
}

pub fn parse_weather_bundle(text: &str) -> Option<WeatherBundle> {
    let raw = text.trim().to_string();
    if raw.is_empty() || !raw.contains("\nSA ") && !raw.starts_with("SA ") {
        return None;
    }

    let mut header_lines = Vec::new();
    let mut reports = Vec::new();
    let lines: Vec<&str> = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some((day, time, inline_report)) = find_sa_header(line) {
            i += 1;
            if i >= lines.len() {
                break;
            }
            let mut text_lines = Vec::new();
            if let Some(inline) = inline_report {
                text_lines.push(inline);
            }
            while i < lines.len() && find_sa_header(lines[i]).is_none() {
                text_lines.push(lines[i]);
                i += 1;
            }
            if let Some(first) = text_lines.first() {
                let station = first.split_whitespace().next()?.to_string();
                reports.push(WeatherReport {
                    station,
                    day: Some(day),
                    time: Some(time),
                    text: text_lines.join("\n"),
                });
            }
        } else {
            header_lines.push(line.to_string());
            i += 1;
        }
    }

    if reports.is_empty() {
        return None;
    }
    Some(WeatherBundle {
        header: (!header_lines.is_empty()).then(|| header_lines.join("\n")),
        reports,
        raw,
    })
}

fn find_sa_header(line: &str) -> Option<(String, String, Option<&str>)> {
    if let Some(found) = parse_sa_header(line) {
        return Some(found);
    }
    let idx = line.find("SA ")?;
    parse_sa_header(&line[idx..])
}

fn parse_sa_header(line: &str) -> Option<(String, String, Option<&str>)> {
    let rest = line.strip_prefix("SA ")?;
    let (day, time) = rest.split_once('/')?;
    let mut time_parts = time.splitn(2, char::is_whitespace);
    let time_token = time_parts.next().unwrap_or(time);
    let inline_report = time_parts.next().map(str::trim).filter(|s| !s.is_empty());
    if day.len() == 2
        && time_token.len() >= 5
        && day.chars().all(|c| c.is_ascii_digit())
        && time_token[..5]
            .chars()
            .all(|c| c.is_ascii_digit() || c == ':')
    {
        Some((day.to_string(), time_token[..5].to_string(), inline_report))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_weather_bundle() {
        let msg = parse_weather_bundle("QUGTWLSXA~1\r\nSA 30/15:34\r\nLFRS 301530Z AUTO 25009KT CAVOK\r\n  28/20 Q1018 NOSIG\r\nSA 30/15:08\r\nLEMD 301500Z 22012KT CAVOK 36/05 Q1018 NOSIG").unwrap();
        assert_eq!(msg.reports.len(), 2);
        assert_eq!(msg.reports[0].station, "LFRS");
        assert_eq!(msg.reports[1].station, "LEMD");
    }
}
