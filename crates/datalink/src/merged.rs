//! Merged receiver configuration and output pipeline.
//!
//! Merged mode runs when `datalink` is started without a bearer subcommand. A
//! TOML configuration describes sources, receiver pipelines attached to I/Q
//! sources, event sources such as Airframes.io, and output sinks. This module
//! validates the configuration, fans samples into bearer decoders, wraps decoded
//! protocol bodies in a common [`DecodedEvent`] envelope, and writes JSONL or
//! Redis pub/sub output.

use crate::iq_pipeline::{collect_iq_frames, IqPipeline};
use crate::source::Source;
use crate::util::{bytes_to_hex, expanduser, RedisPublisher};
use acars::decode::acars::{parse_acars_frame, MessageDirection};
use acars::decode::avlc::parse_avlc_frame;
use acars::demod::resample::ResampleAdapter;
use acars::demod::vdl2::{Vdl2Channel, SYMBOL_RATE};
use acars::demod::vhf::VhfChannel;
use desperado::dsp::channelizer::Channelizer;
use desperado::Gain;
use futures_util::StreamExt;
use serde::Deserialize;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::SystemTime;

use crate::event::{
    Aircraft, Bearer, DecodedEvent, ProtocolMessage, ReceiverMetadata, SourceClass, SourceMetadata,
};

/// Top-level TOML configuration for merged receiver mode.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    #[serde(default)]
    pub output: OutputConfig,
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
}

/// Output sink configuration shared by standalone and merged frontends.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OutputConfig {
    /// JSONL output path. Use "-" or omit for stdout.
    #[serde(default)]
    pub jsonl: Option<String>,
    /// Redis URL for pub/sub output.
    #[serde(default)]
    pub redis_url: Option<String>,

    /// Retry interval in seconds when Redis publish fails. Defaults to 5; 0 disables retry.
    #[serde(default)]
    pub redis_retry_interval: Option<u64>,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            jsonl: Some("-".to_string()),
            redis_url: None,
            redis_retry_interval: Some(5),
        }
    }
}

/// One configured input source in merged receiver mode.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceConfig {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Optional explicit source class; normally inferred from address fields.
    #[serde(default)]
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
    pub gain: Option<Gain>,
    #[serde(default, alias = "rf_amp")]
    pub amp_enable: Option<bool>,
    #[serde(default)]
    pub rf_gain: Option<f64>,
    #[serde(default, alias = "if_gain")]
    pub lna_gain: Option<f64>,
    #[serde(default, alias = "bb_gain")]
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

/// Receiver pipeline attached to an I/Q source.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReceiverConfig {
    pub bearer: Bearer,
    #[serde(default, alias = "channel")]
    pub channels: Option<Vec<u32>>,
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

impl TryFrom<&SourceConfig> for SourceMetadata {
    type Error = anyhow::Error;

