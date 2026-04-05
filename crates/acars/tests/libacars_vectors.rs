use acars::decode::acars::{extract_sublabel_and_mfi, parse_acars_frame, MessageDirection};
use acars::decode::adsc::parse_adsc_app_text;
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
        let expected_reg = fields[3];
        let expected_label = fields[4];
        let expected_block_id = fields[5]
            .chars()
            .next()
            .expect("block_id must not be empty");
        let expected_flight_id = none_if_dash(fields[6]);
        let expected_msg_num = none_if_dash(fields[7]);
        let expected_msg_seq = none_if_dash(fields[8]).and_then(|value| value.chars().next());
        let expected_txt_prefix = fields[9];
        let expected_crc_ok = fields[10]
            .parse::<bool>()
            .unwrap_or_else(|_| panic!("{name}: invalid bool in crc_ok"));

        let message = parse_acars_frame(&bytes, direction)
            .unwrap_or_else(|e| panic!("{name}: decode failed: {e}"));
        assert_eq!(message.reg, expected_reg, "{name}: wrong reg");
        assert_eq!(message.label, expected_label, "{name}: wrong label");
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
            message.message_number.as_deref(),
            expected_msg_num,
            "{name}: wrong message_number"
        );
        assert_eq!(
            message.message_sequence, expected_msg_seq,
            "{name}: wrong message_sequence"
        );
        assert!(
            message.txt.starts_with(expected_txt_prefix),
            "{name}: txt does not start with expected prefix"
        );
        assert_eq!(message.crc_ok, expected_crc_ok, "{name}: wrong crc_ok");
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

        assert_eq!(parsed.atsu_address, expected_atsu, "{name}: wrong ATSU");
        assert_eq!(
            parsed.registration, expected_registration,
            "{name}: wrong registration"
        );
        assert_eq!(parsed.crc_hex, expected_crc, "{name}: wrong CRC");
        assert!(
            parsed
                .payload_no_crc_hex
                .starts_with(expected_payload_prefix),
            "{name}: unexpected payload prefix"
        );
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
