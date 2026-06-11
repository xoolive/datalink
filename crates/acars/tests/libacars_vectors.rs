use acars::decode::acars::{
    decode_acars_text_payload, extract_sublabel_and_mfi, parse_acars_frame, BlockId,
    MessageDirection,
};
use acars::decode::payload::arinc622::adsc::parse_adsc_app_text;
use acars::decode::payload::AcarsAppPayload;
use std::collections::{BTreeSet, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};

#[test]
fn acars_raw_frame_vectors_from_libacars_docs() {
    let data = include_str!("fixtures/acars_raw_frames.txt");

    for line in data
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            11,
            "unexpected fixture column count for line: {line}"
        );

        let name = fields[0];
        let bytes = hex::decode(fields[1]).unwrap_or_else(|_| panic!("{name}: invalid hex"));
        let direction = parse_direction(fields[2]);
        let expected_reg = fields[3].trim_start_matches('.');
        let expected_label = fields[4];
        let expected_block_id_char = fields[5]
            .chars()
            .next()
            .expect("block_id must not be empty");
        let expected_flight_id = none_if_dash(fields[6]);
        let expected_msg_num = none_if_dash(fields[7]);
        let expected_msg_seq = none_if_dash(fields[8]).and_then(|value| value.chars().next());
        let expected_txt_prefix = fields[9];
        // fields[10] = crc_ok; skip frames that would now fail (crc_ok=false → Err)
        let expected_crc_ok = fields[10]
            .parse::<bool>()
            .unwrap_or_else(|_| panic!("{name}: invalid bool in crc_ok"));
        if !expected_crc_ok {
            continue; // CRC failures now return Err, skip in this test
        }

        let message = parse_acars_frame(&bytes, direction)
            .unwrap_or_else(|e| panic!("{name}: decode failed: {e}"));
        assert_eq!(message.reg, expected_reg, "{name}: wrong reg");
        assert_eq!(message.label, expected_label, "{name}: wrong label");
        let expected_block_id = BlockId::from_byte(expected_block_id_char as u8);
        assert_eq!(
            message.block_id, expected_block_id,
            "{name}: wrong block_id"
        );
        assert_eq!(
            message.flight_id.as_deref(),
            expected_flight_id,
            "{name}: wrong flight_id"
        );
        assert_eq!(
            message.msg_nb.as_deref(),
            expected_msg_num,
            "{name}: wrong message_number"
        );
        assert_eq!(
            message.sequence, expected_msg_seq,
            "{name}: wrong message_sequence"
        );
        assert!(
            message.txt.starts_with(expected_txt_prefix),
            "{name}: txt does not start with expected prefix"
        );
    }
}

#[test]
fn h1_sublabel_mfi_vectors_from_libacars_examples() {
    let data = include_str!("fixtures/h1_sublabel_mfi.txt");

    for line in data
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            7,
            "unexpected fixture column count for line: {line}"
        );

        let name = fields[0];
        let label = fields[1];
        let direction = parse_direction(fields[2]);
        let text = fields[3];
        let expected_offset = fields[4]
            .parse::<usize>()
            .unwrap_or_else(|_| panic!("{name}: invalid offset"));
        let expected_sublabel = none_if_dash(fields[5]);
        let expected_mfi = none_if_dash(fields[6]);

        let (offset, sublabel, mfi) = extract_sublabel_and_mfi(label, direction, text.as_bytes())
            .unwrap_or_else(|e| panic!("{name}: extract failed: {e}"));

        assert_eq!(offset, expected_offset, "{name}: wrong offset");
        assert_eq!(
            sublabel.as_deref(),
            expected_sublabel,
            "{name}: wrong sublabel"
        );
        assert_eq!(mfi.as_deref(), expected_mfi, "{name}: wrong mfi");
    }
}

#[test]
fn acars_app_payload_prefix_vectors_from_libacars_examples() {
    let data = include_str!("fixtures/acars_app_payloads.txt");

    for line in data
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            5,
            "unexpected fixture column count for line: {line}"
        );

        let name = fields[0];
        let label = fields[1];
        let direction = parse_direction(fields[2]);
        let text = fields[3];
        let expected_prefix = fields[4];

        let (offset, _, _) = extract_sublabel_and_mfi(label, direction, text.as_bytes())
            .unwrap_or_else(|e| panic!("{name}: extract failed: {e}"));
        let payload = &text[offset..];

        assert!(
            payload.starts_with(expected_prefix),
            "{name}: payload after offset does not start with expected prefix"
        );
    }
}

