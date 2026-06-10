use std::collections::BTreeMap;
use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader};

use acars::decode::acars::MessageDirection;
use acars::decode::payload::arinc622::{parse_with_direction, Payload};
use serde::Deserialize;

#[derive(Default)]
struct Counts(BTreeMap<String, usize>);

impl Counts {
    fn inc(&mut self, key: impl Into<String>) {
        *self.0.entry(key.into()).or_default() += 1;
    }
    fn print(&self, title: &str, n: usize) {
        eprintln!("\n{title}:");
        let mut rows: Vec<_> = self.0.iter().collect();
        rows.sort_by_key(|(_, v)| std::cmp::Reverse(**v));
        for (k, v) in rows.into_iter().take(n) {
            eprintln!("  {k}: {v}");
        }
    }
}

#[derive(Deserialize)]
struct AirframesRow {
    data: AirframesData,
}

#[derive(Deserialize)]
struct AirframesData {
    label: Option<String>,
    text: Option<String>,
    link_direction: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args()
        .nth(1)
        .expect("usage: airframes_arinc622_analyze <jsonl>");
    let reader = BufReader::new(File::open(&path)?);
    let mut rows = 0usize;
    let mut text_rows = 0usize;
    let mut labels = Counts::default();
    let mut imis = Counts::default();
    let mut ads_ok = 0usize;
    let mut ads_err = Counts::default();
    let mut cpdlc_ok = 0usize;
    let mut cpdlc_err = Counts::default();
    let mut other_arinc_ok = Counts::default();

    for line in reader.lines() {
        rows += 1;
        let row: AirframesRow = serde_json::from_str(&line?)?;
        let data = row.data;
        let label = data.label.as_deref().unwrap_or("<null>");
        labels.inc(label);
        let Some(text) = data.text.as_deref() else {
            continue;
        };
        text_rows += 1;
        let Some(imi) = find_imi(text) else {
            continue;
        };
        imis.inc(imi);
        let arinc_text = normalize_arinc622_text(text);
        let direction = infer_direction(label, text, data.link_direction.as_deref());
        match parse_with_direction(&arinc_text, direction) {
            Ok(msg) => match msg.payload {
                Payload::Adsc(_) => ads_ok += 1,
                Payload::Cpdlc(_) => cpdlc_ok += 1,
                Payload::Aoc { .. } => other_arinc_ok.inc("AOC"),
                Payload::Unknown { .. } => other_arinc_ok.inc(format!("unknown:{imi}")),
            },
            Err(err) if imi == "ADS" => ads_err.inc(err.to_string()),
            Err(err) if matches!(imi, "AT1" | "CR1" | "CC1" | "DR1") => {
                cpdlc_err.inc(format!("{imi}: {err}"));
            }
            Err(err) => other_arinc_ok.inc(format!("err:{imi}: {err}")),
        }
    }

    eprintln!("file: {path}");
    eprintln!("rows: {rows}");
    eprintln!("rows_with_text: {text_rows}");
    eprintln!("ads_ok: {ads_ok}");
    eprintln!("cpdlc_ok: {cpdlc_ok}");
    labels.print("labels", 30);
    imis.print("imis", 30);
    ads_err.print("ads_errors", 20);
    cpdlc_err.print("cpdlc_parse_errors", 20);
    other_arinc_ok.print("other_arinc", 20);
    Ok(())
}

fn find_imi(text: &str) -> Option<&str> {
    for imi in ["AT1", "CR1", "CC1", "DR1", "ADS", "DIS", "AB1"] {
        if text.contains(&format!(".{imi}.")) {
            return Some(imi);
        }
    }
    None
}

fn normalize_arinc622_text(text: &str) -> String {
    if text.starts_with('/') {
        return text.to_string();
    }
    for token in text.split_whitespace().rev() {
        if find_imi(token).is_some() {
            return format!("/{token}");
        }
    }
    format!("/{text}")
}

fn infer_direction(label: &str, text: &str, link_direction: Option<&str>) -> MessageDirection {
    if label == "H1" {
        if text.contains("/AA ") {
            return MessageDirection::GroundToAir;
        }
        if text.contains("/BA ") {
            return MessageDirection::AirToGround;
        }
    }
    match label {
        "AA" => MessageDirection::GroundToAir,
        "BA" => MessageDirection::AirToGround,
        _ => match link_direction {
            Some("uplink") => MessageDirection::GroundToAir,
            Some("downlink") => MessageDirection::AirToGround,
            _ => MessageDirection::Unknown,
        },
    }
}
