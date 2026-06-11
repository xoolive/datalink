use std::collections::BTreeMap;
use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader};

use acars::decode::acars::MessageDirection;
use acars::decode::payload::arinc622::cpdlc::{
    parse_cpdlc_payload_hex_with_direction, AtcMessageHeader, CpdlcControlMessage, CpdlcElement,
    CpdlcElementBody, CpdlcMessage, CpdlcPduSummary,
};
use acars::decode::payload::arinc622::{parse_with_direction, Payload as Arinc622Payload};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct FixtureRow {
    airframes_id: String,
    timestamp: String,
    imi: String,
    label: String,
    tail: String,
    payload_hex: String,
    text: String,
    #[serde(default)]
    link_direction: Option<String>,
}

#[derive(Default)]
struct Stats {
    rows: usize,
    parse_errors: usize,
    no_candidate: usize,
    candidates: usize,
    decoded_bodies: usize,
    unsupported_bodies: usize,
    remaining_bits_nonzero: usize,
    element_counts: BTreeMap<String, usize>,
    body_counts: BTreeMap<&'static str, usize>,
    unsupported_counts: BTreeMap<String, usize>,
    control_counts: BTreeMap<&'static str, usize>,
}

#[derive(Serialize)]
struct CandidateOutput<'a> {
    direction: &'a str,
    header: &'a AtcMessageHeader,
    body_kind: &'static str,
    elements: &'a [CpdlcElement],
    remaining_bits_after_element: usize,
}

#[derive(Serialize)]
struct OutputRow<'a> {
    line: usize,
    airframes_id: &'a str,
    timestamp: &'a str,
    tail: &'a str,
    imi: &'a str,
    label: &'a str,
    direction_hint: String,
    payload_hex: &'a str,
    control: &'a Option<CpdlcControlMessage>,
    candidates: Vec<CandidateOutput<'a>>,
}

#[derive(Serialize)]
struct ErrorRow<'a> {
    line: usize,
    airframes_id: &'a str,
    payload_hex: &'a str,
    error: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "crates/acars/tests/fixtures/cpdlc_airframes_1h.jsonl".to_string());
    let file = File::open(&path)?;
    let reader = BufReader::new(file);
    let mut stats = Stats::default();

    for (line_no, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        stats.rows += 1;
        let row: FixtureRow = serde_json::from_str(&line)?;
        let direction = infer_direction(&row);
        match decode_row_cpdlc(&row, direction) {
            Ok(message) => {
                let mut candidates = Vec::new();
                if let Some(summary) = &message.downlink {
                    candidates.push(candidate_output("downlink", summary, &mut stats));
                }
                if let Some(summary) = &message.uplink {
                    candidates.push(candidate_output("uplink", summary, &mut stats));
                }
                let control_kind = message.control.as_ref().map(|control| match control {
                    CpdlcControlMessage::ConnectRequest { .. } => "connect_request",
                    CpdlcControlMessage::ConnectConfirm { .. } => "connect_confirm",
                    CpdlcControlMessage::DisconnectRequest => "disconnect_request",
                });
                if let Some(kind) = control_kind {
                    *stats.control_counts.entry(kind).or_default() += 1;
                }
                if candidates.is_empty() && control_kind.is_none() {
                    stats.no_candidate += 1;
                }
                println!(
                    "{}",
                    serde_json::to_string(&OutputRow {
                        line: line_no + 1,
                        airframes_id: &row.airframes_id,
                        timestamp: &row.timestamp,
                        tail: &row.tail,
                        imi: &row.imi,
                        label: &row.label,
                        direction_hint: format!("{:?}", direction),
                        payload_hex: &row.payload_hex,
                        control: &message.control,
                        candidates,
                    })?
                );
            }
            Err(err) => {
                stats.parse_errors += 1;
                println!(
                    "{}",
                    serde_json::to_string(&ErrorRow {
                        line: line_no + 1,
                        airframes_id: &row.airframes_id,
                        payload_hex: &row.payload_hex,
                        error: err.to_string(),
                    })?
                );
            }
        }
    }

    eprintln!("\nCPDLC fixture decode summary for {path}");
    eprintln!("rows: {}", stats.rows);
    eprintln!("parse_errors: {}", stats.parse_errors);
    eprintln!("no_candidate: {}", stats.no_candidate);
    eprintln!("candidates: {}", stats.candidates);
    eprintln!("decoded_bodies: {}", stats.decoded_bodies);
    eprintln!("unsupported_bodies: {}", stats.unsupported_bodies);
    eprintln!("remaining_bits_nonzero: {}", stats.remaining_bits_nonzero);
    eprintln!("\nbody_counts:");
    for (body, count) in &stats.body_counts {
        eprintln!("  {body}: {count}");
    }
    eprintln!("\ncontrol_counts:");
    for (control, count) in &stats.control_counts {
        eprintln!("  {control}: {count}");
    }
    eprintln!("\nunsupported_elements:");
    for (element, count) in &stats.unsupported_counts {
        eprintln!("  {element}: {count}");
    }
    eprintln!("\nelement_counts:");
    for (element, count) in &stats.element_counts {
        eprintln!("  {element}: {count}");
    }

    Ok(())
}

