//! Classic VHF ACARS frontend.
//!
//! This module implements the `datalink vhf` command: open a file or SDR source,
//! channelize classic ACARS frequencies, demodulate 2,400 bit/s MSK frames,
//! parse ACARS messages, and emit normalized JSONL or Redis messages.
//! Configuration can come from CLI options or merged receiver TOML.
//!
//! ## Default scan channels
//!
//! These channels are used when no explicit channel list is configured and the
//! source bandwidth metadata is too narrow or unavailable for automatic channel
//! selection.
//!
//! | Frequency | Support |
//! |---:|---|
//! | 131.525 MHz | Europe SITA secondary |
//! | 131.550 MHz | Global SITA primary |
//! | 131.725 MHz | Europe SITA |
//! | 131.825 MHz | Strongly observed in the Airframes.io feed |
//! | 131.850 MHz | New European channel |
//!
//! ## Known VHF ACARS channels
//!
//! The automatic channel selector chooses from this list when a source center
//! frequency and sample rate are known. The list is based on documented ACARS
//! channel tables plus frequencies with substantial counts in captured
//! Airframes.io data.
//!
//! | Frequency | Support |
//! |---:|---|
//! | 129.125 MHz | Global ARINC primary |
//! | 129.350 MHz | Strongly observed in the Airframes.io feed |
//! | 129.525 MHz | Strongly observed in the Airframes.io feed |
//! | 130.025 MHz | USA/Canada ARINC secondary |
//! | 130.425 MHz | USA ARINC additional |
//! | 130.450 MHz | USA/Canada ARINC additional |
//! | 130.825 MHz | Strongly observed in the Airframes.io feed |
//! | 131.075 MHz | Strongly observed in the Airframes.io feed |
//! | 131.125 MHz | USA ARINC additional |
//! | 131.200 MHz | Strongly observed in the Airframes.io feed |
//! | 131.450 MHz | Japan |
//! | 131.475 MHz | Air Canada company channel |
//! | 131.525 MHz | Europe SITA secondary |
//! | 131.550 MHz | Global SITA primary |
//! | 131.650 MHz | Strongly observed in the Airframes.io feed |
//! | 131.725 MHz | Europe SITA |
//! | 131.825 MHz | Strongly observed in the Airframes.io feed |
//! | 131.850 MHz | New European channel |

use crate::event::{
    Bearer, DecodedEvent, ProtocolMessage, ReceiverMetadata, SourceClass, SourceMetadata,
};
use crate::iq_pipeline::{collect_iq_frames, FrameContext, IqPipeline};
use crate::source::{Address, Source};
use crate::util::{bytes_to_hex, expanduser, infer_capture_params, RedisPublisher};
use acars::decode::acars::{parse_acars_frame, MessageDirection};
use acars::demod::resample::{maybe_resample, ResampleAdapter};
use acars::demod::vhf::VhfChannel;
use clap::Parser;
use desperado::dsp::channelizer::Channelizer;
use futures_util::StreamExt;
use serde::Deserialize;
use std::time::SystemTime;

const DEFAULT_CENTER_FREQ: u32 = 131_700_000;

/// Strongest channels around the default 131.7 MHz center
/// With the default 1.05 Msps source rate these all fit in-band.
const DEFAULT_CHANNELS: &[u32] = &[
    // Europe SITA secondary
    131_525_000,
    // Global SITA primary
    131_550_000,
    // Europe SITA
    131_725_000,
    // Strongly observed in airframes.io feed
    131_825_000,
    // New European channel.
    131_850_000,
];

/// Common VHF ACARS channels
const KNOWN_ACARS_CHANNELS: &[u32] = &[
    // Global ARINC primary
    129_125_000,
    // Strongly observed in airframes.io feed
    129_350_000,
    // Strongly observed in airframes.io feed
    129_525_000,
    // USA/Canada ARINC secondary
    130_025_000,
    // USA ARINC additional
    130_425_000,
    // USA/Canada ARINC additional
    130_450_000,
    // Strongly observed in airframes.io feed
    130_825_000,
    // Strongly observed in airframes.io feed
    131_075_000,
    // USA ARINC additional
    131_125_000,
    // Strongly observed in airframes.io feed
    131_200_000,
    // Japan
    131_450_000,
    // Air Canada company channel
    131_475_000,
    // Europe SITA secondary
    131_525_000,
    // Global SITA primary
    131_550_000,
    // Strongly observed in airframes.io feed
    131_650_000,
    // Europe SITA
    131_725_000,
    // Strongly observed in airframes.io feed
    131_825_000,
    // New European channel
    131_850_000,
];

