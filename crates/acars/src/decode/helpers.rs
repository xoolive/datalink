//! Serde helpers for AVLC-related byte/address serialisation.
//!
//! Used by `avlc.rs` (for frame fields) and `x25.rs` (for raw byte fields).

use serde::{Deserializer, Serializer};

/// Serialise `Vec<u8>` as a contiguous lowercase hex string (e.g. `"deadbeef"`).
pub fn serialize_bytes_hex<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
    let hex: String = v.iter().map(|b| format!("{:02x}", b)).collect();
    s.serialize_str(&hex)
}

/// Same as `serialize_bytes_hex`; provided as a named alias for use in enum
/// variant fields where `#[serde(serialize_with = "...")]` requires a distinct path.
pub fn serialize_bytes_hex_variant<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
    serialize_bytes_hex(v, s)
}

/// Serialise `Option<Vec<u8>>` — `None` → JSON `null`, `Some(b)` → hex string.
pub fn serialize_opt_bytes_hex<S: Serializer>(
    v: &Option<Vec<u8>>,
    s: S,
) -> Result<S::Ok, S::Error> {
    match v {
        Some(b) => serialize_bytes_hex(b, s),
        None => s.serialize_none(),
    }
}

/// Deserialise `Vec<u8>` from a hex string or byte array.
pub fn deserialize_bytes_hex<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    deserialize_opt_bytes_hex(d)?.ok_or_else(|| serde::de::Error::custom("expected hex bytes"))
}

/// Deserialise `Option<Vec<u8>>` from JSON `null`, a hex string, or a byte array.
pub fn deserialize_opt_bytes_hex<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Vec<u8>>, D::Error> {
    use serde::de::{self, SeqAccess, Visitor};

    struct OptBytesVisitor;

    impl<'de> Visitor<'de> for OptBytesVisitor {
        type Value = Option<Vec<u8>>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "null, a hex string, or a byte array")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Self::Value, D2::Error> {
            struct BytesVisitor;
            impl<'de> Visitor<'de> for BytesVisitor {
                type Value = Vec<u8>;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    write!(f, "a hex string or byte array")
                }
                fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                    parse_hex_bytes(v).map_err(|e| E::custom(e.to_string()))
                }
                fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                    let mut out = Vec::new();
                    while let Some(b) = seq.next_element::<u8>()? {
                        out.push(b);
                    }
                    Ok(out)
                }
            }
            d.deserialize_any(BytesVisitor).map(Some)
        }
    }

    d.deserialize_option(OptBytesVisitor)
}

fn parse_hex_bytes(s: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = s.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if cleaned.is_empty() {
        return Ok(Vec::new());
    }
    if !cleaned.len().is_multiple_of(2) {
        return Err("hex string must have an even number of digits".to_string());
    }
    let mut out = Vec::with_capacity(cleaned.len() / 2);
    for i in (0..cleaned.len()).step_by(2) {
        let byte = u8::from_str_radix(&cleaned[i..i + 2], 16)
            .map_err(|e| format!("invalid hex at offset {i}: {e}"))?;
        out.push(byte);
    }
    Ok(out)
}

/// Serialise a 24-bit AVLC address as a 6-digit lowercase hex string (e.g. `"2a3261"`).
pub fn serialize_addr_hex<S: Serializer>(v: &u32, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&format!("{:06x}", v))
}

/// Deserialise a 24-bit AVLC address from a hex string or integer.
pub fn deserialize_addr_hex<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    use serde::de::{self, Visitor};

    struct AddrVisitor;
    impl<'de> Visitor<'de> for AddrVisitor {
        type Value = u32;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "hex string or integer AVLC address")
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            u32::try_from(v).map_err(|_| E::custom("address out of range"))
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            if v < 0 {
                return Err(E::custom("address must be non-negative"));
            }
            u32::try_from(v as u64).map_err(|_| E::custom("address out of range"))
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            let trimmed = v.trim();
            let no_prefix = trimmed
                .strip_prefix("0x")
                .or_else(|| trimmed.strip_prefix("0X"))
                .unwrap_or(trimmed);
            u32::from_str_radix(no_prefix, 16)
                .map_err(|e| E::custom(format!("invalid hex address: {e}")))
        }
    }
    d.deserialize_any(AddrVisitor)
}
