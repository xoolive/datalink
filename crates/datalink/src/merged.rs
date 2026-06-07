use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    #[serde(default)]
    pub output: OutputConfig,
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OutputConfig {
    /// JSONL output path. Use "-" or omit for stdout.
    #[serde(default)]
    pub jsonl: Option<String>,
    /// Include protocol/source-specific full decode under DecodedEvent.raw.
    #[serde(default)]
    pub raw: bool,
    /// Redis URL for pub/sub output.
    #[serde(default)]
    pub redis_url: Option<String>,
    /// Redis topic. Defaults to "datalink".
    #[serde(default)]
    pub redis_topic: Option<String>,
    /// Retry interval in seconds when Redis publish fails. Defaults to 5; 0 disables retry.
    #[serde(default)]
    pub redis_retry_interval: Option<u64>,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            jsonl: Some("-".to_string()),
            raw: false,
            redis_url: None,
            redis_topic: Some("datalink".to_string()),
            redis_retry_interval: Some(5),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceConfig {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Optional explicit source class; normally inferred from address fields.
    #[serde(default, rename = "type")]
    pub source_type: Option<SourceClass>,

    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub websocket: Option<String>,
    #[serde(default)]
    pub rtlsdr: Option<toml::Value>,
    #[serde(default)]
    pub airspy: Option<toml::Value>,
    #[serde(default)]
    pub hackrf: Option<toml::Value>,
    #[serde(default)]
    pub soapy: Option<toml::Value>,

    #[serde(default, alias = "center")]
    pub center_freq: Option<u32>,
    #[serde(default, alias = "rate")]
    pub sample_rate: Option<u32>,
    #[serde(default, alias = "iq_format")]
    pub format: Option<String>,
    #[serde(default)]
    pub gain: Option<f64>,
    #[serde(default)]
    pub lna_gain: Option<f64>,
    #[serde(default)]
    pub vga_gain: Option<f64>,
    #[serde(default)]
    pub mixer_gain: Option<f64>,
    #[serde(default)]
    pub bias_tee: Option<bool>,
    #[serde(default)]
    pub start_second: Option<f64>,
    #[serde(default)]
    pub max_seconds: Option<f64>,

    #[serde(default)]
    pub receivers: Vec<ReceiverConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SourceClass {
    Iq,
    Events,
    Frames,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReceiverConfig {
    pub bearer: Bearer,
    #[serde(default, alias = "channel")]
    pub channels: Option<Vec<u32>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Bearer {
    Vhf,
    Vdl2,
    Hfdl,
    /// Already-decoded events where an underlying bearer is intentionally unavailable.
    Decoded,
    /// Events where the underlying bearer may exist but cannot be inferred.
    Unknown,
}

impl Config {
    pub(crate) fn load(path: &str) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(expanduser(path))?;
        let cfg: Self = toml::from_str(&text)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.sources.is_empty(), "merged config has no [[sources]]");
        let mut ids = HashSet::new();
        for source in &self.sources {
            anyhow::ensure!(!source.id.trim().is_empty(), "source id must not be empty");
            anyhow::ensure!(
                ids.insert(source.id.as_str()),
                "duplicate source id: {}",
                source.id
            );
            source.validate_gain_fields()?;
            let class = source.inferred_class()?;
            match class {
                SourceClass::Iq => {
                    anyhow::ensure!(
                        !source.receivers.is_empty(),
                        "I/Q source {} must define at least one [[sources.receivers]]",
                        source.id
                    );
                    for receiver in &source.receivers {
                        anyhow::ensure!(
                            matches!(receiver.bearer, Bearer::Vhf | Bearer::Vdl2 | Bearer::Hfdl),
                            "I/Q source {} receiver bearer must be vhf, vdl2, or hfdl",
                            source.id
                        );
                        validate_channel_bandwidth(source, receiver)?;
                    }
                }
                SourceClass::Events => {
                    anyhow::ensure!(
                        source.receivers.is_empty(),
                        "event source {} must not contain [[sources.receivers]]",
                        source.id
                    );
                    anyhow::ensure!(
                        source.websocket.is_some(),
                        "event source {} currently requires websocket",
                        source.id
                    );
                }
                SourceClass::Frames => {
                    // Reserved for later. Accept shape but do not require RF metadata.
                }
            }
        }
        Ok(())
    }
}

impl SourceConfig {
    pub(crate) fn inferred_class(&self) -> anyhow::Result<SourceClass> {
        if let Some(class) = self.source_type {
            return Ok(class);
        }
        if self.websocket.is_some() {
            return Ok(SourceClass::Events);
        }
        if self.file.is_some()
            || self.rtlsdr.is_some()
            || self.airspy.is_some()
            || self.hackrf.is_some()
            || self.soapy.is_some()
        {
            return Ok(SourceClass::Iq);
        }
        anyhow::bail!(
            "source {} has no recognizable address field (file/websocket/rtlsdr/airspy/hackrf/soapy)",
            self.id
        )
    }

    pub(crate) fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }

    fn validate_gain_fields(&self) -> anyhow::Result<()> {
        if self.gain.is_some()
            && (self.lna_gain.is_some() || self.vga_gain.is_some() || self.mixer_gain.is_some())
        {
            anyhow::bail!(
                "source {} must not mix generic gain with element gains (lna_gain/mixer_gain/vga_gain)",
                self.id
            );
        }
        if self.bias_tee == Some(true)
            && !(self.rtlsdr.is_some()
                || self.airspy.is_some()
                || self.hackrf.is_some()
                || self.soapy.is_some())
        {
            anyhow::bail!("source {} uses bias_tee but is not an SDR source", self.id);
        }
        Ok(())
    }
}

fn validate_channel_bandwidth(
    source: &SourceConfig,
    receiver: &ReceiverConfig,
) -> anyhow::Result<()> {
    let (Some(center), Some(rate), Some(channels)) = (
        source.center_freq,
        source.sample_rate,
        receiver.channels.as_ref(),
    ) else {
        return Ok(());
    };
    let half = rate as i64 / 2;
    for &channel in channels {
        let delta = channel as i64 - center as i64;
        anyhow::ensure!(
            delta.abs() <= half,
            "receiver channel {} Hz for source {} is outside source bandwidth center={} sample_rate={}",
            channel,
            source.id,
            center,
            rate
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DecodedEvent {
    pub event: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
    pub bearer: Bearer,
    pub source: SourceMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver: Option<ReceiverMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aircraft: Option<Value>,
    pub message: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SourceMetadata {
    pub id: String,
    pub name: String,
    pub class: SourceClass,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ReceiverMetadata {
    pub bearer: Bearer,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_hz: Option<u32>,
}

impl SourceMetadata {
    pub(crate) fn from_config(source: &SourceConfig) -> anyhow::Result<Self> {
        Ok(Self {
            id: source.id.clone(),
            name: source.display_name().to_string(),
            class: source.inferred_class()?,
            format: source.format.clone(),
        })
    }
}

pub(crate) struct OutputSink {
    writer: Option<BufWriter<File>>,
    raw: bool,
    redis: Option<RedisPublisher>,
    redis_topic: String,
}

impl OutputSink {
    pub(crate) async fn new(config: &OutputConfig) -> anyhow::Result<Self> {
        let writer = match config.jsonl.as_deref().unwrap_or("-") {
            "-" => None,
            path => Some(BufWriter::new(File::create(expanduser(path))?)),
        };
        let redis = if let Some(url) = config.redis_url.as_deref() {
            Some(RedisPublisher::connect(url, config.redis_retry_interval.unwrap_or(5)).await?)
        } else {
            None
        };
        Ok(Self {
            writer,
            raw: config.raw,
            redis,
            redis_topic: config
                .redis_topic
                .clone()
                .unwrap_or_else(|| "datalink".to_string()),
        })
    }

    pub(crate) fn raw_enabled(&self) -> bool {
        self.raw
    }

    pub(crate) async fn emit(&mut self, mut event: DecodedEvent) -> anyhow::Result<()> {
        if !self.raw {
            event.raw = None;
        }
        let line = serde_json::to_string(&event)?;
        if let Some(writer) = self.writer.as_mut() {
            writeln!(writer, "{line}")?;
        } else {
            println!("{line}");
        }
        if let Some(redis) = self.redis.as_mut() {
            redis.publish(&self.redis_topic, &line).await;
        }
        Ok(())
    }

    pub(crate) fn flush(&mut self) -> anyhow::Result<()> {
        if let Some(writer) = self.writer.as_mut() {
            writer.flush()?;
        }
        Ok(())
    }
}

pub(crate) struct RedisPublisher {
    connection: redis::aio::MultiplexedConnection,
    retry_interval: Duration,
}

impl RedisPublisher {
    async fn connect(url: &str, retry_interval_secs: u64) -> anyhow::Result<Self> {
        let client = redis::Client::open(url)?;
        let connection = client.get_multiplexed_async_connection().await?;
        Ok(Self {
            connection,
            retry_interval: Duration::from_secs(retry_interval_secs),
        })
    }

    async fn publish(&mut self, topic: &str, payload: &str) {
        use redis::AsyncCommands;
        loop {
            let result: redis::RedisResult<()> = self.connection.publish(topic, payload).await;
            match result {
                Ok(()) => return,
                Err(err) if self.retry_interval.is_zero() => {
                    eprintln!("datalink: Redis publish to {topic} failed: {err}");
                    return;
                }
                Err(err) => {
                    eprintln!(
                        "datalink: Redis publish to {topic} failed: {err}; retrying in {}s",
                        self.retry_interval.as_secs()
                    );
                    tokio::time::sleep(self.retry_interval).await;
                }
            }
        }
    }
}

pub(crate) async fn run(config_path: Option<String>) -> anyhow::Result<()> {
    let path = config_path.ok_or_else(|| {
        anyhow::anyhow!(
            "missing --config; merged mode is configured with `datalink --config datalink.toml`"
        )
    })?;
    let config = Config::load(&path)?;
    let mut output = OutputSink::new(&config.output).await?;
    for source in &config.sources {
        match source.inferred_class()? {
            SourceClass::Iq => run_iq_source(source, &mut output).await?,
            SourceClass::Events => crate::airframes::run_config_source(source, &mut output).await?,
            SourceClass::Frames => {
                // Frame sources are reserved for later.
            }
        }
    }
    output.flush()?;
    Ok(())
}

async fn run_iq_source(source: &SourceConfig, output: &mut OutputSink) -> anyhow::Result<()> {
    let Some(file) = source.file.as_deref() else {
        // Live SDR fanout is implemented in a later step; config parsing and validation are already shared.
        return Ok(());
    };
    let source_meta = SourceMetadata::from_config(source)?;
    for receiver in &source.receivers {
        let values = match receiver.bearer {
            Bearer::Hfdl => crate::hfdl::decode_file_values(
                file,
                source.format.as_deref(),
                source.center_freq,
                source.sample_rate,
                receiver.channels.clone(),
                source.start_second.unwrap_or(0.0),
                source.max_seconds.unwrap_or(20.0),
            )?,
            Bearer::Vhf => {
                crate::vhf::decode_file_values(
                    file,
                    source.format.as_deref(),
                    source.center_freq,
                    source.sample_rate,
                    receiver.channels.clone(),
                    output.raw_enabled(),
                )
                .await?
            }
            Bearer::Vdl2 => {
                crate::vdl2::decode_file_values(
                    file,
                    source.format.as_deref(),
                    source.center_freq,
                    source.sample_rate,
                    receiver.channels.clone(),
                    output.raw_enabled(),
                )
                .await?
            }
            Bearer::Decoded | Bearer::Unknown => continue,
        };
        for value in values {
            let channel_hz = event_channel_hz(receiver.bearer, &value);
            let raw = output.raw_enabled().then(|| value.clone());
            output
                .emit(DecodedEvent {
                    event: "message",
                    timestamp: value.get("timestamp").and_then(|v| v.as_f64()),
                    bearer: receiver.bearer,
                    source: source_meta.clone(),
                    receiver: Some(ReceiverMetadata {
                        bearer: receiver.bearer,
                        channel_hz,
                    }),
                    aircraft: aircraft_summary(receiver.bearer, &value),
                    message: value,
                    raw,
                })
                .await?;
        }
    }
    Ok(())
}

fn event_channel_hz(bearer: Bearer, value: &Value) -> Option<u32> {
    match bearer {
        Bearer::Hfdl => value
            .get("channel_khz")
            .and_then(|v| v.as_f64())
            .map(|khz| (khz * 1000.0).round() as u32),
        Bearer::Vhf | Bearer::Vdl2 => value
            .get("metadata")
            .and_then(|m| m.get("channel_mhz"))
            .and_then(|v| v.as_f64())
            .map(|mhz| (mhz * 1_000_000.0).round() as u32),
        Bearer::Decoded | Bearer::Unknown => None,
    }
}

fn aircraft_summary(bearer: Bearer, value: &Value) -> Option<Value> {
    let mut obj = serde_json::Map::new();
    if bearer == Bearer::Hfdl {
        if let Some(id) = value.get("src_aircraft_id").and_then(|v| v.as_u64()) {
            obj.insert("aircraft_id".into(), Value::from(id));
        }
    }
    if let Some(icao) = value
        .get("icao24")
        .or_else(|| value.get("src").and_then(|v| v.get("icao24")))
        .and_then(|v| v.as_str())
    {
        obj.insert("icao24".into(), Value::from(icao));
    }
    if obj.is_empty() {
        None
    } else {
        Some(Value::Object(obj))
    }
}

pub(crate) fn expanduser(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hackrf_multi_receiver_config() {
        let cfg: Config = toml::from_str(
            r#"
            [output]
            jsonl = "-"
            raw = true
            redis_url = "redis://localhost:6379"

            [[sources]]
            id = "hackrf-vhf-wide"
            hackrf = { device = 0 }
            center_freq = 134000000
            sample_rate = 8000000
            lna_gain = 32
            vga_gain = 20

              [[sources.receivers]]
              bearer = "vhf"
              channels = [131525000, 131725000]

              [[sources.receivers]]
              bearer = "vdl2"
              channels = [136875000]
            "#,
        )
        .unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.sources[0].inferred_class().unwrap(), SourceClass::Iq);
        assert_eq!(cfg.sources[0].receivers.len(), 2);
    }

    #[test]
    fn rejects_event_source_with_receiver() {
        let cfg: Config = toml::from_str(
            r#"
            [[sources]]
            id = "airframes"
            websocket = "airframes://"
            format = "airframes.io"

              [[sources.receivers]]
              bearer = "vdl2"
            "#,
        )
        .unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_channel_outside_bandwidth() {
        let cfg: Config = toml::from_str(
            r#"
            [[sources]]
            id = "bad"
            file = "capture.cs16"
            center_freq = 134000000
            sample_rate = 1000000

            [[sources.receivers]]
            bearer = "vdl2"
            channels = [136875000]
            "#,
        )
        .unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_raw_under_receiver() {
        let err = toml::from_str::<Config>(
            r#"
            [[sources]]
            id = "bad"
            file = "capture.cs16"

            [[sources.receivers]]
            bearer = "vhf"
            raw = true
            "#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn parses_example_configs() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        for path in [
            "examples/merged/acars-sdruno-vhf.toml",
            "examples/merged/gqrx-vdl2-vhf.toml",
            "examples/merged/hfdl-sdruno.toml",
            "examples/merged/vdl2-rtlsdr.toml",
        ] {
            let path = root.join(path);
            Config::load(path.to_str().unwrap())
                .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        }
    }

    #[test]
    fn normalizes_airframes_payload_as_event_source() {
        let meta = SourceMetadata {
            id: "airframes".into(),
            name: "Airframes".into(),
            class: SourceClass::Events,
            format: Some("airframes.io".into()),
        };
        let payload = serde_json::json!({
            "label": "SA",
            "text": "hello",
            "from_hex": "A1B2C3",
            "source_type": "vdl2",
            "timestamp": 123.0
        });
        let event = crate::airframes::normalize_payload(meta, "message", payload, true).unwrap();
        assert_eq!(event.bearer, Bearer::Vdl2);
        assert!(event.raw.is_some());
    }
}
