use acars::decode::acars::{decode_acars_text_payload, extract_sublabel_and_mfi, MessageDirection};
use acars::decode::payload::AcarsAppPayload;

#[derive(Debug, serde::Deserialize)]
struct AcarsExtractionFixture {
    name: String,
    label: String,
    direction: String,
    text: String,
    expect: AcarsExtractionExpectation,
}

#[derive(Debug, serde::Deserialize)]
struct AcarsExtractionExpectation {
    offset: usize,
    #[serde(default)]
    sublabel: Option<String>,
    #[serde(default)]
    mfi: Option<String>,
    #[serde(default)]
    payload_prefix: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct AcarsPayloadFixture {
    name: String,
    label: String,
    direction: String,
    text: String,
    expect: AcarsPayloadExpectation,
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "variant")]
enum AcarsPayloadExpectation {
    AtisDelivery {
        airport: String,
        atis_letter: String,
        issued_time: String,
    },
    Afn {
        facility: String,
        message_type: String,
        registration: String,
        #[serde(default)]
        icao24: Option<String>,
        app_count: usize,
    },
    OceanicClearance {
        facility: String,
        clearance_type: String,
        flight_id: String,
        entry_point: String,
    },
    Weather {
        report_count: usize,
        first_station: String,
    },
    Label5z {
        field_count: usize,
        key: String,
        value: String,
    },
    AocPosition {
        #[serde(default)]
        lat: Option<f64>,
        #[serde(default)]
        lon: Option<f64>,
        #[serde(default)]
        lat_min: Option<f64>,
        #[serde(default)]
        lon_max: Option<f64>,
        #[serde(default)]
        departure: Option<String>,
        #[serde(default)]
        destination: Option<String>,
    },
    Label32 {
        timestamp: String,
        lat: f64,
        lon: f64,
        altitude: i32,
    },
    Label16 {
        timestamp: String,
        field_count: usize,
    },
    Label37 {
        prefix: String,
        line_count: usize,
    },
}

/// Verifies the ACARS extraction stage only.
///
/// These fixtures exercise `extract_sublabel_and_mfi`: given an ACARS label,
/// direction, and raw message text, the test checks where the application
/// payload begins and which H1 sublabel/MFI metadata was found. It deliberately
/// does not decode the resulting application payload.
#[test]
fn acars_extraction_offsets_and_metadata() {
    let data = include_str!("fixtures/acars_extraction.jsonl");

    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        let case: AcarsExtractionFixture = serde_json::from_str(line).expect("fixture row JSON");
        let direction = parse_direction(&case.direction);

        let (offset, sublabel, mfi) =
            extract_sublabel_and_mfi(&case.label, direction, case.text.as_bytes())
                .unwrap_or_else(|e| panic!("{}: extract failed: {e}", case.name));

        assert_eq!(offset, case.expect.offset, "{}: wrong offset", case.name);
        assert_eq!(
            sublabel.as_deref(),
            case.expect.sublabel.as_deref(),
            "{}: wrong sublabel",
            case.name
        );
        assert_eq!(
            mfi.as_deref(),
            case.expect.mfi.as_deref(),
            "{}: wrong mfi",
            case.name
        );
    }
}

/// Verifies that the extraction offset points at the expected application text.
///
/// This is still an extraction test, not a payload decoder test: it confirms
/// that H1 wrappers, non-H1 labels, and already-normalized application texts all
/// expose the expected payload prefix after applying the extracted offset.
#[test]
fn acars_extraction_payload_prefixes() {
    let data = include_str!("fixtures/acars_extraction.jsonl");

    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        let case: AcarsExtractionFixture = serde_json::from_str(line).expect("fixture row JSON");
        let Some(expected_prefix) = case.expect.payload_prefix.as_deref() else {
            continue;
        };
        let direction = parse_direction(&case.direction);

        let (offset, _, _) = extract_sublabel_and_mfi(&case.label, direction, case.text.as_bytes())
            .unwrap_or_else(|e| panic!("{}: extract failed: {e}", case.name));
        let payload = &case.text[offset..];

        assert!(
            payload.starts_with(expected_prefix),
            "{}: payload after offset does not start with expected prefix",
            case.name
        );
    }
}

