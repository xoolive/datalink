use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) fn expanduser(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

pub(crate) fn unix_timestamp_value(value: &Value) -> Option<f64> {
    if let Some(n) = value.as_f64() {
        return Some(n);
    }
    let s = value.as_str()?.trim();
    if let Ok(n) = s.parse::<f64>() {
        return Some(n);
    }
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_micros() as f64 / 1_000_000.0)
}

pub(crate) fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut s, "{b:02X}");
    }
    s
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CaptureParams {
    pub center_freq: u32,
    pub sample_rate: Option<u32>,
    pub format: Option<&'static str>,
}

pub(crate) fn infer_capture_params(path: &str) -> Option<CaptureParams> {
    infer_gqrx_capture_params(path).or_else(|| {
        infer_sdruno_center_freq(path).map(|center_freq| CaptureParams {
            center_freq,
            sample_rate: None,
            format: None,
        })
    })
}

fn infer_gqrx_capture_params(path: &str) -> Option<CaptureParams> {
    let stem = Path::new(path).file_stem()?.to_string_lossy();
    let parts: Vec<&str> = stem.split('_').collect();
    let fc_pos = parts.iter().rposition(|part| *part == "fc")?;
    if fc_pos < 2 {
        return None;
    }
    let center_freq = parts[fc_pos - 2].parse::<u32>().ok()?;
    let sample_rate = parts[fc_pos - 1].parse::<u32>().ok()?;
    Some(CaptureParams {
        center_freq,
        sample_rate: Some(sample_rate),
        format: Some("cf32"),
    })
}

pub(crate) fn infer_sdruno_center_freq(path: &str) -> Option<u32> {
    let name = Path::new(path).file_name()?.to_string_lossy();
    let lower = name.to_ascii_lowercase();
    let khz_pos = lower.rfind("khz")?;
    let prefix = &lower[..khz_pos];
    let digits_rev: String = prefix
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits_rev.is_empty() {
        return None;
    }
    let digits: String = digits_rev.chars().rev().collect();
    digits.parse::<u32>().ok().map(|khz| khz * 1000)
}

#[cfg(feature = "hackrf")]
pub(crate) fn hackrf_gain(src: &crate::source::Source) -> desperado::Gain {
    let mut elements = Vec::new();
    if let Some(value_db) = src.lna_gain {
        elements.push(desperado::GainElement {
            name: desperado::GainElementName::Lna,
            value_db,
        });
    }
    if let Some(value_db) = src.vga_gain {
        elements.push(desperado::GainElement {
            name: desperado::GainElementName::Vga,
            value_db,
        });
    }
    if elements.is_empty() {
        src.gain(30.0)
    } else {
        desperado::Gain::Elements(elements)
    }
}

#[cfg(feature = "airspy")]
pub(crate) fn parse_airspy_serial(value: &str) -> anyhow::Result<u64> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        return Ok(u64::from_str_radix(hex, 16)?);
    }
    value
        .parse::<u64>()
        .or_else(|_| u64::from_str_radix(value, 16))
        .map_err(Into::into)
}

pub(crate) fn redis_topic_for_record(record: &Value) -> &'static str {
    if value_contains_app(record, "cpdlc") {
        "datalink-cpdlc"
    } else if value_contains_app(record, "squitter")
        || record.get("label").and_then(Value::as_str) == Some("SQ")
        || value_contains_app(record, "sq")
    {
        "datalink-sq"
    } else if value_contains_app(record, "acars")
        || record.get("path").and_then(Value::as_str) == Some("acars")
    {
        "datalink-acars"
    } else {
        "datalink-other"
    }
}

fn value_contains_app(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(s) => s.to_ascii_lowercase().contains(needle),
        Value::Array(values) => values.iter().any(|v| value_contains_app(v, needle)),
        Value::Object(map) => map.iter().any(|(key, value)| {
            matches!(
                key.as_str(),
                "app"
                    | "protocol"
                    | "protocol_stack"
                    | "path"
                    | "label"
                    | "acars"
                    | "hfnpdu"
                    | "lpdus"
                    | "aircraft"
            ) && value_contains_app(value, needle)
        }),
        _ => false,
    }
}

pub(crate) struct RedisPublisher {
    connection: redis::aio::MultiplexedConnection,
    retry_interval: Duration,
    log_prefix: &'static str,
}

impl RedisPublisher {
    pub(crate) async fn connect(url: &str, retry_interval_secs: u64) -> anyhow::Result<Self> {
        Self::connect_with_prefix(url, retry_interval_secs, "datalink").await
    }

    pub(crate) async fn connect_with_prefix(
        url: &str,
        retry_interval_secs: u64,
        log_prefix: &'static str,
    ) -> anyhow::Result<Self> {
        let client = redis::Client::open(url)?;
        let connection = client.get_multiplexed_async_connection().await?;
        Ok(Self {
            connection,
            retry_interval: Duration::from_secs(retry_interval_secs),
            log_prefix,
        })
    }

    pub(crate) async fn publish(&mut self, topic: &str, payload: &str) {
        use redis::AsyncCommands;
        loop {
            let result: redis::RedisResult<()> = self.connection.publish(topic, payload).await;
            match result {
                Ok(()) => return,
                Err(err) if self.retry_interval.is_zero() => {
                    eprintln!(
                        "{}: Redis publish to {topic} failed: {err}",
                        self.log_prefix
                    );
                    return;
                }
                Err(err) => {
                    eprintln!(
                        "{}: Redis publish to {topic} failed: {err}; retrying in {}s",
                        self.log_prefix,
                        self.retry_interval.as_secs()
                    );
                    tokio::time::sleep(self.retry_interval).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_airframes_timestamp() {
        let value = Value::String("2026-05-22T08:37:19.050Z".into());
        assert_eq!(unix_timestamp_value(&value), Some(1779439039.05));
    }

    #[test]
    fn infers_gqrx_capture_params() {
        let params = infer_capture_params(
            "~/Documents/data/samples/decode_datalink/gqrx_20260518_114025_136500000_1800000_fc.raw",
        )
        .unwrap();
        assert_eq!(params.center_freq, 136_500_000);
        assert_eq!(params.sample_rate, Some(1_800_000));
        assert_eq!(params.format, Some("cf32"));
    }

    #[test]
    fn infers_sdruno_center_only() {
        let params = infer_capture_params("HFDL_10081kHz.wav").unwrap();
        assert_eq!(params.center_freq, 10_081_000);
        assert_eq!(params.sample_rate, None);
        assert_eq!(params.format, None);
    }
}