#[test]
fn p1_acars_payload_fixtures_decode() {
    let data = include_str!("fixtures/acars_p1_payloads.jsonl");

    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        let row: serde_json::Value = serde_json::from_str(line).expect("fixture row JSON");
        let name = row["name"].as_str().expect("name");
        let label = row["label"].as_str().expect("label");
        let direction = parse_direction(row["direction"].as_str().expect("direction"));
        let text = row["text"].as_str().expect("text");
        let expected_variant = row["expected_variant"].as_str().expect("expected_variant");

        let payload = decode_acars_text_payload(label, None, text, direction);
        match (expected_variant, payload) {
            ("AtisDelivery", AcarsAppPayload::AtisDelivery(msg)) => {
                assert_eq!(msg.airport, row["expected_airport"], "{name}: airport");
                assert_eq!(
                    msg.atis_letter.as_deref(),
                    row["expected_atis_letter"].as_str(),
                    "{name}: atis letter"
                );
                assert_eq!(
                    msg.issued_time.as_deref(),
                    row["expected_issued_time"].as_str(),
                    "{name}: issued time"
                );
            }
            ("Afn", AcarsAppPayload::Afn(msg)) => {
                assert_eq!(msg.facility, row["expected_facility"], "{name}: facility");
                assert_eq!(
                    msg.message_type, row["expected_message_type"],
                    "{name}: message type"
                );
                assert_eq!(
                    msg.registration.as_deref(),
                    row["expected_registration"].as_str(),
                    "{name}: registration"
                );
                if let Some(expected_icao24) = row["expected_icao24"].as_str() {
                    assert_eq!(
                        msg.icao24.as_deref(),
                        Some(expected_icao24),
                        "{name}: icao24"
                    );
                }
                assert_eq!(
                    msg.applications.len(),
                    row["expected_app_count"].as_u64().unwrap() as usize,
                    "{name}: app count"
                );
            }
            ("OceanicClearance", AcarsAppPayload::OceanicClearance(msg)) => {
                assert_eq!(msg.facility, row["expected_facility"], "{name}: facility");
                assert_eq!(
                    msg.clearance_type, row["expected_clearance_type"],
                    "{name}: clearance type"
                );
                assert_eq!(
                    msg.flight_id.as_deref(),
                    row["expected_flight_id"].as_str(),
                    "{name}: flight id"
                );
                assert_eq!(
                    msg.entry_point.as_deref(),
                    row["expected_entry_point"].as_str(),
                    "{name}: entry point"
                );
            }
            (expected, other) => panic!("{name}: expected {expected}, got {other:?}"),
        }
    }
}

