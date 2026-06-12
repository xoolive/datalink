use acars::decode::acars::{extract_sublabel_and_mfi, MessageDirection};
use acars::decode::payload::arinc622::adsc::parse_adsc_app_text;
use acars::decode::payload::arinc622::{parse_and_dispatch, Imi, Payload};
use std::collections::{BTreeSet, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "kind")]
enum AdscFixture {
    #[serde(rename = "message")]
    Message {
        name: String,
        text: String,
        expect: AdscMessageExpectation,
    },
    #[serde(rename = "tag_coverage")]
    TagCoverage { tag: u8, text: String },
    #[serde(rename = "h1_wrapped")]
    H1Wrapped {
        name: String,
        label: String,
        direction: String,
        text: String,
    },
    #[serde(rename = "not_adsc")]
    NotAdsc { name: String, text: String },
    #[serde(rename = "json_schema")]
    JsonSchema { text: String },
    #[serde(rename = "h1_chain")]
    H1Chain {
        text: String,
        expect: AdscMessageExpectation,
    },
}

#[derive(Debug, serde::Deserialize)]
struct AdscMessageExpectation {
    atsu: String,
    registration: String,
}

fn adsc_fixtures() -> impl Iterator<Item = AdscFixture> {
    include_str!("fixtures/adsc.jsonl")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("fixture row JSON"))
}

#[test]
fn opensky_adsc_message_samples_parse() {
    for case in adsc_fixtures() {
        let AdscFixture::Message { name, text, expect } = case else {
            continue;
        };

        let parsed = parse_adsc_app_text(&text)
            .unwrap_or_else(|e| panic!("{name}: ADS-C parse failed: {e}"));

        assert_eq!(parsed.atsu_address, expect.atsu, "{name}: wrong ATSU");
        assert_eq!(
            parsed.registration, expect.registration,
            "{name}: wrong registration"
        );
        assert!(!parsed.tags.is_empty(), "{name}: expected non-empty tags");
    }
}

#[test]
fn decode_opensky_adsc_dataset_if_configured() {
    let Ok(path) = std::env::var("OPENSKY_ADSC_FILE") else {
        return;
    };

    let file = File::open(&path).expect("failed to open OPENSKY_ADSC_FILE");
    let reader = BufReader::new(file);

    let expected: HashSet<&str> = [
        "03", "04", "05", "06", "07", "09", "10", "12", "13", "14", "15", "16", "17", "18", "19",
        "20", "22", "23",
    ]
    .into_iter()
    .collect();

    let mut seen_tags: HashSet<String> = HashSet::new();
    let mut current_raw: Option<String> = None;
    let mut current_tags: HashSet<String> = HashSet::new();
    let mut checked = 0usize;
    let mut skipped = 0usize;

    let flush = |raw: &mut Option<String>,
                 tags: &mut HashSet<String>,
                 seen: &mut HashSet<String>,
                 checked: &mut usize,
                 skipped: &mut usize| {
        if let Some(raw_line) = raw.take() {
            if parse_adsc_app_text(&raw_line).is_ok() {
                *checked += 1;
            } else {
                *skipped += 1;
            }
            for tag in tags.drain() {
                seen.insert(tag);
            }
        }
    };

    for line in reader.lines().map_while(Result::ok) {
        let text = line.trim();

        if text.starts_with("Registration:") {
            flush(
                &mut current_raw,
                &mut current_tags,
                &mut seen_tags,
                &mut checked,
                &mut skipped,
            );
            continue;
        }

        if text.starts_with('/') && text.contains(".ADS.") {
            current_raw = Some(text.to_string());
            continue;
        }

        if let Some(tag) = text.strip_prefix("Tag ").and_then(|rest| rest.get(0..2)) {
            if tag.as_bytes().iter().all(|b| b.is_ascii_digit()) {
                current_tags.insert(tag.to_string());
            }
        }
    }

    flush(
        &mut current_raw,
        &mut current_tags,
        &mut seen_tags,
        &mut checked,
        &mut skipped,
    );

    assert!(checked > 0, "no ADS-C app lines were found in dataset");
    assert!(
        checked > skipped,
        "more ADS-C lines failed ({skipped}) than succeeded ({checked})"
    );
    for tag in expected {
        assert!(
            seen_tags.contains(tag),
            "missing ADS-C tag {tag} in dataset scan"
        );
    }
}

