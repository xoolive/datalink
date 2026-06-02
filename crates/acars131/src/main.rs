mod source;

use acars::decode::acars::{parse_acars_frame, MessageDirection};
use acars::demod::acars131::Acars131Channel;
use acars::demod::resample::{maybe_resample, ResampleAdapter};
use clap::Parser;
use futures_util::StreamExt;
use serde::Deserialize;
use source::{Address, Source, DEFAULT_CHUNK_SIZE};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::fs;

#[derive(Debug, Default, Clone, Deserialize, Parser)]
#[command(name = "acars131", about = "Classic VHF ACARS frontend")]
struct Options {
    /// Activate JSON output (currently JSONL is always emitted; kept for jet1090-style config compatibility)
    #[arg(short, long)]
    #[serde(default)]
    verbose: bool,

    /// Dump a copy of decoded messages as JSONL
    #[arg(short, long)]
    output: Option<String>,

    /// Include the full nested decoder output under raw_decode
    #[arg(long)]
    #[serde(default)]
    raw: bool,

    /// Print demod/decode counters to stderr at end
    #[arg(long)]
    #[serde(default)]
    stats: bool,

    /// Legacy file input path; equivalent to a file:// source
    #[arg(short, long)]
    #[serde(skip)]
    file: Option<String>,

    /// Legacy I/Q sample format for --file: cu8, cs8, cs16, cf32
    #[arg(long, default_value = "cu8")]
    #[serde(default)]
    format: Option<String>,

    /// Legacy center frequency for --file and default SDR sources
    #[arg(long)]
    #[serde(default)]
    center_freq: Option<u32>,

    /// Legacy sample rate for --file and default SDR sources
    #[arg(long)]
    #[serde(default)]
    sample_rate: Option<u32>,

    /// Legacy ACARS channel frequencies in Hz
    #[arg(long, num_args = 1..)]
    #[serde(default)]
    channel: Option<Vec<u32>>,

    /// Dump 12.5 kHz per-channel demod input as float WAV (single source only)
    #[arg(long)]
    #[serde(skip)]
    dump_demod_wav: Option<String>,

    /// Source URLs: file://, rtlsdr://, airspy://, hackrf://, soapy://
    #[serde(default)]
    sources: Vec<Source>,
}

#[derive(Default)]
struct DecodeStats {
    demod_frames: u64,
    parsed_ok: u64,
    parse_fail: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut options = load_config().await.unwrap_or_default();
    let cli = Options::parse();
    merge_cli(&mut options, cli)?;

    if options.sources.is_empty() {
        options.sources.push(Source {
            address: Address::File {
                file: "-".to_string(),
            },
            name: Some("stdin".to_string()),
            center_freq: options.center_freq,
            sample_rate: options.sample_rate,
            channels: options.channel.clone(),
            gain: None,
            bias_tee: None,
            amp_enable: None,
            lna_gain: None,
            vga_gain: None,
            format: options.format.clone(),
        });
    }

    let mut output = if let Some(path) = options.output.as_deref() {
        Some(std::io::BufWriter::new(std::fs::File::create(expanduser(
            path,
        ))?))
    } else {
        None
    };

    let mut total = DecodeStats::default();
    for src in options.sources.iter() {
        let stats = decode_source(
            src,
            options.dump_demod_wav.as_deref(),
            options.raw,
            output.as_mut(),
        )
        .await?;
        total.demod_frames += stats.demod_frames;
        total.parsed_ok += stats.parsed_ok;
        total.parse_fail += stats.parse_fail;
    }

    if let Some(writer) = output.as_mut() {
        use std::io::Write;
        writer.flush()?;
    }

    if options.stats {
        eprintln!(
            "acars131 stats: demod_frames={} parsed_ok={} parse_fail={}",
            total.demod_frames, total.parsed_ok, total.parse_fail
        );
    }

    Ok(())
}