/// Verifies the ACARS application payload decoding stage.
///
/// These fixtures start with payload text suitable for `decode_acars_text_payload`
/// and assert the dispatched application variant plus a small set of meaningful
/// decoded fields. They do not test H1 offset/sublabel extraction; that is covered
/// by the extraction tests above.
#[test]
fn acars_payload_fixtures_decode() {
    let data = include_str!("fixtures/acars_payloads.jsonl");

    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        let case: AcarsPayloadFixture = serde_json::from_str(line).expect("fixture row JSON");
        let direction = parse_direction(&case.direction);

        let payload = decode_acars_text_payload(&case.label, None, &case.text, direction);
        match (&case.expect, payload) {
            (
                AcarsPayloadExpectation::AtisDelivery {
                    airport,
                    atis_letter,
                    issued_time,
                },
                AcarsAppPayload::AtisDelivery(msg),
            ) => {
                assert_eq!(msg.airport, *airport, "{}: airport", case.name);
                assert_eq!(
                    msg.atis_letter.as_deref(),
                    Some(atis_letter.as_str()),
                    "{}: atis letter",
                    case.name
                );
                assert_eq!(
                    msg.issued_time.as_deref(),
                    Some(issued_time.as_str()),
                    "{}: issued time",
                    case.name
                );
            }
            (
                AcarsPayloadExpectation::Afn {
                    facility,
                    message_type,
                    registration,
                    icao24,
                    app_count,
                },
                AcarsAppPayload::Afn(msg),
            ) => {
                assert_eq!(msg.facility, *facility, "{}: facility", case.name);
                assert_eq!(
                    msg.message_type, *message_type,
                    "{}: message type",
                    case.name
                );
                assert_eq!(
                    msg.registration.as_deref(),
                    Some(registration.as_str()),
                    "{}: registration",
                    case.name
                );
                assert_eq!(
                    msg.icao24.as_deref(),
                    icao24.as_deref(),
                    "{}: icao24",
                    case.name
                );
                assert_eq!(
                    msg.applications.len(),
                    *app_count,
                    "{}: app count",
                    case.name
                );
            }
            (
                AcarsPayloadExpectation::OceanicClearance {
                    facility,
                    clearance_type,
                    flight_id,
                    entry_point,
                },
                AcarsAppPayload::OceanicClearance(msg),
            ) => {
                assert_eq!(msg.facility, *facility, "{}: facility", case.name);
                assert_eq!(
                    msg.clearance_type, *clearance_type,
                    "{}: clearance type",
                    case.name
                );
                assert_eq!(
                    msg.flight_id.as_deref(),
                    Some(flight_id.as_str()),
                    "{}: flight id",
                    case.name
                );
                assert_eq!(
                    msg.entry_point.as_deref(),
                    Some(entry_point.as_str()),
                    "{}: entry point",
                    case.name
                );
            }
            (
                AcarsPayloadExpectation::Weather {
                    report_count,
                    first_station,
                },
                AcarsAppPayload::Weather(msg),
            ) => {
                assert_eq!(
                    msg.reports.len(),
                    *report_count,
                    "{}: report count",
                    case.name
                );
                assert_eq!(
                    msg.reports[0].station, *first_station,
                    "{}: first station",
                    case.name
                );
            }
            (
                AcarsPayloadExpectation::Label5z {
                    field_count,
                    key,
                    value,
                },
                AcarsAppPayload::Label5z(msg),
            ) => {
                assert_eq!(msg.fields.len(), *field_count, "{}: field count", case.name);
                let field = msg.fields.iter().find(|f| f.key == *key).expect("field");
                assert_eq!(field.value, *value, "{}: value", case.name);
            }
            (
                AcarsPayloadExpectation::AocPosition {
                    lat,
                    lon,
                    lat_min,
                    lon_max,
                    departure,
                    destination,
                },
                AcarsAppPayload::AocPosition(msg),
            ) => {
                if let Some(lat) = lat {
                    assert!(
                        (msg.latitude.unwrap() - lat).abs() < 0.01,
                        "{}: lat",
                        case.name
                    );
                }
                if let Some(lon) = lon {
                    assert!(
                        (msg.longitude.unwrap() - lon).abs() < 0.01,
                        "{}: lon",
                        case.name
                    );
                }
                if let Some(min_lat) = lat_min {
                    assert!(msg.latitude.unwrap() > *min_lat, "{}: lat min", case.name);
                }
                if let Some(max_lon) = lon_max {
                    assert!(msg.longitude.unwrap() < *max_lon, "{}: lon max", case.name);
                }
                if let Some(dep) = departure {
                    assert_eq!(
                        msg.departure.as_deref(),
                        Some(dep.as_str()),
                        "{}: dep",
                        case.name
                    );
                }
                if let Some(dest) = destination {
                    assert_eq!(
                        msg.destination.as_deref(),
                        Some(dest.as_str()),
                        "{}: dest",
                        case.name
                    );
                }
            }
            (
                AcarsPayloadExpectation::Label32 {
                    timestamp,
                    lat,
                    lon,
                    altitude,
                },
                AcarsAppPayload::Label32(msg),
            ) => {
                assert_eq!(
                    msg.timestamp.as_deref(),
                    Some(timestamp.as_str()),
                    "{}: timestamp",
                    case.name
                );
                assert!(
                    (msg.latitude.unwrap() - lat).abs() < 0.001,
                    "{}: lat",
                    case.name
                );
                assert!(
                    (msg.longitude.unwrap() - lon).abs() < 0.001,
                    "{}: lon",
                    case.name
                );
                assert_eq!(msg.altitude_ft, Some(*altitude), "{}: altitude", case.name);
            }
            (
                AcarsPayloadExpectation::Label16 {
                    timestamp,
                    field_count,
                },
                AcarsAppPayload::Label16(msg),
            ) => {
                assert_eq!(
                    msg.timestamp.as_deref(),
                    Some(timestamp.as_str()),
                    "{}: timestamp",
                    case.name
                );
                assert_eq!(msg.fields.len(), *field_count, "{}: field count", case.name);
            }
            (
                AcarsPayloadExpectation::Label37 { prefix, line_count },
                AcarsAppPayload::Label37(msg),
            ) => {
                assert_eq!(
                    msg.prefix.as_deref(),
                    Some(prefix.as_str()),
                    "{}: prefix",
                    case.name
                );
                assert_eq!(msg.line_count, *line_count, "{}: line count", case.name);
            }
            (expected, other) => panic!("{}: expected {expected:?}, got {other:?}", case.name),
        }
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