fn decode_row_cpdlc(
    row: &FixtureRow,
    direction: MessageDirection,
) -> acars::decode::DecodeResult<CpdlcMessage> {
    let text = normalize_arinc622_text(&row.text);
    match parse_with_direction(&text, direction) {
        Ok(message) => match message.payload {
            Arinc622Payload::Cpdlc(cpdlc) => Ok(*cpdlc),
            _ => parse_cpdlc_payload_hex_with_direction(&row.payload_hex, direction),
        },
        Err(_) => parse_cpdlc_payload_hex_with_direction(&row.payload_hex, direction),
    }
}

fn normalize_arinc622_text(text: &str) -> String {
    if text.starts_with('/') {
        return text.to_string();
    }
    for token in text.split_whitespace().rev() {
        if token.contains(".AT1.")
            || token.contains(".CR1.")
            || token.contains(".CC1.")
            || token.contains(".DR1.")
            || token.contains(".ADS.")
        {
            return format!("/{token}");
        }
    }
    format!("/{text}")
}

fn infer_direction(row: &FixtureRow) -> MessageDirection {
    // The native ACARS path already passes MessageDirection from block id / AVLC.
    // These Airframes fixture rows are flattened, so use the ACARS CPDLC label
    // convention observed in the capture: AA is uplink, BA is downlink.
    if row.label == "H1" {
        if row.text.contains("/AA ") {
            return MessageDirection::GroundToAir;
        }
        if row.text.contains("/BA ") {
            return MessageDirection::AirToGround;
        }
    }
    match row.label.as_str() {
        "AA" => MessageDirection::GroundToAir,
        "BA" => MessageDirection::AirToGround,
        _ => match row.link_direction.as_deref() {
            Some("uplink") => MessageDirection::GroundToAir,
            Some("downlink") => MessageDirection::AirToGround,
            _ => MessageDirection::Unknown,
        },
    }
}

fn candidate_output<'a>(
    direction: &'a str,
    summary: &'a CpdlcPduSummary,
    stats: &mut Stats,
) -> CandidateOutput<'a> {
    stats.candidates += 1;
    if summary.remaining_bits_after_element != 0 {
        stats.remaining_bits_nonzero += 1;
    }
    let first_body_kind = summary
        .elements
        .first()
        .and_then(|element| element.body.as_ref())
        .map(body_kind)
        .unwrap_or("parse_failed");

    for element in &summary.elements {
        *stats
            .element_counts
            .entry(format!("{direction}:{}", element.id))
            .or_default() += 1;
        let element_body_kind = element
            .body
            .as_ref()
            .map(body_kind)
            .unwrap_or("parse_failed");
        *stats.body_counts.entry(element_body_kind).or_default() += 1;
        if matches!(element.body, Some(CpdlcElementBody::Unsupported)) {
            stats.unsupported_bodies += 1;
            *stats
                .unsupported_counts
                .entry(format!("{direction}:{}", element.id))
                .or_default() += 1;
        } else if element.body.is_some() {
            stats.decoded_bodies += 1;
        }
    }

    CandidateOutput {
        direction,
        header: &summary.header,
        body_kind: first_body_kind,
        elements: &summary.elements,
        remaining_bits_after_element: summary.remaining_bits_after_element,
    }
}

