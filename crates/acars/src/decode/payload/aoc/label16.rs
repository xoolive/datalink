//! ACARS label 16 heterogeneous telemetry classifier.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Label16Message {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    pub fields: Vec<String>,
    pub raw: String,
}

pub fn parse_label16(text: &str) -> Option<Label16Message> {
    let raw = text.trim().to_string();
    if raw.is_empty() || !raw.contains(',') {
        return None;
    }
    let fields: Vec<String> = raw.split(',').map(|s| s.trim().to_string()).collect();
    if fields.len() < 3 {
        return None;
    }
    let timestamp = fields
        .first()
        .filter(|s| s.len() == 6 && s.chars().all(|c| c.is_ascii_digit()))
        .cloned();
    Some(Label16Message {
        timestamp,
        fields,
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_label16() {
        let msg = parse_label16("153103,,1828, 225,N   .    MMMM.MMM").unwrap();
        assert_eq!(msg.timestamp.as_deref(), Some("153103"));
        assert_eq!(msg.fields.len(), 5);
    }
}
