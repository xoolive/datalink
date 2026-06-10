use crate::merged::ProtocolMessage;
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

pub fn deserialize_timestamp<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct TsVisitor;

    impl<'de> serde::de::Visitor<'de> for TsVisitor {
        type Value = Option<f64>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a float or a timestamp string")
        }

        fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
            Ok(Some(value))
        }

        fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
            Ok(Some(value as f64))
        }

        fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
            if let Ok(n) = value.parse::<f64>() {
                Ok(Some(n))
            } else if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
                Ok(Some(dt.timestamp_micros() as f64 / 1_000_000.0))
            } else {
                Ok(None)
            }
        }

        fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D2: serde::Deserializer<'de>>(
            self,
            deserializer: D2,
        ) -> Result<Self::Value, D2::Error> {
            deserializer.deserialize_any(self)
        }
    }

    deserializer.deserialize_any(TsVisitor)
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

pub(crate) fn redis_topic_for_record(record: &ProtocolMessage) -> &'static str {
    match record {
        ProtocolMessage::Avlc(frame) => {
            if let Some(payload) = &frame.payload {
                match payload {
                    acars::decode::avlc::AvlcPayload::Acars(acars) => acars_redis_topic(&acars.app),
                    acars::decode::avlc::AvlcPayload::X25(_) => "datalink-x25",
                    acars::decode::avlc::AvlcPayload::Xid(_) => "datalink-xid",
                    acars::decode::avlc::AvlcPayload::Unknown(_) => "datalink-unknown",
                }
            } else {
                "datalink-vdl2"
            }
        }
        ProtocolMessage::Acars(msg) => acars_redis_topic(&msg.app),
        ProtocolMessage::Hfdl(_) => "datalink-hfdl",
        ProtocolMessage::Airframes(af) => {
            if let Some(app) = &af.app {
                acars_redis_topic(app)
            } else if af.payload.label.as_deref() == Some("SQ") {
                "datalink-sq"
            } else {
                "datalink-acars"
            }
        }
        ProtocolMessage::App(app) => acars_redis_topic(app),
    }
}

// TODO maybe use a trait instead?
fn acars_redis_topic(app: &acars::decode::payload::AcarsAppPayload) -> &'static str {
    match app {
        acars::decode::payload::AcarsAppPayload::Arinc622(arinc) => match arinc.imi {
            acars::decode::payload::arinc622::Imi::At1
            | acars::decode::payload::arinc622::Imi::Cr1
            | acars::decode::payload::arinc622::Imi::Cc1
            | acars::decode::payload::arinc622::Imi::Dr1 => "datalink-cpdlc",
            acars::decode::payload::arinc622::Imi::Ads => "datalink-adsc",
            _ => "datalink-acars",
        },
        acars::decode::payload::AcarsAppPayload::Squitter(_) => "datalink-sq",
        _ => "datalink-acars",
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
        use serde::Deserialize;
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(deserialize_with = "deserialize_timestamp")]
            ts: Option<f64>,
        }
        let json = r#"{"ts": "2026-05-22T08:37:19.050Z"}"#;
        let w: Wrapper = serde_json::from_str(json).unwrap();
        assert_eq!(w.ts, Some(1779439039.05));
    }

    #[test]
    fn infers_gqrx_capture_params() {
        let params = infer_capture_params("gqrx_20260518_114025_136500000_1800000_fc.raw").unwrap();
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