#[test]
fn opensky_adsc_samples_cover_all_tag_types() {
    let mut seen = BTreeSet::new();

    for case in adsc_fixtures() {
        let AdscFixture::TagCoverage { tag, text } = case else {
            continue;
        };

        let parsed = parse_adsc_app_text(&text)
            .unwrap_or_else(|e| panic!("failed to parse ADS-C sample for tag {tag:02}: {e}"));
        assert!(
            parsed.tags.iter().any(|item| item.id() == tag),
            "expected tag {tag:02} not found in parsed ADS-C payload"
        );
        seen.insert(format!("{tag:02}"));
    }

    let expected: BTreeSet<String> = [
        "03", "04", "05", "06", "07", "09", "10", "12", "13", "14", "15", "16", "17", "18", "19",
        "20", "22", "23",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();

    assert_eq!(seen, expected, "ADS-C tag coverage fixture mismatch");
}

#[test]
fn h1_wrapped_adsc_vectors_parse_after_normalization() {
    for case in adsc_fixtures() {
        let AdscFixture::H1Wrapped {
            name,
            label,
            direction,
            text,
        } = case
        else {
            continue;
        };
        let direction = parse_direction(&direction);

        let (offset, _, _) = extract_sublabel_and_mfi(&label, direction, text.as_bytes())
            .unwrap_or_else(|e| panic!("{name}: extract failed: {e}"));
        let payload = &text[offset..];
        let normalized = if payload.starts_with('/') {
            payload.to_string()
        } else {
            format!("/{payload}")
        };

        parse_adsc_app_text(&normalized)
            .unwrap_or_else(|e| panic!("{name}: normalized ADS-C parse failed: {e}"));
    }
}

#[test]
fn cpdlc_vectors_are_rejected_by_adsc_parser() {
    for case in adsc_fixtures() {
        let AdscFixture::NotAdsc { name, text } = case else {
            continue;
        };

        let parsed = parse_adsc_app_text(&text);
        assert!(parsed.is_err(), "{name}: CPDLC-like sample parsed as ADS-C");
    }
}

#[test]
fn verify_json_output_schema() {
    for case in adsc_fixtures() {
        let AdscFixture::JsonSchema { text } = case else {
            continue;
        };

        let message = parse_and_dispatch(&text).expect("parse and dispatch should succeed");
        let message_json = serde_json::to_value(&message).expect("message should serialize");
        assert!(
            message_json.get("atsu_address").is_some(),
            "missing atsu_address"
        );
        assert!(message_json.get("imi").is_some(), "missing imi");
        assert!(
            message_json.get("registration").is_some(),
            "missing registration"
        );
        assert!(message_json.get("payload").is_some(), "missing payload");

        let Payload::Adsc(adsc_msg) = message.payload else {
            panic!("Expected Adsc variant");
        };
        let msg_json = serde_json::to_value(&adsc_msg).expect("adsc should serialize");
        assert!(
            msg_json.get("atsu_address").is_some(),
            "missing atsu_address in ADS-C"
        );
        assert!(
            msg_json.get("registration").is_some(),
            "missing registration in ADS-C"
        );
        assert!(msg_json.get("tags").is_some(), "missing tags in ADS-C");
        if let Some(tags_array) = msg_json.get("tags").and_then(|v| v.as_array()) {
            assert!(!tags_array.is_empty(), "tags should not be empty");
        }
    }
}

#[test]
fn end_to_end_acars_h1_arinc622_adsc_json_chain() {
    for case in adsc_fixtures() {
        let AdscFixture::H1Chain { text, expect } = case else {
            continue;
        };

        let (offset, _, _) =
            extract_sublabel_and_mfi("H1", MessageDirection::AirToGround, text.as_bytes())
                .expect("H1 sublabel extraction should work");
        let normalized = &text[offset..];
        let normalized = if normalized.starts_with('/') {
            normalized.to_string()
        } else {
            format!("/{normalized}")
        };

        let message = parse_and_dispatch(&normalized).expect("parse_and_dispatch should succeed");
        assert_eq!(message.imi, Imi::Ads, "wrong IMI");
        assert_eq!(
            message.registration, expect.registration,
            "wrong aircraft registration"
        );
        assert_eq!(message.atsu_address, expect.atsu, "wrong ATSU");

        let Payload::Adsc(adsc_msg) = message.payload else {
            panic!("Expected Adsc payload");
        };
        assert_eq!(
            adsc_msg.registration, expect.registration,
            "wrong registration in ADS-C"
        );
        assert_eq!(adsc_msg.atsu_address, expect.atsu, "wrong ATSU in ADS-C");
        assert!(
            !adsc_msg.tags.is_empty(),
            "ADS-C message should have decoded tags"
        );
        let json = serde_json::to_value(&adsc_msg).expect("JSON serialization should work");
        assert!(json.get("tags").is_some(), "JSON should include tags");
        assert!(
            json.get("registration").is_some(),
            "JSON should include registration"
        );
    }
}

fn parse_direction(value: &str) -> MessageDirection {
    match value {
        "uplink" => MessageDirection::GroundToAir,
        "downlink" => MessageDirection::AirToGround,
        "unknown" => MessageDirection::Unknown,
        other => panic!("unknown direction fixture value: {other}"),
    }
}