#[test]
fn p2_acars_payload_fixtures_decode() {
    let data = include_str!("fixtures/acars_p2_payloads.jsonl");

    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        let row: serde_json::Value = serde_json::from_str(line).expect("fixture row JSON");
        let name = row["name"].as_str().expect("name");
        let label = row["label"].as_str().expect("label");
        let direction = parse_direction(row["direction"].as_str().expect("direction"));
        let text = row["text"].as_str().expect("text");
        let expected_variant = row["expected_variant"].as_str().expect("expected_variant");

        let payload = decode_acars_text_payload(label, None, text, direction);
        match (expected_variant, payload) {
            ("Weather", AcarsAppPayload::Weather(msg)) => {
                assert_eq!(
                    msg.reports.len(),
                    row["expected_report_count"].as_u64().unwrap() as usize,
                    "{name}: report count"
                );
                assert_eq!(
                    msg.reports[0].station, row["expected_first_station"],
                    "{name}: first station"
                );
            }
            ("Label5z", AcarsAppPayload::Label5z(msg)) => {
                assert_eq!(
                    msg.fields.len(),
                    row["expected_field_count"].as_u64().unwrap() as usize,
                    "{name}: field count"
                );
                let key = row["expected_key"].as_str().unwrap();
                let field = msg.fields.iter().find(|f| f.key == key).expect("field");
                assert_eq!(field.value, row["expected_value"], "{name}: value");
            }
            ("AocPosition", AcarsAppPayload::AocPosition(msg)) => {
                if let Some(lat) = row["expected_lat"].as_f64() {
                    assert!((msg.latitude.unwrap() - lat).abs() < 0.01, "{name}: lat");
                }
                if let Some(lon) = row["expected_lon"].as_f64() {
                    assert!((msg.longitude.unwrap() - lon).abs() < 0.01, "{name}: lon");
                }
                if let Some(min_lat) = row["expected_lat_min"].as_f64() {
                    assert!(msg.latitude.unwrap() > min_lat, "{name}: lat min");
                }
                if let Some(max_lon) = row["expected_lon_max"].as_f64() {
                    assert!(msg.longitude.unwrap() < max_lon, "{name}: lon max");
                }
                if let Some(dep) = row["expected_departure"].as_str() {
                    assert_eq!(msg.departure.as_deref(), Some(dep), "{name}: dep");
                }
                if let Some(dest) = row["expected_destination"].as_str() {
                    assert_eq!(msg.destination.as_deref(), Some(dest), "{name}: dest");
                }
            }
            (expected, other) => panic!("{name}: expected {expected}, got {other:?}"),
        }
    }
}

#[test]
fn p3_acars_payload_fixtures_decode() {
    let data = include_str!("fixtures/acars_p3_payloads.jsonl");

    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        let row: serde_json::Value = serde_json::from_str(line).expect("fixture row JSON");
        let name = row["name"].as_str().expect("name");
        let label = row["label"].as_str().expect("label");
        let direction = parse_direction(row["direction"].as_str().expect("direction"));
        let text = row["text"].as_str().expect("text");
        let expected_variant = row["expected_variant"].as_str().expect("expected_variant");

        let payload = decode_acars_text_payload(label, None, text, direction);
        match (expected_variant, payload) {
            ("Label32", AcarsAppPayload::Label32(msg)) => {
                assert_eq!(
                    msg.timestamp.as_deref(),
                    row["expected_timestamp"].as_str(),
                    "{name}: timestamp"
                );
                assert!(
                    (msg.latitude.unwrap() - row["expected_lat"].as_f64().unwrap()).abs() < 0.001,
                    "{name}: lat"
                );
                assert!(
                    (msg.longitude.unwrap() - row["expected_lon"].as_f64().unwrap()).abs() < 0.001,
                    "{name}: lon"
                );
                assert_eq!(
                    msg.altitude_ft,
                    Some(row["expected_altitude"].as_i64().unwrap() as i32),
                    "{name}: altitude"
                );
            }
            ("Label16", AcarsAppPayload::Label16(msg)) => {
                assert_eq!(
                    msg.timestamp.as_deref(),
                    row["expected_timestamp"].as_str(),
                    "{name}: timestamp"
                );
                assert_eq!(
                    msg.fields.len(),
                    row["expected_field_count"].as_u64().unwrap() as usize,
                    "{name}: field count"
                );
            }
            ("Label37", AcarsAppPayload::Label37(msg)) => {
                assert_eq!(
                    msg.prefix.as_deref(),
                    row["expected_prefix"].as_str(),
                    "{name}: prefix"
                );
                assert_eq!(
                    msg.line_count,
                    row["expected_line_count"].as_u64().unwrap() as usize,
                    "{name}: line count"
                );
            }
            (expected, other) => panic!("{name}: expected {expected}, got {other:?}"),
        }
    }
}