const DEFAULT_CHUNK_SIZE: usize = 65_536;

fn auto_channels(src: &Source) -> Vec<u32> {
    auto_channels_for(src.center_freq_or(DEFAULT_CENTER_FREQ), src.sample_rate())
}

/// Internal VHF ACARS configuration after merging TOML, source URL, and CLI values.
#[derive(Debug, Default, Clone, Deserialize)]
pub(crate) struct Options {
    #[serde(default)]
    output: Option<String>,
    #[serde(default)]
    stats: bool,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    center_freq: Option<u32>,
    #[serde(default)]
    sample_rate: Option<u32>,
    #[serde(default, alias = "channels")]
    channel: Option<Vec<u32>>,
    #[serde(default)]
    redis_url: Option<String>,
    #[serde(default)]
    redis_retry_interval: Option<u64>,
    #[serde(default)]
    source: Option<Source>,
}

/// Command-line options for the standalone `datalink vhf` frontend.
#[derive(Debug, Default, Clone, Parser)]
#[command(about = "Classic VHF ACARS frontend")]
pub(crate) struct Cli {
    /// Dump a copy of decoded messages as JSONL
    #[arg(short, long)]
    output: Option<String>,
    /// Print demod/decode counters to stderr at end
    #[arg(long)]
    stats: bool,
    /// I/Q sample format for file input: cu8, cs8, cs16, cf32
    #[arg(long)]
    format: Option<String>,
    /// Center frequency for file and SDR sources
    #[arg(long)]
    center_freq: Option<u32>,
    /// Sample rate for file and SDR sources
    #[arg(long)]
    sample_rate: Option<u32>,
    /// ACARS channel frequencies in Hz
    #[arg(long, num_args = 1..)]
    channel: Option<Vec<u32>>,
    /// Publish decoded messages to application-specific Redis pub/sub topics
    #[arg(long, value_name = "REDIS URL")]
    redis_url: Option<String>,
    /// Retry interval (seconds) when publishing to Redis fails; 0 disables retry
    #[arg(long)]
    redis_retry_interval: Option<u64>,
    /// Source URL: file://, rtlsdr://, airspy://, hackrf://, soapy://
    source: Option<Source>,
}

impl Options {
    fn apply_cli_overrides(&mut self, cli: Cli) {
        if cli.output.is_some() {
            self.output = cli.output;
        }
        if cli.stats {
            self.stats = true;
        }
        if cli.format.is_some() {
            self.format = cli.format;
        }
        if cli.center_freq.is_some() {
            self.center_freq = cli.center_freq;
        }
        if cli.sample_rate.is_some() {
            self.sample_rate = cli.sample_rate;
        }
        if cli.channel.is_some() {
            self.channel = cli.channel;
        }
        if cli.redis_url.is_some() {
            self.redis_url = cli.redis_url;
        }
        if cli.redis_retry_interval.is_some() {
            self.redis_retry_interval = cli.redis_retry_interval;
        }
        if cli.source.is_some() {
            self.source = cli.source;
        }
        apply_source_overrides(self);
    }
}

#[derive(Default)]
struct DecodeStats {
    demod_frames: u64,
    parsed_ok: u64,
    parse_fail: u64,
}

