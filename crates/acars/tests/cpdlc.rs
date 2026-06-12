use acars::decode::acars::MessageDirection;
use acars::decode::payload::arinc622::cpdlc::{CpdlcControlMessage, CpdlcElementBody};
use acars::decode::payload::arinc622::{parse_with_direction, Payload};

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "kind")]
enum CpdlcFixture {
    #[serde(rename = "corpus")]
    Corpus { payload_hex: String },
    #[serde(rename = "body_regression")]
    BodyRegression {
        expected_element: String,
        #[serde(default)]
        label: String,
        text: String,
        link_direction: Option<String>,
    },
    #[serde(rename = "control")]
    Control {
        text: String,
        direction: String,
        expect: String,
    },
}

#[derive(Debug, Clone, Copy)]
enum ExpectedCpdlcDirection {
    Uplink,
    Downlink,
}

fn cpdlc_fixtures() -> impl Iterator<Item = CpdlcFixture> {
    include_str!("fixtures/cpdlc.jsonl")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("fixture row JSON"))
}

#[test]
fn cpdlc_airframes_fixtures_parse_shallow() {
    let mut count = 0usize;
    let mut interpreted = 0usize;

    for case in cpdlc_fixtures() {
        let CpdlcFixture::Corpus { payload_hex } = case else {
            continue;
        };

        let msg = acars::decode::payload::arinc622::cpdlc::parse_cpdlc_payload_hex(&payload_hex)
            .unwrap_or_else(|e| panic!("CPDLC fixture failed: {e}; payload_hex={payload_hex}"));
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
    let mut rows = 0usize;

    for case in cpdlc_fixtures() {
        let CpdlcFixture::BodyRegression {
            expected_element,
            label,
            text,
            link_direction,
        } = case
        else {
            continue;
        };

        let (expected_direction, expected_id) =
            cpdlc_catalog_name_direction_and_id(&expected_element);
        let direction = infer_direction(&label, &text, link_direction.as_deref());
        let normalized = normalize_arinc622_fixture_text(&text);
        let message = parse_with_direction(&normalized, direction)
            .unwrap_or_else(|e| panic!("failed to parse {expected_element}: {e}; text={text}"));
        let Payload::Cpdlc(cpdlc) = message.payload else {
            panic!("{expected_element}: expected CPDLC payload");
        };
        let summary = match expected_direction {
            ExpectedCpdlcDirection::Uplink => cpdlc.uplink.as_ref().unwrap_or_else(|| {
                panic!("{expected_element}: expected uplink CPDLC summary; text={text}")
            }),
            ExpectedCpdlcDirection::Downlink => cpdlc.downlink.as_ref().unwrap_or_else(|| {
                panic!("{expected_element}: expected downlink CPDLC summary; text={text}")
            }),
        };
        let element = summary
            .elements
            .iter()
            .find(|element| element.id == expected_id)
            .unwrap_or_else(|| {
                panic!("{expected_element}: element #{expected_id} not found; text={text}")
            });
        assert!(
            element.body.is_some(),
            "{expected_element}: body was not decoded"
        );
        assert!(
            !matches!(element.body, Some(CpdlcElementBody::Unsupported)),
            "{expected_element}: body is still unsupported"
        );
        if expected_element == "dM48PositionReport" {
            assert!(
                matches!(element.body, Some(CpdlcElementBody::PositionReport(_))),
                "dM48PositionReport should decode as a structured position report"
            );
        }
        if expected_element == "dM40RouteClearance" {
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
    for case in cpdlc_fixtures() {
        let CpdlcFixture::Control {
            text,
            direction,
            expect,
        } = case
        else {
            continue;
        };
        let direction = parse_direction(&direction);

        let message = parse_with_direction(&text, direction)
            .unwrap_or_else(|e| panic!("control message failed: {e}; text={text}"));
        let Payload::Cpdlc(cpdlc) = message.payload else {
            panic!("expected CPDLC payload for {text}");
        };
        match (expect.as_str(), cpdlc.control.as_ref()) {
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

fn cpdlc_catalog_name_direction_and_id(name: &str) -> (ExpectedCpdlcDirection, u16) {
    let direction = match name.as_bytes().first().copied() {
        Some(b'u') | Some(b'U') => ExpectedCpdlcDirection::Uplink,
        Some(b'd') | Some(b'D') => ExpectedCpdlcDirection::Downlink,
        _ => panic!("{name}: expected CPDLC catalog name starting with uM or dM"),
    };

    let after_m = name
        .split_once('M')
        .map(|(_, rest)| rest)
        .unwrap_or_else(|| panic!("{name}: expected CPDLC catalog name like dM48PositionReport"));

    let digits: String = after_m
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();

    let id = digits
        .parse()
        .unwrap_or_else(|_| panic!("{name}: expected numeric CPDLC element id"));

    (direction, id)
}

fn infer_direction(label: &str, text: &str, link_direction: Option<&str>) -> MessageDirection {
    match label {
        "AA" => MessageDirection::GroundToAir,
        "BA" => MessageDirection::AirToGround,
        "H1" if text.contains("/AA ") => MessageDirection::GroundToAir,
        "H1" if text.contains("/BA ") => MessageDirection::AirToGround,
        _ => match link_direction {
            Some("uplink") => MessageDirection::GroundToAir,
            Some("downlink") => MessageDirection::AirToGround,
            _ => MessageDirection::Unknown,
        },
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

fn parse_direction(value: &str) -> MessageDirection {
    match value {
        "uplink" => MessageDirection::GroundToAir,
        "downlink" => MessageDirection::AirToGround,
        "unknown" => MessageDirection::Unknown,
        other => panic!("unknown direction fixture value: {other}"),
    }
}
