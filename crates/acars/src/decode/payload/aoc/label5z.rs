//! ACARS label 5Z AOC slash-field parser.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Label5zMessage {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<SlashField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remarks: Option<String>,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SlashField {
    pub key: String,
    pub value: String,
}

pub fn parse_label5z(text: &str) -> Option<Label5zMessage> {
    let raw = text.trim().to_string();
    if !raw.starts_with('/') {
        return None;
    }
    let first_line = raw.lines().next().unwrap_or_default();
    let mut fields = Vec::new();
    for chunk in first_line.split('/').filter(|s| !s.trim().is_empty()) {
        let chunk = chunk.trim();
        let key_len = chunk
            .char_indices()
            .find(|(_, c)| c.is_whitespace())
            .map(|(i, _)| i)
            .unwrap_or_else(|| chunk.len().min(2));
        let (key, value) = chunk.split_at(key_len);
        if !key.is_empty() {
            fields.push(SlashField {
                key: key.to_string(),
                value: value.trim().to_string(),
            });
        }
    }
    if fields.is_empty() {
        return None;
    }
    let remarks = raw
        .lines()
        .skip(1)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    Some(Label5zMessage {
        fields,
        remarks: (!remarks.is_empty()).then_some(remarks),
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_label5z() {
        let msg = parse_label5z("/IR GRR0100001/UM   /WC 02/IB   /ETA 1610\r\nPOTABLE WATER AFT.")
            .unwrap();
        assert_eq!(msg.fields[0].key, "IR");
        assert_eq!(
            msg.fields.iter().find(|f| f.key == "ETA").unwrap().value,
            "1610"
        );
        assert_eq!(msg.remarks.as_deref(), Some("POTABLE WATER AFT."));
    }
}