/// Run the standalone classic VHF ACARS frontend from CLI arguments.
pub(crate) async fn run(cli: Cli) -> anyhow::Result<()> {
    let mut options = Options::default();
    options.apply_cli_overrides(cli);

    anyhow::ensure!(
        options.source.is_some(),
        "missing source; pass an explicit source such as file://capture.cu8, -, or rtlsdr://"
    );
    let mut output = if let Some(path) = options.output.as_deref() {
        Some(std::io::BufWriter::new(std::fs::File::create(expanduser(
            path,
        ))?))
    } else {
        None
    };

    let mut redis = if let Some(url) = options.redis_url.as_deref() {
        Some(
            RedisPublisher::connect_with_prefix(
                url,
                options.redis_retry_interval.unwrap_or(5),
                "datalink vhf",
            )
            .await?,
        )
    } else {
        None
    };

    let mut total = DecodeStats::default();
    let src = options.source.as_ref().expect("source checked before run");
    let source_meta = SourceMetadata {
        id: "vhf_cli".into(),
        name: src.label(),
        class: SourceClass::Iq,
        format: None,
    };
    let stats = decode_source(
        src,
        output.as_mut(),
        redis.as_mut(),
        None,
        &source_meta,
        Bearer::Vhf,
    )
    .await?;
    total.demod_frames += stats.demod_frames;
    total.parsed_ok += stats.parsed_ok;
    total.parse_fail += stats.parse_fail;

    if let Some(writer) = output.as_mut() {
        use std::io::Write;
        writer.flush()?;
    }

    if options.stats {
        eprintln!(
            "datalink vhf stats: demod_frames={} parsed_ok={} parse_fail={}",
            total.demod_frames, total.parsed_ok, total.parse_fail
        );
    }

    Ok(())
}

fn apply_source_overrides(options: &mut Options) {
    let Some(source) = options.source.as_mut() else {
        return;
    };
    if options.center_freq.is_some() {
        source.center_freq = options.center_freq;
    }
    if options.sample_rate.is_some() {
        source.sample_rate = options.sample_rate;
    }
    if options.channel.is_some() {
        source.channels = options.channel.clone();
    }
    if options.format.is_some() {
        source.format = options.format.clone();
    }
}

/// Decode a VHF ACARS file source and collect common decoded events for merged mode.
pub(crate) async fn decode_file_values(
    file: &str,
    format: Option<&str>,
    center_freq: Option<u32>,
    sample_rate: Option<u32>,
    channels: Option<&[u32]>,
    source_meta: &SourceMetadata,
    receiver_bearer: Bearer,
) -> anyhow::Result<Vec<DecodedEvent>> {
    let src = Source {
        address: Address::File {
            file: file.to_string(),
        },
        name: None,
        center_freq,
        sample_rate,
        channels: channels.map(<[u32]>::to_vec),
        gain: None,
        bias_tee: None,
        amp_enable: None,
        rf_gain: None,
        lna_gain: None,
        mixer_gain: None,
        vga_gain: None,
        format: format.map(str::to_string),
    };
    let mut out = Vec::new();
    decode_source(
        &src,
        None,
        None,
        Some(&mut out),
        source_meta,
        receiver_bearer,
    )
    .await?;
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
async fn decode_source(
    src: &Source,
    mut output: Option<&mut std::io::BufWriter<std::fs::File>>,
    mut redis: Option<&mut RedisPublisher>,
    mut collect: Option<&mut Vec<DecodedEvent>>,
    source_meta: &SourceMetadata,
    receiver_bearer: Bearer,
) -> anyhow::Result<DecodeStats> {
    if let Address::File { file } = &src.address {
        if file.to_ascii_lowercase().ends_with(".wav") {
            return decode_wav_source(
                src,
                file,
                output,
                redis,
                collect,
                source_meta,
                receiver_bearer,
            )
            .await;
        }
    }

    let effective_src = effective_file_source(src, None);
    let center_freq = effective_src.center_freq_or(DEFAULT_CENTER_FREQ);
    let raw_sample_rate = effective_src.sample_rate();
    let channels = effective_src.channels_with(auto_channels);
    let demod_sample_rate = 125_000;
    let mut channelizer = Channelizer::from_absolute_frequencies(
        center_freq,
        raw_sample_rate,
        demod_sample_rate,
        &channels,
    )
    .map_err(|err| anyhow::anyhow!(err))?;

    let mut demods: Vec<VhfChannel> = channels
        .iter()
        .map(|_| VhfChannel::new(demod_sample_rate as f32, 0.0))
        .collect();

    let mut stream = open_source(&effective_src).await?;
    let run_start = SystemTime::now();
    let mut sample_indices = vec![0_u64; channels.len()];
    let mut stats = DecodeStats::default();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result?;
        for channel_chunk in channelizer.process(&chunk) {
            let idx = channel_chunk.channel_index;
            for sample in channel_chunk.samples {
                let one = std::slice::from_mut(&mut demods[idx]);
                let mut passthrough = ResampleAdapter::new(None);
                let mut pipeline = IqPipeline::new(
                    &mut passthrough,
                    one,
                    &mut sample_indices[idx],
                    demod_sample_rate,
                    run_start,
                );
                let frames =
                    collect_iq_frames(&mut pipeline, sample.re, sample.im, |d, re, im| {
                        Ok(d.process_sample(re, im))
                    })?;
                for (mut ctx, demod_frame) in frames {
                    ctx.channel_index = idx;
                    handle_acars_frame(
                        &channels,
                        FrameSinks {
                            output: output.as_deref_mut(),
                            redis: redis.as_deref_mut(),
                            collect: collect.as_deref_mut(),
                        },
                        &mut stats,
                        ctx,
                        demod_frame,
                        source_meta,
                        receiver_bearer,
                    )
                    .await?;
                }
            }
        }
    }

    Ok(stats)
}

