//! ACARS label 37 obfuscated/encoded airline ops classifier.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Label37Message {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    pub line_count: usize,
    pub raw: String,
}

pub fn parse_label37(text: &str) -> Option<Label37Message> {
    let raw = text.trim().to_string();
    if raw.is_empty() {
        return None;
    }
    let mut lines = raw.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next().unwrap_or_default();
    let prefix = first
        .split_whitespace()
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let line_count = raw.lines().filter(|l| !l.trim().is_empty()).count();
    Some(Label37Message {
        prefix,
        line_count,
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_label37() {
        let msg = parse_label37("09BP-HJ\r\nA.-XKABVCKO/8:Y866KX/LHY").unwrap();
        assert_eq!(msg.prefix.as_deref(), Some("09BP-HJ"));
        assert_eq!(msg.line_count, 2);
    }
}