async fn load_config() -> anyhow::Result<Options> {
    let mut path = match std::env::var("XDG_CONFIG_HOME") {
        Ok(value) => expanduser(&value),
        Err(_) => dirs::config_dir().unwrap_or_default(),
    };
    path.push("acars131");
    path.push("config.toml");

    let explicit = std::env::var("ACARS131_CONFIG")
        .ok()
        .map(|p| expanduser(&p));
    let path = explicit.unwrap_or(path);
    if !path.exists() {
        return Ok(Options::default());
    }
    let text = fs::read_to_string(path).await?;
    Ok(toml::from_str(&text)?)
}

fn merge_cli(options: &mut Options, cli: Options) -> anyhow::Result<()> {
    if cli.verbose {
        options.verbose = true;
    }
    if cli.output.is_some() {
        options.output = cli.output;
    }
    if cli.raw {
        options.raw = true;
    }
    if cli.stats {
        options.stats = true;
    }
    if cli.dump_demod_wav.is_some() {
        options.dump_demod_wav = cli.dump_demod_wav;
    }
    if let Some(file) = cli.file {
        options.sources = vec![Source {
            address: Address::File { file },
            name: None,
            center_freq: cli.center_freq,
            sample_rate: cli.sample_rate,
            channels: cli.channel.clone(),
            gain: None,
            bias_tee: None,
            amp_enable: None,
            lna_gain: None,
            vga_gain: None,
            format: cli.format.clone(),
        }];
    } else if !cli.sources.is_empty() {
        options.sources = cli.sources;
    }
    if cli.center_freq.is_some() {
        options.center_freq = cli.center_freq;
    }
    if cli.sample_rate.is_some() {
        options.sample_rate = cli.sample_rate;
    }
    if cli.channel.is_some() {
        options.channel = cli.channel;
    }
    if cli.format.as_deref() != Some("cu8") {
        options.format = cli.format;
    }
    Ok(())
}

async fn decode_source(
    src: &Source,
    dump_demod_wav: Option<&str>,
    raw: bool,
    mut output: Option<&mut std::io::BufWriter<std::fs::File>>,
) -> anyhow::Result<DecodeStats> {
    let center_freq = src.center_freq();
    let raw_sample_rate = src.sample_rate();
    let channels = src.channels();

    // Resample to nearest valid ACARS-131 demod rate (integer multiple of 12 500 Hz)
    let (sample_rate, resample_rs) = maybe_resample(raw_sample_rate, 12_500);
    let mut adapter = ResampleAdapter::new(resample_rs);
    if sample_rate != raw_sample_rate {
        eprintln!(
            "acars131: resampling {:.3} MHz \u{2192} {:.3} MHz for ACARS demod",
            raw_sample_rate as f64 / 1e6,
            sample_rate as f64 / 1e6
        );
    }

    let mut demods: Vec<Acars131Channel> = channels
        .iter()
        .map(|&ch_freq| {
            Acars131Channel::new(sample_rate as f32, ch_freq as f32 - center_freq as f32)
        })
        .collect();

    let mut demod_wav = if let Some(path) = dump_demod_wav {
        let spec = hound::WavSpec {
            channels: channels.len() as u16,
            sample_rate: 12_500,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        Some(hound::WavWriter::create(path, spec)?)
    } else {
        None
    };

    let mut stream = open_source(src).await?;
    let run_start = SystemTime::now();
    let mut sample_index: u64 = 0;
    let mut stats = DecodeStats::default();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result?;
        for raw_sample in &chunk {
            for sample in adapter.feed(raw_sample.re, raw_sample.im) {
                sample_index = sample_index.saturating_add(1);
                let seconds_into_recording = sample_index as f64 / sample_rate as f64;
                let frame_ts = run_start + Duration::from_secs_f64(seconds_into_recording);
                let timestamp_unix = frame_ts
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or_default();

                for (idx, d) in demods.iter_mut().enumerate() {
                    let (frames, maybe_dm) = d.process_sample_with_dm(sample.re, sample.im);
                    if let (Some(writer), Some(dm)) = (demod_wav.as_mut(), maybe_dm) {
                        let _ = writer.write_sample(dm);
                    }
                    for demod_frame in frames {
                        stats.demod_frames += 1;
                        match parse_acars_frame(&demod_frame.bytes, MessageDirection::Unknown) {
                            Ok(message) => {
                                stats.parsed_ok += 1;
                                let mut obj = serde_json::to_value(&message)?;
                                if let serde_json::Value::Object(ref mut m) = obj {
                                    let channel_hz = channels[idx] as u64;
                                    m.insert("timestamp".into(), timestamp_unix.into());
                                    m.insert(
                                        "frame".into(),
                                        bytes_to_hex(&demod_frame.bytes).into(),
                                    );
                                    m.insert(
                                        "metadata".into(),
                                        serde_json::json!({
                                            "bearer": "acars_vhf",
                                            "channel_mhz": channel_hz as f64 / 1_000_000.0,
                                        }),
                                    );
                                }
                                let obj = acars::decode::compact::compact_acars_value(obj, raw);
                                let line = serde_json::to_string(&obj)?;
                                println!("{line}");
                                if let Some(writer) = output.as_mut() {
                                    use std::io::Write;
                                    writeln!(writer, "{line}")?;
                                }
                            }
                            Err(_) => stats.parse_fail += 1,
                        }
                    } // end for demod_frame
                } // end for (idx, d)
            } // end for sample in adapter.feed
        } // end for raw_sample
    }

    if let Some(writer) = demod_wav.take() {
        writer.finalize()?;
    }

    Ok(stats)
}