struct FrameSinks<'a> {
    output: Option<&'a mut std::io::BufWriter<std::fs::File>>,
    redis: Option<&'a mut RedisPublisher>,
    collect: Option<&'a mut Vec<DecodedEvent>>,
}

/// Parse one demodulated ACARS frame and emit it to configured sinks.
async fn handle_acars_frame(
    channels: &[u32],
    mut sinks: FrameSinks<'_>,
    stats: &mut DecodeStats,
    ctx: FrameContext,
    demod_frame: acars::demod::vhf::DemodFrame,
    source_meta: &SourceMetadata,
    receiver_bearer: Bearer,
) -> anyhow::Result<()> {
    stats.demod_frames += 1;
    match parse_acars_frame(&demod_frame.bytes, MessageDirection::Unknown) {
        Ok(message) => {
            stats.parsed_ok += 1;

            let pmsg = ProtocolMessage::Acars(Box::new(message));

            let event = DecodedEvent {
                event: "message".to_string(),
                timestamp: Some(ctx.timestamp_unix),
                bearer: receiver_bearer,
                source: source_meta.clone(),
                receiver: Some(ReceiverMetadata {
                    bearer: receiver_bearer,
                    channel_hz: Some(channels[ctx.channel_index]),
                }),
                aircraft: crate::merged::aircraft_summary(&pmsg),
                kinematics: pmsg.kinematics(),
                raw_frame_hex: Some(bytes_to_hex(&demod_frame.bytes)),
                message: pmsg,
            };

            if let Some(values) = sinks.collect.as_mut() {
                values.push(event);
            } else {
                let line = serde_json::to_string(&event)?;
                println!("{line}");
                if let Some(writer) = sinks.output.as_mut() {
                    use std::io::Write;
                    writeln!(writer, "{line}")?;
                }
                if let Some(redis) = sinks.redis {
                    redis
                        .publish(crate::util::redis_topic_for_record(&event.message), &line)
                        .await;
                }
            }
        }
        Err(_) => stats.parse_fail += 1,
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn decode_wav_source(
    src: &Source,
    file: &str,
    mut output: Option<&mut std::io::BufWriter<std::fs::File>>,
    mut redis: Option<&mut RedisPublisher>,
    mut collect: Option<&mut Vec<DecodedEvent>>,
    source_meta: &SourceMetadata,
    receiver_bearer: Bearer,
) -> anyhow::Result<DecodeStats> {
    let mut reader = hound::WavReader::open(expanduser(file))?;
    let spec = reader.spec();
    anyhow::ensure!(spec.channels == 2, "VHF WAV input must be stereo I/Q");
    anyhow::ensure!(
        spec.sample_format == hound::SampleFormat::Int && spec.bits_per_sample == 16,
        "VHF WAV input currently supports 16-bit PCM stereo I/Q"
    );
    let raw_sample_rate = spec.sample_rate;
    let effective_src = effective_file_source(src, Some(raw_sample_rate));
    let center_freq = effective_src.center_freq_or(DEFAULT_CENTER_FREQ);
    let channels = effective_src.channels_with(auto_channels);
    let (sample_rate, resample_rs) = maybe_resample(raw_sample_rate, 12_500);
    let mut adapter = ResampleAdapter::new(resample_rs);
    let mut demods: Vec<VhfChannel> = channels
        .iter()
        .map(|&ch_freq| VhfChannel::new(sample_rate as f32, ch_freq as f32 - center_freq as f32))
        .collect();
    let run_start = SystemTime::now();
    let mut sample_index: u64 = 0;
    let mut stats = DecodeStats::default();
    let mut samples = reader.samples::<i16>();
    while let (Some(i), Some(q)) = (samples.next(), samples.next()) {
        let i = i? as f32 / 32768.0;
        let q = q? as f32 / 32768.0;
        let mut pipeline = IqPipeline::new(
            &mut adapter,
            &mut demods,
            &mut sample_index,
            sample_rate,
            run_start,
        );
        let frames =
            collect_iq_frames(
                &mut pipeline,
                i,
                q,
                |d, re, im| Ok(d.process_sample(re, im)),
            )?;
        for (ctx, demod_frame) in frames {
            handle_acars_frame(
                &channels,
                FrameSinks {
                    output: output.as_deref_mut(),
                    redis: redis.as_deref_mut(),
                    collect: collect.as_deref_mut(),
                },
                &mut stats,
                ctx,
                demod_frame,
                source_meta,
                receiver_bearer,
            )
            .await?;
        }
    }
    Ok(stats)
}

fn auto_channels_for(center_freq: u32, sample_rate: u32) -> Vec<u32> {
    let half_bw = (sample_rate as f64 * 0.45) as u32;
    let lo = center_freq.saturating_sub(half_bw);
    let hi = center_freq.saturating_add(half_bw);
    let channels: Vec<u32> = KNOWN_ACARS_CHANNELS
        .iter()
        .copied()
        .filter(|&ch| ch >= lo && ch <= hi)
        .collect();
    if channels.is_empty() {
        DEFAULT_CHANNELS.to_vec()
    } else {
        channels
    }
}

fn effective_file_source(src: &Source, sample_rate_override: Option<u32>) -> Source {
    let inferred = file_source_path(src).and_then(infer_capture_params);
    let mut effective_src = src.clone();
    if effective_src.center_freq.is_none() {
        effective_src.center_freq = inferred.map(|params| params.center_freq);
    }
    effective_src.sample_rate = sample_rate_override
        .or(effective_src.sample_rate)
        .or_else(|| inferred.and_then(|params| params.sample_rate));
    if effective_src.format.is_none() {
        effective_src.format = inferred.and_then(|params| params.format.map(str::to_string));
    }
    effective_src
}

fn file_source_path(src: &Source) -> Option<&str> {
    match &src.address {
        Address::File { file } => Some(file.as_str()),
        _ => None,
    }
}

/// Open a VHF-compatible file, stdin, or SDR source as an async I/Q stream.
async fn open_source(src: &Source) -> anyhow::Result<desperado::IqAsyncSource> {
    use desperado::{DeviceConfig, IqAsyncSource};

    let center_freq = src.center_freq_or(DEFAULT_CENTER_FREQ);
    let sample_rate = src.sample_rate();
    match &src.address {
        Address::File { file } if file == "-" => Ok(IqAsyncSource::from_stdin(
            center_freq,
            sample_rate,
            DEFAULT_CHUNK_SIZE,
            src.iq_format(),
        )),
        Address::File { file } => Ok(IqAsyncSource::from_file(
            expanduser(file),
            center_freq,
            sample_rate,
            DEFAULT_CHUNK_SIZE,
            src.iq_format(),
        )
        .await?),
        #[cfg(feature = "rtlsdr")]
        Address::Rtlsdr { device, serial } => {
            let selector = if let Some(serial) = serial {
                desperado::rtlsdr::DeviceSelector::Filter {
                    manufacturer: None,
                    product: None,
                    serial: Some(serial.clone()),
                }
            } else {
                desperado::rtlsdr::DeviceSelector::Index(device.unwrap_or(0))
            };
            let cfg = desperado::rtlsdr::RtlSdrConfig {
                device: selector,
                center_freq,
                sample_rate,
                gain: src.gain_or(desperado::Gain::Manual(49.6)),
                bias_tee: src.bias_tee.unwrap_or(false),
                freq_correction_ppm: 0,
            };
            Ok(IqAsyncSource::from_device_config(&DeviceConfig::RtlSdr(cfg)).await?)
        }
        #[cfg(feature = "airspy")]
        Address::Airspy { device, serial } => {
            let selector = if let Some(serial) = serial {
                desperado::airspy::DeviceSelector::Serial(crate::util::parse_airspy_serial(serial)?)
            } else {
                desperado::airspy::DeviceSelector::Index(device.unwrap_or(0))
            };
            let cfg = desperado::airspy::AirspyConfig {
                device: selector,
                center_freq,
                sample_rate,
                gain: src.gain_or(desperado::Gain::Manual(50.0)),
                bias_tee: src.bias_tee.unwrap_or(false),
                packing: false,
                lna_gain: src.lna_gain.map(|v| v as u8),
                mixer_gain: src.mixer_gain.map(|v| v as u8),
                vga_gain: src.vga_gain.map(|v| v as u8),
                gain_mode: desperado::airspy::AirspyGainMode::Sensitivity,
            };
            Ok(IqAsyncSource::from_device_config(&DeviceConfig::Airspy(cfg)).await?)
        }
        #[cfg(feature = "hackrf")]
        Address::Hackrf { device } => {
            let cfg = desperado::hackrf::HackRfConfig {
                device_index: device.unwrap_or(0),
                center_freq: center_freq as u64,
                sample_rate,
                gain: crate::util::hackrf_gain(src),
                amp_enable: src.hackrf_amp_enable(),
                bias_tee: src.bias_tee.unwrap_or(false),
            };
            Ok(IqAsyncSource::from_device_config(&DeviceConfig::HackRf(cfg)).await?)
        }
        #[cfg(feature = "soapy")]
        Address::Soapy { soapy } => {
            let cfg = desperado::soapy::SoapyConfig {
                args: soapy.clone(),
                center_freq: center_freq as f64,
                sample_rate: sample_rate as f64,
                channel: 0,
                gain: src.gain_or(desperado::Gain::Manual(49.6)),
                bias_tee: src.bias_tee.unwrap_or(false),
            };
            Ok(IqAsyncSource::from_device_config(&DeviceConfig::Soapy(cfg)).await?)
        }
        #[allow(unreachable_patterns)]
        _ => Err(anyhow::anyhow!("source type is not enabled in this build")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_source(file: &str) -> Source {
        Source {
            address: Address::File {
                file: file.to_string(),
            },
            name: None,
            center_freq: None,
            sample_rate: None,
            channels: None,
            gain: None,
            bias_tee: None,
            amp_enable: None,
            rf_gain: None,
            lna_gain: None,
            mixer_gain: None,
            vga_gain: None,
            format: None,
        }
    }

    #[test]
    fn file_auto_channels_use_gqrx_inferred_sample_rate() {
        let src = file_source("gqrx_20260518_114201_131500000_1800000_fc.raw");
        let effective = effective_file_source(&src, None);
        assert_eq!(effective.center_freq_or(DEFAULT_CENTER_FREQ), 131_500_000);
        assert_eq!(effective.sample_rate(), 1_800_000);
        assert_eq!(effective.format.as_deref(), Some("cf32"));
        assert!(effective
            .channels_with(auto_channels)
            .contains(&131_525_000));
        assert!(effective
            .channels_with(auto_channels)
            .contains(&131_725_000));
        assert!(effective
            .channels_with(auto_channels)
            .contains(&131_825_000));
    }

    #[test]
    fn wav_auto_channels_use_inferred_center_and_wav_sample_rate() {
        let src = file_source("SDRuno_20200908_152020Z_131725kHz.wav");
        let effective = effective_file_source(&src, Some(500_000));
        assert_eq!(effective.center_freq_or(DEFAULT_CENTER_FREQ), 131_725_000);
        assert_eq!(effective.sample_rate(), 500_000);
        assert_eq!(
            effective.channels_with(auto_channels),
            vec![
                131_525_000,
                131_550_000,
                131_650_000,
                131_725_000,
                131_825_000,
                131_850_000,
            ]
        );
    }
}
