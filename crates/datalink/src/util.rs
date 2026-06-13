//! Miscellaneous helpers shared by frontends and merged receiver mode.
//!
//! This module contains path expansion, hexadecimal formatting, capture-parameter
//! inference from common SDR filenames, hardware gain helpers, and Redis pub/sub
//! publishing for decoded application messages.

use crate::event::ProtocolMessage;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Expand a leading `~/` path segment to the current user's home directory.
pub(crate) fn expanduser(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Format bytes as uppercase hexadecimal for JSON output.
pub(crate) fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut s, "{b:02X}");
    }
    s
}

/// RF/I/Q parameters inferred from a capture filename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CaptureParams {
    /// Center frequency in Hz.
    pub center_freq: u32,
    /// Sample rate in samples per second, when present in the filename.
    pub sample_rate: Option<u32>,
    /// I/Q format hint, such as `cf32` for Gqrx float captures.
    pub format: Option<&'static str>,
}

/// Infer capture parameters from known filename conventions.
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
    let fc_pos = parts.iter().rposition(|&part| part == "fc")?;
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
        src.gain_or(desperado::Gain::Manual(30.0))
    } else {
        desperado::Gain::Elements(elements)
    }
}

#[cfg(feature = "airspy")]
pub(crate) fn parse_airspy_serial(value: &str) -> anyhow::Result<u64> {
    desperado::sdr::parse_airspy_serial(value).map_err(anyhow::Error::msg)
}

/// Select the Redis pub/sub topic for a decoded protocol message.
pub(crate) fn redis_topic_for_record(record: &ProtocolMessage) -> &'static str {
    match record {
        ProtocolMessage::Avlc(frame) => match &frame.payload {
            Some(acars::decode::avlc::AvlcPayload::Acars(acars)) => acars_redis_topic(&acars.app),
            Some(acars::decode::avlc::AvlcPayload::X25(x25)) => x25_redis_topic(x25),
            Some(acars::decode::avlc::AvlcPayload::Xid(_)) => "datalink-xid",
            Some(acars::decode::avlc::AvlcPayload::Unknown(_)) => "datalink-unknown",
            None => "datalink-vdl2",
        },
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

fn x25_redis_topic(x25: &acars::decode::x25::X25Packet) -> &'static str {
    use acars::decode::x25::{ClnpInner, X25Inner};

    let has_atn_cpdlc = matches!(
        x25.inner.as_ref(),
        Some(X25Inner::ClnpCompressed(clnp))
            if matches!(clnp.inner.as_ref(), Some(ClnpInner::Cotp(pdus)) if pdus.iter().any(|p| p.atn_cpdlc.is_some()))
    );

    if has_atn_cpdlc {
        "datalink-cpdlc"
    } else {
        "datalink-x25"
    }
}

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

/// Minimal Redis pub/sub publisher with optional retry-on-failure behaviour.
pub(crate) struct RedisPublisher {
    connection: redis::aio::MultiplexedConnection,
    retry_interval: Duration,
    log_prefix: &'static str,
}

impl RedisPublisher {
    /// Connect to Redis using the default log prefix.
    pub(crate) async fn connect(url: &str, retry_interval_secs: u64) -> anyhow::Result<Self> {
        Self::connect_with_prefix(url, retry_interval_secs, "datalink").await
    }

    /// Connect to Redis using a caller-specific log prefix.
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

    /// Publish one payload to a topic, retrying after reconnect when configured.
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