async fn open_source(src: &Source) -> anyhow::Result<desperado::IqAsyncSource> {
    use desperado::{DeviceConfig, IqAsyncSource};

    let center_freq = src.center_freq();
    let sample_rate = src.sample_rate();
    match &src.address {
        Address::File { file } if file == "-" => Ok(IqAsyncSource::from_stdin(
            center_freq,
            sample_rate,
            DEFAULT_CHUNK_SIZE,
            src.iq_format(),
        )),
        Address::File { file } => Ok(IqAsyncSource::from_file(
            file,
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
                gain: src.gain(49.6),
                bias_tee: src.bias_tee.unwrap_or(false),
                freq_correction_ppm: 0,
            };
            Ok(IqAsyncSource::from_device_config(&DeviceConfig::RtlSdr(cfg)).await?)
        }
        #[cfg(feature = "airspy")]
        Address::Airspy { device, serial } => {
            let selector = if let Some(serial) = serial {
                desperado::airspy::DeviceSelector::Serial(parse_airspy_serial(serial)?)
            } else {
                desperado::airspy::DeviceSelector::Index(device.unwrap_or(0))
            };
            let cfg = desperado::airspy::AirspyConfig {
                device: selector,
                center_freq,
                sample_rate,
                gain: src.gain(50.0),
                bias_tee: src.bias_tee.unwrap_or(false),
                packing: false,
                lna_gain: None,
                mixer_gain: None,
                vga_gain: None,
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
                gain: hackrf_gain(src),
                amp_enable: src.amp_enable.unwrap_or(false),
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
                gain: src.gain(49.6),
                bias_tee: src.bias_tee.unwrap_or(false),
            };
            Ok(IqAsyncSource::from_device_config(&DeviceConfig::Soapy(cfg)).await?)
        }
        #[allow(unreachable_patterns)]
        _ => Err(anyhow::anyhow!("source type is not enabled in this build")),
    }
}

#[cfg(feature = "hackrf")]
fn hackrf_gain(src: &Source) -> desperado::Gain {
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

fn parse_airspy_serial(value: &str) -> anyhow::Result<u64> {
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

fn expanduser(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut s, "{:02X}", b);
    }
    s
}
