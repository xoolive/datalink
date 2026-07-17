#![allow(rustdoc::missing_crate_level_docs)]

mod utils;

use std::fmt::Display;

use acars::decode::acars::{parse_acars_frame, MessageDirection};
use acars::decode::payload::arinc622;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use utils::set_panic_hook;

const MAX_INPUT_LENGTH: usize = 64 * 1024;

/// Install a browser panic hook for useful console errors during development.
#[wasm_bindgen]
pub fn run() {
    set_panic_hook();
}

/// Decode an ARINC 622 application-text envelope.
///
/// The envelope IMI dispatches to ADS-C (`ADS`) or FANS-1/A CPDLC
/// (`AT1`, `CR1`, `CC1`, `DR1`). `direction` must be `uplink`, `downlink`,
/// or `unknown`.
#[wasm_bindgen]
pub fn decode_arinc622(text: &str, direction: &str) -> Result<JsValue, JsError> {
    set_panic_hook();
    let text = checked_input(text, "ARINC 622 text").map_err(js_error)?;
    let direction = parse_direction(direction).map_err(js_error)?;
    let message = arinc622::parse_with_direction(text, direction)
        .map_err(|error| contextual_error("ARINC 622 decode failed", error))?;
    to_js_value(&message)
}

/// Decode a hex-encoded binary ACARS frame and route its application payload.
///
/// `direction` must be `uplink`, `downlink`, or `unknown`.
#[wasm_bindgen]
pub fn decode_acars(frame_hex: &str, direction: &str) -> Result<JsValue, JsError> {
    set_panic_hook();
    let frame_hex = checked_input(frame_hex, "ACARS frame hex").map_err(js_error)?;
    let direction = parse_direction(direction).map_err(js_error)?;
    let bytes = hex::decode(frame_hex)
        .map_err(|error| contextual_error("invalid ACARS frame hex", error))?;
    let message = parse_acars_frame(&bytes, direction)
        .map_err(|error| contextual_error("ACARS decode failed", error))?;
    to_js_value(&message)
}

fn to_js_value<T: Serialize>(value: &T) -> Result<JsValue, JsError> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|error| contextual_error("failed to serialize decoder output", error))
}

fn checked_input<'a>(input: &'a str, label: &str) -> Result<&'a str, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if input.len() > MAX_INPUT_LENGTH {
        return Err(format!(
            "{label} exceeds the {MAX_INPUT_LENGTH}-byte input limit"
        ));
    }
    Ok(input)
}

fn parse_direction(value: &str) -> Result<MessageDirection, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "uplink" | "ground-to-air" | "ground_to_air" => Ok(MessageDirection::GroundToAir),
        "downlink" | "air-to-ground" | "air_to_ground" => Ok(MessageDirection::AirToGround),
        "unknown" => Ok(MessageDirection::Unknown),
        other => Err(format!(
            "invalid direction {other:?}; expected uplink, downlink, or unknown"
        )),
    }
}

fn js_error(message: String) -> JsError {
    JsError::new(&message)
}

fn contextual_error(context: &str, error: impl Display) -> JsError {
    JsError::new(&format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_book_direction_names() {
        assert!(matches!(
            parse_direction("uplink"),
            Ok(MessageDirection::GroundToAir)
        ));
        assert!(matches!(
            parse_direction("downlink"),
            Ok(MessageDirection::AirToGround)
        ));
        assert!(matches!(
            parse_direction("unknown"),
            Ok(MessageDirection::Unknown)
        ));
        assert!(parse_direction("sideways").is_err());
    }

    #[test]
    fn rejects_empty_and_oversized_input() {
        assert!(checked_input("   ", "input").is_err());
        assert!(checked_input("ok", "input").is_ok());
        assert!(checked_input(&"x".repeat(MAX_INPUT_LENGTH + 1), "input").is_err());
    }
}
