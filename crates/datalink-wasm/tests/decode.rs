use datalink_wasm::{decode_acars, decode_arinc622};
use serde_json::Value;
use wasm_bindgen_test::*;

const ADSC_ISSUE_17: &str = "/MGQCAYA.ADS.A6-BLJ0707E9392157890809021F0E0B30E940040F0CD9A280046DD7";
const ADSC_CONTRACT_UPLINK: &str = "/UPGCAYA.ADS.B-324P07020BCD0D010E0110014BAA";
const ADSC_EVENT_UPLINK: &str = "/OAKODYA.ADS.N2645U0805140A28E574";
const ADSC_DISCONNECT: &str = "/NYCODYA.DIS.N861NW80FBFC";
const CPDLC_UPLINK: &str = "/AKLCDYA.AT1.9M-MTB215B659D84995674293583561CB9906744E9AF40F9EB";
const ACARS_BINARY: &str = "32aed3d0ad4cc4451532b3b302cdb0b9c14c4fb032c4cd4fceceb0314c4fb032c4cd2f2a2a32b932b03431454c4c5845d057c132b03431b0b0323883dfcb7f";

fn json(value: wasm_bindgen::JsValue) -> Value {
    serde_wasm_bindgen::from_value(value).expect("decoder output should be JSON-compatible")
}

#[wasm_bindgen_test]
fn decodes_adsc_earth_and_air_reference_data() {
    let decoded = json(decode_arinc622(ADSC_ISSUE_17, "downlink").expect("ADS-C should decode"));
    assert_eq!(decoded["imi"], "ADS");

    let tags = decoded["payload"]["adsc"]
        .as_array()
        .expect("ADS-C data array");
    let earth = tags
        .iter()
        .find_map(|tag| tag.get("earth_reference_data"))
        .expect("Tag 14");
    let air = tags
        .iter()
        .find_map(|tag| tag.get("air_reference_data"))
        .expect("Tag 15");

    assert_eq!(earth["ground_speed_kt"], 466.5);
    assert_eq!(earth["true_track_degrees"], 31.46484375);
    assert_eq!(air["mach"], 0.837);
    assert_eq!(air["true_heading_degrees"], 36.123046875);
}

#[wasm_bindgen_test]
fn structures_adsc_periodic_contract_request() {
    let decoded = json(
        decode_arinc622(ADSC_CONTRACT_UPLINK, "uplink")
            .expect("ADS-C contract request should decode"),
    );
    let request = &decoded["payload"]["adsc"][0]["periodic_contract_request"];
    assert_eq!(request["contract_number"], 2);
    assert_eq!(request["report_interval_secs"], 896);
    let groups = request["requested_groups"]
        .as_array()
        .expect("requested groups array");
    assert_eq!(groups[0]["predicted_route"]["modulus"], 1);
    assert_eq!(groups[1]["earth_reference_data"]["modulus"], 1);
    assert_eq!(groups[2]["meteo_data"]["modulus"], 1);
    assert!(groups.iter().all(|group| group.get("kind").is_none()));
}

#[wasm_bindgen_test]
fn structures_adsc_event_contract_request() {
    let decoded = json(
        decode_arinc622(ADSC_EVENT_UPLINK, "uplink").expect("ADS-C event request should decode"),
    );
    let request = &decoded["payload"]["adsc"][0]["event_contract_request"];
    assert_eq!(request["contract_number"], 5);
    let events = request["events"].as_array().expect("event trigger array");
    assert_eq!(events[0], "waypoint_change");
    assert_eq!(events[1]["lateral_deviation_change"]["threshold_nm"], 5.0);
}

#[wasm_bindgen_test]
fn decodes_adsc_disconnect_reason() {
    let decoded =
        json(decode_arinc622(ADSC_DISCONNECT, "downlink").expect("ADS-C DIS should decode"));
    assert_eq!(decoded["imi"], "DIS");
    assert_eq!(decoded["payload"]["adsc_disconnect"], "normal_disconnect");
}

#[wasm_bindgen_test]
fn dispatches_at1_to_cpdlc() {
    let decoded = json(decode_arinc622(CPDLC_UPLINK, "uplink").expect("CPDLC should decode"));
    assert_eq!(decoded["imi"], "AT1");
    let element = &decoded["payload"]["cpdlc"]["uplink"]["elements"][0];
    assert_eq!(element["fragments"][0]["text"], "AT ");
    assert_eq!(element["fragments"][1]["value"], "position");
    let body = &element["body"]["position_icao_unit_name_frequency"];
    assert_eq!(body["position"]["fix_name"], "LUNBI");
    assert_eq!(body["icao_unit_name"]["facility"]["name"], "AUCKLAND");
    assert_eq!(body["frequency"]["vhf_khz"], 123900);
    assert!(element["body"].get("kind").is_none());
}

#[wasm_bindgen_test]
fn decodes_binary_acars_frame() {
    let decoded = json(decode_acars(ACARS_BINARY, "downlink").expect("ACARS should decode"));
    assert_eq!(decoded["label"], "23");
    assert_eq!(decoded["flight_id"], "LO02DM");
    assert_eq!(decoded["msg_nb"], "M09");
    assert_eq!(
        decoded["app"]["text"],
        "ONN01LO02DM/**292041ELLXEPWA20410028"
    );
    assert!(decoded["app"].get("kind").is_none());
}