    fn try_from(source: &SourceConfig) -> Result<Self, Self::Error> {
        Ok(Self {
            id: source.id.clone(),
            name: source.display_name().to_string(),
            class: source.inferred_class()?,
            format: source.format.clone(),
        })
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
            && (self.rf_gain.is_some()
                || self.lna_gain.is_some()
                || self.vga_gain.is_some()
                || self.mixer_gain.is_some())
        {
            anyhow::bail!(
                "source {} must not mix generic gain with element gains (rf_gain/lna_gain/mixer_gain/vga_gain)",
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

/// Runtime output sink for JSONL and optional Redis publishing.
pub(crate) struct OutputSink {
    writer: Option<BufWriter<File>>,
    redis: Option<RedisPublisher>,
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
        Ok(Self { writer, redis })
    }

    pub(crate) async fn emit(&mut self, event: DecodedEvent) -> anyhow::Result<()> {
        let line = serde_json::to_string(&event)?;
        if let Some(writer) = self.writer.as_mut() {
            writeln!(writer, "{line}")?;
        } else {
            println!("{line}");
        }
        if let Some(redis) = self.redis.as_mut() {
            redis
                .publish(crate::util::redis_topic_for_record(&event.message), &line)
                .await;
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

/// Run merged receiver mode from a TOML configuration path.
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

fn source_url(source: &SourceConfig) -> anyhow::Result<String> {
    if let Some(file) = source.file.as_deref() {
        return Ok(expanduser(file).to_string_lossy().into_owned());
    }
    if let Some(value) = source.rtlsdr.as_ref() {
        return Ok(device_url("rtlsdr", value));
    }
    if let Some(value) = source.airspy.as_ref() {
        return Ok(device_url("airspy", value));
    }
    if let Some(value) = source.hackrf.as_ref() {
        return Ok(device_url("hackrf", value));
    }
    if let Some(value) = source.soapy.as_ref() {
        return Ok(device_url("soapy", value));
    }
    anyhow::bail!("source {} has no IQ source address", source.id)
}

fn device_url(scheme: &str, value: &toml::Value) -> String {
    match value {
        toml::Value::Integer(device) => format!("{scheme}://{device}"),
        toml::Value::String(s) => format!("{scheme}://{s}"),
        toml::Value::Table(table) => {
            if let Some(serial) = table.get("serial").and_then(toml::Value::as_str) {
                format!("{scheme}://serial={serial}")
            } else if let Some(device) = table.get("device").and_then(toml::Value::as_integer) {
                format!("{scheme}://{device}")
            } else if scheme == "soapy" {
                table
                    .get("args")
                    .and_then(toml::Value::as_str)
                    .map(|args| format!("soapy://{args}"))
                    .unwrap_or_else(|| "soapy://".to_string())
            } else {
                format!("{scheme}://0")
            }
        }
        _ => format!("{scheme}://0"),
    }
}

async fn run_iq_source(source: &SourceConfig, output: &mut OutputSink) -> anyhow::Result<()> {
    let source_url = source_url(source)?;
    let source_meta = SourceMetadata::try_from(source)?;

    let has_hfdl = source.receivers.iter().any(|r| r.bearer == Bearer::Hfdl);
    let is_wav = source
        .file
        .as_deref()
        .is_some_and(|file| file.to_ascii_lowercase().ends_with(".wav"));
    if !has_hfdl && !is_wav {
        return run_live_iq_source(source, &source_url, &source_meta, output).await;
    }

    for receiver in &source.receivers {
        let values: Vec<DecodedEvent> = match receiver.bearer {
            Bearer::Hfdl => {
                crate::hfdl::decode_file_values(
                    &source_url,
                    source.format.as_deref(),
                    source.center_freq,
                    source.sample_rate,
                    receiver.channels.clone(),
                    source.start_second.unwrap_or(0.0),
                    source.max_seconds.unwrap_or(20.0),
                    &source_meta,
                    receiver.bearer,
                )
                .await?
            }
            Bearer::Vhf => {
                let Some(file) = source.file.as_deref() else {
                    continue;
                };
                crate::vhf::decode_file_values(
                    file,
                    source.format.as_deref(),
                    source.center_freq,
                    source.sample_rate,
                    receiver.channels.clone(),
                    &source_meta,
                    receiver.bearer,
                )
                .await?
            }
            Bearer::Vdl2 => {
                let Some(file) = source.file.as_deref() else {
                    continue;
                };
                crate::vdl2::decode_file_values(
                    file,
                    source.format.as_deref(),
                    source.center_freq,
                    source.sample_rate,
                    receiver.channels.clone(),
                    &source_meta,
                    receiver.bearer,
                )
                .await?
            }
            Bearer::Decoded | Bearer::Unknown => continue,
        };
        for event in values {
            output.emit(event).await?;
        }
    }
    Ok(())
}

async fn run_live_iq_source(
    source: &SourceConfig,
    source_url: &str,
    source_meta: &SourceMetadata,
    output: &mut OutputSink,
) -> anyhow::Result<()> {
    let runtime_source = runtime_source(source, source_url)?;
    let center_freq = runtime_source
        .center_freq
        .ok_or_else(|| anyhow::anyhow!("live I/Q source {} must set center_freq", source.id))?;
    let raw_sample_rate = runtime_source.sample_rate();
    let mut receivers = Vec::new();

    for receiver in &source.receivers {
        match receiver.bearer {
            Bearer::Vhf => receivers.push(LiveReceiver::vhf(
                receiver.bearer,
                receiver.channels.clone().unwrap_or_else(default_vhf_channels),
                center_freq,
                raw_sample_rate,
            )),
            Bearer::Vdl2 => receivers.push(LiveReceiver::vdl2(
                receiver.bearer,
                receiver.channels.clone().unwrap_or_else(default_vdl2_channels),
                center_freq,
                raw_sample_rate,
            )),
            Bearer::Hfdl => anyhow::bail!(
                "live merged HFDL is not supported by this path yet; use the hfdl command or a file source"
            ),
            Bearer::Decoded | Bearer::Unknown => {}
        }
    }

    anyhow::ensure!(
        !receivers.is_empty(),
        "I/Q source {} has no live VHF/VDL2 receivers",
        source.id
    );

    let mut stream = crate::vdl2::open_source(&runtime_source).await?;
    let run_start = SystemTime::now();
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result?;
        for receiver in &mut receivers {
            for event in receiver.process_chunk(&chunk, run_start, source_meta)? {
                output.emit(event).await?;
            }
        }
    }
    Ok(())
}

enum LiveReceiver {
    Vhf {
        bearer: Bearer,
        channels: Vec<u32>,
        channelizer: Channelizer,
        demods: Vec<VhfChannel>,
        sample_indices: Vec<u64>,
        sample_rate: u32,
    },
    Vdl2 {
        bearer: Bearer,
        channels: Vec<u32>,
        channelizer: Channelizer,
        demods: Vec<Vdl2Channel>,
        sample_indices: Vec<u64>,
        sample_rate: u32,
    },
}

impl LiveReceiver {
    fn vhf(bearer: Bearer, channels: Vec<u32>, center_freq: u32, raw_sample_rate: u32) -> Self {
        let sample_rate = 125_000;
        let channelizer = Channelizer::from_absolute_frequencies(
            center_freq,
            raw_sample_rate,
            sample_rate,
            &channels,
        )
        .expect("valid VHF channelizer rates");
        let demods = channels
            .iter()
            .map(|_| VhfChannel::new(sample_rate as f32, 0.0))
            .collect();
        let sample_indices = vec![0; channels.len()];
        Self::Vhf {
            bearer,
            channels,
            channelizer,
            demods,
            sample_indices,
            sample_rate,
        }
    }

    fn vdl2(bearer: Bearer, channels: Vec<u32>, center_freq: u32, raw_sample_rate: u32) -> Self {
        let sample_rate = SYMBOL_RATE * 100;
        let channelizer = Channelizer::from_absolute_frequencies(
            center_freq,
            raw_sample_rate,
            sample_rate,
            &channels,
        )
        .expect("valid VDL2 channelizer rates");
        let demods = channels
            .iter()
            .map(|&ch| {
                let mut d = Vdl2Channel::new(sample_rate as f32, 0.0, ch as f32);
                d.set_sync_threshold(3.2);
                d
            })
            .collect();
        let sample_indices = vec![0; channels.len()];
        Self::Vdl2 {
            bearer,
            channels,
            channelizer,
            demods,
            sample_indices,
            sample_rate,
        }
    }

    fn process_chunk(
        &mut self,
        chunk: &[rustfft::num_complex::Complex<f32>],
        run_start: SystemTime,
        source_meta: &SourceMetadata,
    ) -> anyhow::Result<Vec<DecodedEvent>> {
        match self {
            Self::Vhf {
                bearer,
                channels,
                channelizer,
                demods,
                sample_indices,
                sample_rate,
            } => {
                let mut events = Vec::new();
                for channel_chunk in channelizer.process(chunk) {
                    let idx = channel_chunk.channel_index;
                    for sample in channel_chunk.samples {
                        let one = std::slice::from_mut(&mut demods[idx]);
                        let mut passthrough = ResampleAdapter::new(None);
                        let mut pipeline = IqPipeline::new(
                            &mut passthrough,
                            one,
                            &mut sample_indices[idx],
                            *sample_rate,
                            run_start,
                        );
                        let frames =
                            collect_iq_frames(&mut pipeline, sample.re, sample.im, |d, re, im| {
                                Ok(d.process_sample(re, im))
                            })?;
                        for (mut ctx, demod_frame) in frames {
                            ctx.channel_index = idx;
                            let Ok(message) =
                                parse_acars_frame(&demod_frame.bytes, MessageDirection::Unknown)
                            else {
                                continue;
                            };
                            let pmsg = ProtocolMessage::Acars(Box::new(message));
                            events.push(DecodedEvent {
                                event: "message".to_string(),
                                timestamp: Some(ctx.timestamp_unix),
                                bearer: *bearer,
                                source: source_meta.clone(),
                                receiver: Some(ReceiverMetadata {
                                    bearer: *bearer,
                                    channel_hz: Some(channels[idx]),
                                }),
                                aircraft: aircraft_summary(&pmsg),
                                kinematics: pmsg.kinematics(),
                                raw_frame_hex: Some(bytes_to_hex(&demod_frame.bytes)),
                                message: pmsg,
                            });
                        }
                    }
                }
                Ok(events)
            }
            Self::Vdl2 {
                bearer,
                channels,
                channelizer,
                demods,
                sample_indices,
                sample_rate,
            } => {
                let mut events = Vec::new();
                for channel_chunk in channelizer.process(chunk) {
                    let idx = channel_chunk.channel_index;
                    for sample in channel_chunk.samples {
                        let one = std::slice::from_mut(&mut demods[idx]);
                        let mut passthrough = ResampleAdapter::new(None);
                        let mut pipeline = IqPipeline::new(
                            &mut passthrough,
                            one,
                            &mut sample_indices[idx],
                            *sample_rate,
                            run_start,
                        );
                        let frames =
                            collect_iq_frames(&mut pipeline, sample.re, sample.im, |d, re, im| {
                                Ok(d.process_sample(re, im))
                            })?;
                        for (mut ctx, demod_frame) in frames {
                            ctx.channel_index = idx;
                            let Ok(avlc) = parse_avlc_frame(&demod_frame.bytes) else {
                                continue;
                            };
                            if !avlc.fcs_ok {
                                continue;
                            }
                            let pmsg = ProtocolMessage::Avlc(Box::new(avlc));
                            events.push(DecodedEvent {
                                event: "message".to_string(),
                                timestamp: Some(ctx.timestamp_unix),
                                bearer: *bearer,
                                source: source_meta.clone(),
                                receiver: Some(ReceiverMetadata {
                                    bearer: *bearer,
                                    channel_hz: Some(channels[idx]),
                                }),
                                aircraft: aircraft_summary(&pmsg),
                                kinematics: pmsg.kinematics(),
                                raw_frame_hex: Some(bytes_to_hex(&demod_frame.bytes)),
                                message: pmsg,
                            });
                        }
                    }
                }
                Ok(events)
            }
        }
    }
}

fn runtime_source(source: &SourceConfig, source_url: &str) -> anyhow::Result<Source> {
    let mut parsed: Source = source_url.parse()?;
    parsed.name = source.name.clone();
    parsed.center_freq = source.center_freq;
    parsed.sample_rate = source.sample_rate;
    parsed.format = source.format.clone();
    parsed.gain = source.gain.clone();
    parsed.bias_tee = source.bias_tee;
    parsed.amp_enable = source.amp_enable;
    parsed.rf_gain = source.rf_gain;
    parsed.lna_gain = source.lna_gain;
    parsed.mixer_gain = source.mixer_gain;
    parsed.vga_gain = source.vga_gain;
    parsed.channels = None;
    Ok(parsed)
}

fn default_vhf_channels() -> Vec<u32> {
    vec![
        129_125_000,
        129_525_000,
        130_025_000,
        130_425_000,
        131_125_000,
        131_525_000,
        131_725_000,
        131_825_000,
        136_900_000,
    ]
}

fn default_vdl2_channels() -> Vec<u32> {
    (0..17).map(|i| 136_600_000_u32 + i * 25_000).collect()
}

/// Extract a compact aircraft identity summary from any protocol message.
pub(crate) fn aircraft_summary(msg: &ProtocolMessage) -> Option<Aircraft> {
    match msg {
        ProtocolMessage::Avlc(frame) => {
            let icao24 = if frame.src.is_aircraft() {
                Some(format!("{:06x}", frame.src.icao24))
            } else if frame.dst.is_aircraft() {
                Some(format!("{:06x}", frame.dst.icao24))
            } else {
                None
            };
            let registration =
                if let Some(acars::decode::avlc::AvlcPayload::Acars(msg)) = &frame.payload {
                    Some(msg.reg.clone())
                } else {
                    None
                };
            if icao24.is_some() || registration.is_some() {
                Some(Aircraft {
                    icao24,
                    aircraft_id: None,
                    registration,
                })
            } else {
                None
            }
        }
        ProtocolMessage::Acars(msg) => Some(Aircraft {
            icao24: None,
            aircraft_id: None,
            registration: Some(msg.reg.clone()),
        }),
        ProtocolMessage::Hfdl(hf) => {
            if let acars::decode::hfdl::HfdlPdu::Mpdu(acars::decode::hfdl::Mpdu::Downlink(dl)) =
                &hf.pdu
            {
                Some(Aircraft {
                    icao24: None,
                    aircraft_id: Some(dl.src_aircraft_id as u64),
                    registration: None,
                })
            } else {
                None
            }
        }
        ProtocolMessage::Airframes(af) => {
            // TODO avoid allocating new strings
            let icao24 = crate::airframes::extract_airframes_aircraft(af);
            let registration = crate::airframes::extract_airframes_registration(&af.payload);
            let aircraft_id = af.payload.airframe_id.filter(|id| *id != 0);
            if icao24.is_some() || registration.is_some() || aircraft_id.is_some() {
                Some(Aircraft {
                    icao24,
                    aircraft_id,
                    registration,
                })
            } else {
                None
            }
        }
        ProtocolMessage::App(app) => match &**app {
            acars::decode::payload::AcarsAppPayload::Arinc622(msg) => Some(Aircraft {
                icao24: None,
                aircraft_id: None,
                registration: Some(msg.registration.clone()),
            }),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // use std::path::PathBuf;

    #[test]
    fn parses_hackrf_multi_receiver_config() {
        let cfg: Config = toml::from_str(
            r#"
            [output]
            jsonl = "-"
            redis_url = "redis://localhost:6379"

            [[sources]]
            id = "hackrf-vhf-wide"
            hackrf = { device = 0 }
            center_freq = 134000000
            sample_rate = 8000000
            rf_gain = 14
            if_gain = 32
            bb_gain = 20

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
        assert_eq!(cfg.sources[0].rf_gain, Some(14.0));
        assert_eq!(cfg.sources[0].lna_gain, Some(32.0));
        assert_eq!(cfg.sources[0].vga_gain, Some(20.0));
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

    // #[test]
    // fn parses_representative_configs() {
    //     for text in [
    //         r#"
    //         [output]
    //         jsonl = "-"
    //         raw = true

    //         [[sources]]
    //         id = "acars-sdruno-129535"
    //         file = "SDRuno_20200908_152020Z_129535kHz.wav"

    //         [[sources.receivers]]
    //         bearer = "vhf"
    //         "#,
    //         r#"
    //         [output]
    //         jsonl = "-"
    //         raw = true

    //         [[sources]]
    //         id = "gqrx-vdl2-136500"
    //         file = "gqrx_20260518_114025_136500000_1800000_fc.raw"
    //         format = "cf32"
    //         center_freq = 136500000
    //         sample_rate = 1800000

    //         [[sources.receivers]]
    //         bearer = "vdl2"
    //         channels = [136875000]
    //         "#,
    //         r#"
    //         [output]
    //         jsonl = "-"
    //         raw = true

    //         [[sources]]
    //         id = "hfdl-sdruno-11404"
    //         file = "SDRuno_20200904_143937Z_11404kHz.wav"
    //         start_second = 0.0
    //         max_seconds = 20.0

    //         [[sources.receivers]]
    //         bearer = "hfdl"
    //         channels = [11387000]
    //         "#,
    //         r#"
    //         [output]
    //         jsonl = "-"
    //         raw = true

    //         [[sources]]
    //         id = "hfdl-hackrf-10m"
    //         hackrf = { device = 0 }
    //         center_freq = 10000000
    //         sample_rate = 8000000
    //         start_second = 0.0
    //         max_seconds = 20.0

    //         [[sources.receivers]]
    //         bearer = "hfdl"
    //         channels = [10081000]
    //         "#,
    //     ] {
    //         let cfg: Config = toml::from_str(text).unwrap();
    //         cfg.validate().unwrap();
    //     }
    // }
}
