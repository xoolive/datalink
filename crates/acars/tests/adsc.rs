use acars::decode::acars::{extract_sublabel_and_mfi, MessageDirection};
use acars::decode::payload::arinc622::adsc::{parse_adsc_app_text, AdscTag};
use acars::decode::payload::arinc622::{parse, Imi, Payload};
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

        let message = parse(&text).unwrap_or_else(|e| panic!("{name}: ADS-C parse failed: {e}"));

        assert_eq!(message.atsu_address, expect.atsu, "{name}: wrong ATSU");
        assert_eq!(
            message.registration, expect.registration,
            "{name}: wrong registration"
        );
        let Payload::Adsc(adsc) = message.payload else {
            panic!("{name}: expected ADS-C payload");
        };
        assert!(!adsc.tags.is_empty(), "{name}: expected non-empty tags");
    }
}

#[test]
fn earth_and_air_reference_decode_tag_specific_scales() {
    // Regression for issue #17. A single ADS-C message carries both Tag 14
    // (Earth Reference) and Tag 15 (Air Reference). The tags share a bit layout
    // but decode speed with different scales: ground speed in knots for Tag 14
    // and Mach for Tag 15. Previously Tag 15 reused the knot scale, turning
    // Mach 0.837 into 837.0.
    let text = "/MGQCAYA.ADS.A6-BLJ0707E9392157890809021F0E0B30E940040F0CD9A280046DD7";
    let parsed = parse_adsc_app_text(text).expect("issue #17 sample should parse");

    let earth = parsed
        .tags
        .iter()
        .find_map(|tag| match tag {
            AdscTag::EarthReferenceData(data) => Some(data),
            _ => None,
        })
        .expect("expected a Tag 14 Earth Reference group");
    assert!(!earth.track_invalid);
    assert!(approx_eq(earth.true_track_degrees.unwrap(), 31.46484375));
    assert!(approx_eq(earth.ground_speed_kt, 466.5));
    assert_eq!(earth.vertical_speed_ft_per_min, 16);

    let air = parsed
        .tags
        .iter()
        .find_map(|tag| match tag {
            AdscTag::AirReferenceData(data) => Some(data),
            _ => None,
        })
        .expect("expected a Tag 15 Air Reference group");
    assert!(!air.heading_invalid);
    assert!(approx_eq(air.true_heading_degrees.unwrap(), 36.123046875));
    assert!(approx_eq(air.mach, 0.837));
    assert_eq!(air.vertical_speed_ft_per_min, 16);
}

#[test]
fn adsc_serializes_with_external_payload_and_tag_keys() {
    let text = "/MGQCAYA.ADS.A6-BLJ0707E9392157890809021F0E0B30E940040F0CD9A280046DD7";
    let message = parse(text).expect("issue #17 sample should parse");
    let json = serde_json::to_value(message).expect("ARINC 622 should serialize");

    assert!(json["payload"].get("kind").is_none());
    let data = json["payload"]["adsc"]
        .as_array()
        .expect("ADS-C data array");
    assert!(data.iter().any(|tag| tag.get("basic_report").is_some()));
    assert!(data
        .iter()
        .any(|tag| tag.get("earth_reference_data").is_some()));
    assert!(data
        .iter()
        .any(|tag| tag.get("air_reference_data").is_some()));
}

fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
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

        let message = parse(&text).expect("parse and dispatch should succeed");
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
        let data = msg_json
            .as_array()
            .expect("ADS-C should serialize transparently as its data array");
        assert!(!data.is_empty(), "ADS-C data should not be empty");
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

        let message = parse(&normalized).expect("parse_and_dispatch should succeed");
        assert_eq!(message.imi, Imi::Ads, "wrong IMI");
        assert_eq!(
            message.registration, expect.registration,
            "wrong aircraft registration"
        );
        assert_eq!(message.atsu_address, expect.atsu, "wrong ATSU");

        let Payload::Adsc(adsc_msg) = message.payload else {
            panic!("Expected Adsc payload");
        };
        assert!(
            !adsc_msg.tags.is_empty(),
            "ADS-C message should have decoded tags"
        );
        let json = serde_json::to_value(&adsc_msg).expect("JSON serialization should work");
        assert!(
            json.as_array().is_some_and(|data| !data.is_empty()),
            "ADS-C should serialize as a non-empty data array"
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