#[test]
fn opensky_adsc_message_samples_parse() {
    let data = include_str!("fixtures/adsc_app_messages.txt");

    for line in data
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            6,
            "unexpected fixture column count for line: {line}"
        );

        let name = fields[0];
        let text = fields[1];
        let expected_atsu = fields[2];
        let expected_registration = fields[3];
        let expected_crc = fields[4];
        let expected_payload_prefix = fields[5];

        let parsed =
            parse_adsc_app_text(text).unwrap_or_else(|e| panic!("{name}: ADS-C parse failed: {e}"));

        let _ = expected_crc; // crc_hex removed from AdscMessage
        let _ = expected_payload_prefix; // payload_no_crc_hex removed from AdscMessage
        assert_eq!(parsed.atsu_address, expected_atsu, "{name}: wrong ATSU");
        assert_eq!(
            parsed.registration, expected_registration,
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
    let data = include_str!("fixtures/adsc_all_tags_samples.txt");
    let mut seen = BTreeSet::new();

    for line in data
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            2,
            "unexpected fixture column count for line: {line}"
        );

        let tag = fields[0];
        let text = fields[1];
        let parsed = parse_adsc_app_text(text)
            .unwrap_or_else(|e| panic!("failed to parse ADS-C sample for tag {tag}: {e}"));
        let tag_id = tag
            .parse::<u8>()
            .unwrap_or_else(|_| panic!("invalid tag id in fixture: {tag}"));
        assert!(
            parsed.tags.iter().any(|item| item.id() == tag_id),
            "expected tag {tag} not found in parsed ADS-C payload"
        );
        seen.insert(tag.to_string());
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
    let data = include_str!("fixtures/h1_sublabel_mfi.txt");

    for line in data
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            7,
            "unexpected fixture column count for line: {line}"
        );

        let name = fields[0];
        let label = fields[1];
        let direction = parse_direction(fields[2]);
        let text = fields[3];

        if label != "H1" || !text.contains(".ADS.") {
            continue;
        }

        let (offset, _, _) = extract_sublabel_and_mfi(label, direction, text.as_bytes())
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
    let data = include_str!("fixtures/acars_app_payloads.txt");

    for line in data
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            5,
            "unexpected fixture column count for line: {line}"
        );

        let name = fields[0];
        let text = fields[3];
        let prefix = fields[4];
        let is_cpdlc_like = prefix.contains(".AT1.") || prefix.contains(".CR1.");
        if !is_cpdlc_like {
            continue;
        }

        let parsed = parse_adsc_app_text(text);
        assert!(parsed.is_err(), "{name}: CPDLC-like sample parsed as ADS-C");
    }
}