fn body_kind(body: &CpdlcElementBody) -> &'static str {
    match body {
        CpdlcElementBody::Null => "null",
        CpdlcElementBody::Altitude(_) => "altitude",
        CpdlcElementBody::AltitudeTime { .. } => "altitude_time",
        CpdlcElementBody::FreeText(_) => "free_text",
        CpdlcElementBody::IcaoFacilityDesignation(_) => "icao_facility_designation",
        CpdlcElementBody::IcaoFacilityDesignationTp4Table { .. } => {
            "icao_facility_designation_tp4_table"
        }
        CpdlcElementBody::IcaoUnitNameFrequency { .. } => "icao_unit_name_frequency",
        CpdlcElementBody::TimeIcaoUnitNameFrequency { .. } => "time_icao_unit_name_frequency",
        CpdlcElementBody::DistanceOffsetDirection { .. } => "distance_offset_direction",
        CpdlcElementBody::Position(_) => "position",
        CpdlcElementBody::PositionAltitude { .. } => "position_altitude",
        CpdlcElementBody::AltitudePosition { .. } => "altitude_position",
        CpdlcElementBody::AltitudeAltitude { .. } => "altitude_altitude",
        CpdlcElementBody::TimeAltitude { .. } => "time_altitude",
        CpdlcElementBody::PositionDistanceOffsetDirection { .. } => {
            "position_distance_offset_direction"
        }
        CpdlcElementBody::PositionIcaoUnitNameFrequency { .. } => {
            "position_icao_unit_name_frequency"
        }
        CpdlcElementBody::TimeDistanceOffsetDirection { .. } => "time_distance_offset_direction",
        CpdlcElementBody::Frequency(_) => "frequency",
        CpdlcElementBody::Time(_) => "time",
        CpdlcElementBody::DirectionDegrees { .. } => "direction_degrees",
        CpdlcElementBody::Degrees(_) => "degrees",
        CpdlcElementBody::AtisCode(_) => "atis_code",
        CpdlcElementBody::ProcedureName(_) => "procedure_name",
        CpdlcElementBody::Speed(_) => "speed",
        CpdlcElementBody::TimeSpeed { .. } => "time_speed",
        CpdlcElementBody::PositionSpeedSpeed { .. } => "position_speed_speed",
        CpdlcElementBody::PositionAltitudeSpeed { .. } => "position_altitude_speed",
        CpdlcElementBody::PositionTime { .. } => "position_time",
        CpdlcElementBody::PositionTimeTime { .. } => "position_time_time",
        CpdlcElementBody::TimePositionAltitude { .. } => "time_position_altitude",
        CpdlcElementBody::PositionTimeAltitude { .. } => "position_time_altitude",
        CpdlcElementBody::TimePositionAltitudeSpeed { .. } => "time_position_altitude_speed",
        CpdlcElementBody::AltitudeSpeedSpeed { .. } => "altitude_speed_speed",
        CpdlcElementBody::PositionPosition { .. } => "position_position",
        CpdlcElementBody::PositionReport(_) => "position_report",
        CpdlcElementBody::ErrorInformation(_) => "error_information",
        CpdlcElementBody::Altimeter(_) => "altimeter",
        CpdlcElementBody::RouteClearance(_) | CpdlcElementBody::OpaqueRouteClearance { .. } => {
            "route_clearance"
        }
        CpdlcElementBody::BeaconCode(_) => "beacon_code",
        CpdlcElementBody::VersionNumber(_) => "version_number",
        CpdlcElementBody::Unsupported => "unsupported",
    }
}