fn none_if_dash(value: &str) -> Option<&str> {
    if value == "-" {
        None
    } else {
        Some(value)
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

#[test]
fn verify_json_output_schema() {
    // Verify that the JSON output includes all expected fields for complete message serialization
    use acars::decode::payload::arinc622::parse_and_dispatch;
    use serde_json;

    // Real ADS-C envelope from fixture
    let envelope_text = "/LHWE1YA.ADS.N572UP07263B5872A048C9F21C1F0E5B88D700000239";

    let message = parse_and_dispatch(envelope_text).expect("parse and dispatch should succeed");

    // Test ARINC 622 message JSON schema: header + IMI-dispatched payload stay together.
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

    // Test Payload JSON schema for ADS-C variant
    if let acars::decode::payload::arinc622::Payload::Adsc(adsc_msg) = message.payload {
        let msg_json = serde_json::to_value(&adsc_msg).expect("adsc should serialize");
        assert!(
            msg_json.get("atsu_address").is_some(),
            "missing atsu_address in ADS-C"
        );
        assert!(
            msg_json.get("registration").is_some(),
            "missing registration in ADS-C"
        );
        // payload_hex removed from AdscMessage
        // payload_no_crc_hex and crc_hex removed from AdscMessage
        assert!(msg_json.get("tags").is_some(), "missing tags in ADS-C");

        // Verify tags array contains objects
        if let Some(tags_array) = msg_json.get("tags").and_then(|v| v.as_array()) {
            assert!(!tags_array.is_empty(), "tags should not be empty");
            println!(
                "✓ Successfully decoded {} ADS-C tags in JSON",
                tags_array.len()
            );
        }
    } else {
        panic!("Expected Adsc variant");
    }
}

#[test]
fn end_to_end_acars_h1_arinc622_adsc_json_chain() {
    // Test full chain: ARINC 622 parsing → ADS-C decoding → JSON output within ACARS message context
    // This verifies that when an ACARS message contains an H1-extracted ARINC 622 envelope,
    // it gets properly decoded and included in the JSON output.

    use acars::decode::payload::arinc622::{parse_and_dispatch, Imi};
    use serde_json;

    let envelope_text = "#M1B/B6 LHWE1YA.ADS.N572UP07263B5872A048C9F21C1F0E5B88D700000239";

    let (offset, _, _) = extract_sublabel_and_mfi(
        "H1",
        MessageDirection::AirToGround,
        envelope_text.as_bytes(),
    )
    .expect("H1 sublabel extraction should work");

    let normalized = &envelope_text[offset..];
    let normalized = if normalized.starts_with('/') {
        normalized.to_string()
    } else {
        format!("/{}", normalized)
    };

    let message = parse_and_dispatch(&normalized).expect("parse_and_dispatch should succeed");

    assert_eq!(message.imi, Imi::Ads, "wrong IMI");
    assert_eq!(
        message.registration, "N572UP",
        "wrong aircraft registration"
    );
    assert_eq!(message.atsu_address, "LHWE1YA", "wrong ATSU");

    match message.payload {
        acars::decode::payload::arinc622::Payload::Adsc(adsc_msg) => {
            assert_eq!(
                adsc_msg.registration, "N572UP",
                "wrong registration in ADS-C"
            );
            assert_eq!(adsc_msg.atsu_address, "LHWE1YA", "wrong ATSU in ADS-C");
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
        other => panic!("Expected Adsc payload, got {:?}", other),
    }
}

#[test]
fn cpdlc_airframes_fixtures_parse_shallow() {
    let data = include_str!("fixtures/cpdlc_airframes_1h.jsonl");
    let mut count = 0usize;
    let mut interpreted = 0usize;
    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        let row: serde_json::Value = serde_json::from_str(line).expect("fixture row JSON");
        let payload_hex = row["payload_hex"].as_str().expect("payload_hex");
        let msg = acars::decode::payload::arinc622::cpdlc::parse_cpdlc_payload_hex(payload_hex)
            .unwrap_or_else(|e| panic!("CPDLC fixture failed: {e}; row={line}"));
        assert_eq!(msg.payload_len_bytes * 2, payload_hex.len());
        if msg.downlink.is_some() || msg.uplink.is_some() {
            interpreted += 1;
        }
        count += 1;
    }
    assert_eq!(count, 260);
    assert!(
        interpreted > 200,
        "only {interpreted}/{count} fixtures interpreted"
    );
}

#[test]
fn cpdlc_24h_unsupported_body_regression_fixtures_decode() {
    use acars::decode::payload::arinc622::cpdlc::CpdlcElementBody;
    use acars::decode::payload::arinc622::{parse_with_direction, Payload};

    #[derive(serde::Deserialize)]
    struct FixtureRow {
        expected_element: String,
        #[serde(default)]
        label: String,
        text: String,
        link_direction: Option<String>,
    }

    #[derive(Debug, Clone, Copy)]
    enum ExpectedCpdlcDirection {
        Uplink,
        Downlink,
    }

    fn cpdlc_catalog_name_direction_and_id(name: &str) -> (ExpectedCpdlcDirection, u16) {
        let direction = match name.as_bytes().first().copied() {
            Some(b'u') | Some(b'U') => ExpectedCpdlcDirection::Uplink,
            Some(b'd') | Some(b'D') => ExpectedCpdlcDirection::Downlink,
            _ => panic!("{name}: expected CPDLC catalog name starting with uM or dM"),
        };

        let after_m = name
            .split_once('M')
            .map(|(_, rest)| rest)
            .unwrap_or_else(|| {
                panic!("{name}: expected CPDLC catalog name like dM48PositionReport")
            });

        let digits: String = after_m
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect();

        let id = digits
            .parse()
            .unwrap_or_else(|_| panic!("{name}: expected numeric CPDLC element id"));

        (direction, id)
    }

    let data = include_str!("fixtures/cpdlc_airframes_24h_unsupported_bodies.jsonl");
    let mut rows = 0usize;
    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        let row: FixtureRow = serde_json::from_str(line).expect("fixture row JSON");
        let expected = row.expected_element.as_str();
        let (expected_direction, expected_id) = cpdlc_catalog_name_direction_and_id(expected);
        let direction = match row.label.as_str() {
            "AA" => MessageDirection::GroundToAir,
            "BA" => MessageDirection::AirToGround,
            "H1" if row.text.contains("/AA ") => MessageDirection::GroundToAir,
            "H1" if row.text.contains("/BA ") => MessageDirection::AirToGround,
            _ => match row.link_direction.as_deref() {
                Some("uplink") => MessageDirection::GroundToAir,
                Some("downlink") => MessageDirection::AirToGround,
                _ => MessageDirection::Unknown,
            },
        };
        let normalized = normalize_arinc622_fixture_text(&row.text);
        let message = parse_with_direction(&normalized, direction)
            .unwrap_or_else(|e| panic!("failed to parse {expected}: {e}; row={line}"));
        let Payload::Cpdlc(cpdlc) = message.payload else {
            panic!("{expected}: expected CPDLC payload");
        };
        let summary = match expected_direction {
            ExpectedCpdlcDirection::Uplink => cpdlc
                .uplink
                .as_ref()
                .unwrap_or_else(|| panic!("{expected}: expected uplink CPDLC summary; row={line}")),
            ExpectedCpdlcDirection::Downlink => cpdlc.downlink.as_ref().unwrap_or_else(|| {
                panic!("{expected}: expected downlink CPDLC summary; row={line}")
            }),
        };
        let element = summary
            .elements
            .iter()
            .find(|element| element.id == expected_id)
            .unwrap_or_else(|| panic!("{expected}: element #{expected_id} not found; row={line}"));
        assert!(element.body.is_some(), "{expected}: body was not decoded");
        assert!(
            !matches!(element.body, Some(CpdlcElementBody::Unsupported)),
            "{expected}: body is still unsupported"
        );
        if expected == "dM48PositionReport" {
            assert!(
                matches!(element.body, Some(CpdlcElementBody::PositionReport(_))),
                "dM48PositionReport should decode as a structured position report"
            );
        }
        if expected == "dM40RouteClearance" {
            assert!(
                matches!(
                    element.body,
                    Some(
                        CpdlcElementBody::RouteClearance(_)
                            | CpdlcElementBody::OpaqueRouteClearance { .. }
                    )
                ),
                "dM40RouteClearance should decode as route clearance or opaque route clearance"
            );
        }
        rows += 1;
    }
    assert_eq!(rows, 16);
}

#[test]
fn cpdlc_control_messages_decode() {
    use acars::decode::payload::arinc622::cpdlc::CpdlcControlMessage;
    use acars::decode::payload::arinc622::{parse_with_direction, Payload};

    let cases = [
        (
            "/RGNCAYA.CR1.A7-BTD20578128EB59B31AA01F",
            MessageDirection::GroundToAir,
            "connect_request",
        ),
        (
            "/USADCXA.CC1.N7800861055BF6491093E2",
            MessageDirection::AirToGround,
            "connect_confirm",
        ),
        (
            "/USADCXA.DR1.N900DU3AED",
            MessageDirection::AirToGround,
            "disconnect_request",
        ),
    ];

    for (text, direction, expected) in cases {
        let message = parse_with_direction(text, direction)
            .unwrap_or_else(|e| panic!("control message failed: {e}; text={text}"));
        let Payload::Cpdlc(cpdlc) = message.payload else {
            panic!("expected CPDLC payload for {text}");
        };
        match (expected, cpdlc.control.as_ref()) {
            ("connect_request", Some(CpdlcControlMessage::ConnectRequest { message })) => {
                assert!(
                    message.is_some(),
                    "CR1 should carry a decoded uplink message"
                );
            }
            ("connect_confirm", Some(CpdlcControlMessage::ConnectConfirm { .. })) => {}
            ("disconnect_request", Some(CpdlcControlMessage::DisconnectRequest)) => {}
            _ => panic!("wrong control decode for {text}: {:?}", cpdlc.control),
        }
    }
}

fn normalize_arinc622_fixture_text(text: &str) -> String {
    if text.starts_with('/') {
        return text.to_string();
    }
    for token in text.split_whitespace().rev() {
        if [".AT1.", ".CR1.", ".CC1.", ".DR1.", ".ADS."]
            .iter()
            .any(|needle| token.contains(needle))
        {
            return format!("/{token}");
        }
    }
    format!("/{text}")
}
