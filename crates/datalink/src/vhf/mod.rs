mod source;

use acars::decode::acars::{parse_acars_frame, MessageDirection};
use acars::demod::resample::{maybe_resample, ResampleAdapter};
use acars::demod::vhf::VhfChannel;
use clap::Parser;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use source::{Address, Source, DEFAULT_CHANNELS, DEFAULT_CHUNK_SIZE, KNOWN_ACARS_CHANNELS};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Default, Clone, Deserialize, Parser)]
#[command(about = "Classic VHF ACARS frontend")]
pub(crate) struct Options {
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

    /// I/Q sample format for file input: cu8, cs8, cs16, cf32
    #[arg(long, default_value = "cu8")]
    #[serde(default)]
    format: Option<String>,

    /// Center frequency for file and SDR sources
    #[arg(long)]
    #[serde(default)]
    center_freq: Option<u32>,

    /// Sample rate for file and SDR sources
    #[arg(long)]
    #[serde(default)]
    sample_rate: Option<u32>,

    /// ACARS channel frequencies in Hz
    #[arg(long, num_args = 1..)]
    #[serde(default)]
    channel: Option<Vec<u32>>,

    /// Dump 12.5 kHz per-channel demod input as float WAV (single source only)
    #[arg(long)]
    #[serde(skip)]
    dump_demod_wav: Option<String>,

    /// Source URL: file://, rtlsdr://, airspy://, hackrf://, soapy://
    #[serde(default)]
    source: Option<Source>,
}

#[derive(Default)]
struct DecodeStats {
    demod_frames: u64,
    parsed_ok: u64,
    parse_fail: u64,
}

pub(crate) async fn run(cli: Options) -> anyhow::Result<()> {
    let mut options = Options::default();
    merge_cli(&mut options, cli)?;

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

    let mut total = DecodeStats::default();
    let src = options.source.as_ref().expect("source checked before run");
    let stats = decode_source(
        src,
        options.dump_demod_wav.as_deref(),
        options.raw,
        output.as_mut(),
        None,
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
    if cli.source.is_some() {
        options.source = cli.source;
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
    apply_source_overrides(options);
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
    if options.format.as_deref() != Some("cu8") {
        source.format = options.format.clone();
    }
}

pub(crate) async fn decode_file_values(
    file: &str,
    format: Option<&str>,
    center_freq: Option<u32>,
    sample_rate: Option<u32>,
    channels: Option<Vec<u32>>,
    raw: bool,
) -> anyhow::Result<Vec<Value>> {
    let src = Source {
        address: Address::File {
            file: file.to_string(),
        },
        name: None,
        center_freq,
        sample_rate,
        channels,
        gain: None,
        bias_tee: None,
        amp_enable: None,
        lna_gain: None,
        vga_gain: None,
        format: format.map(str::to_string),
    };
    let mut out = Vec::new();
    decode_source(&src, None, raw, None, Some(&mut out)).await?;
    Ok(out)
}

async fn decode_source(
    src: &Source,
    dump_demod_wav: Option<&str>,
    raw: bool,
    mut output: Option<&mut std::io::BufWriter<std::fs::File>>,
    mut collect: Option<&mut Vec<Value>>,
) -> anyhow::Result<DecodeStats> {
    if let Address::File { file } = &src.address {
        if file.to_ascii_lowercase().ends_with(".wav") {
            return decode_wav_source(src, file, raw, output, collect).await;
        }
    }

    let center_freq = src.center_freq();
    let raw_sample_rate = src.sample_rate();
    let channels = src.channels();

    // Resample to nearest valid ACARS-131 demod rate (integer multiple of 12 500 Hz)
    let (sample_rate, resample_rs) = maybe_resample(raw_sample_rate, 12_500);
    let mut adapter = ResampleAdapter::new(resample_rs);
    if sample_rate != raw_sample_rate {
        eprintln!(
            "datalink vhf: resampling {:.3} MHz \u{2192} {:.3} MHz for ACARS demod",
            raw_sample_rate as f64 / 1e6,
            sample_rate as f64 / 1e6
        );
    }

    let mut demods: Vec<VhfChannel> = channels
        .iter()
        .map(|&ch_freq| VhfChannel::new(sample_rate as f32, ch_freq as f32 - center_freq as f32))
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
                                if let Some(values) = collect.as_mut() {
                                    values.push(obj);
                                } else {
                                    let line = serde_json::to_string(&obj)?;
                                    println!("{line}");
                                    if let Some(writer) = output.as_mut() {
                                        use std::io::Write;
                                        writeln!(writer, "{line}")?;
                                    }
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

async fn decode_wav_source(
    src: &Source,
    file: &str,
    raw: bool,
    mut output: Option<&mut std::io::BufWriter<std::fs::File>>,
    mut collect: Option<&mut Vec<Value>>,
) -> anyhow::Result<DecodeStats> {
    let mut reader = hound::WavReader::open(expanduser(file))?;
    let spec = reader.spec();
    anyhow::ensure!(spec.channels == 2, "VHF WAV input must be stereo I/Q");
    anyhow::ensure!(
        spec.sample_format == hound::SampleFormat::Int && spec.bits_per_sample == 16,
        "VHF WAV input currently supports 16-bit PCM stereo I/Q"
    );
    let raw_sample_rate = spec.sample_rate;
    let inferred_center = infer_sdruno_center_freq(file);
    let center_freq = src
        .center_freq
        .or(inferred_center)
        .unwrap_or_else(|| src.center_freq());
    let channels = src
        .channels
        .clone()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| auto_channels_for(center_freq, raw_sample_rate));
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
        for sample in adapter.feed(i, q) {
            sample_index = sample_index.saturating_add(1);
            let seconds_into_recording = sample_index as f64 / sample_rate as f64;
            let frame_ts = run_start + Duration::from_secs_f64(seconds_into_recording);
            let timestamp_unix = frame_ts
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or_default();
            for (idx, d) in demods.iter_mut().enumerate() {
                for demod_frame in d.process_sample(sample.re, sample.im) {
                    stats.demod_frames += 1;
                    match parse_acars_frame(&demod_frame.bytes, MessageDirection::Unknown) {
                        Ok(message) => {
                            stats.parsed_ok += 1;
                            let mut obj = serde_json::to_value(&message)?;
                            if let serde_json::Value::Object(ref mut m) = obj {
                                let channel_hz = channels[idx] as u64;
                                m.insert("timestamp".into(), timestamp_unix.into());
                                m.insert("frame".into(), bytes_to_hex(&demod_frame.bytes).into());
                                m.insert("metadata".into(), serde_json::json!({"bearer":"acars_vhf", "channel_mhz": channel_hz as f64 / 1_000_000.0}));
                            }
                            let obj = acars::decode::compact::compact_acars_value(obj, raw);
                            if let Some(values) = collect.as_mut() {
                                values.push(obj);
                            } else {
                                let line = serde_json::to_string(&obj)?;
                                println!("{line}");
                                if let Some(writer) = output.as_mut() {
                                    use std::io::Write;
                                    writeln!(writer, "{line}")?;
                                }
                            }
                        }
                        Err(_) => stats.parse_fail += 1,
                    }
                }
            }
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

fn infer_sdruno_center_freq(path: &str) -> Option<u32> {
    let name = std::path::Path::new(path).file_name()?.to_string_lossy();
    let khz_pos = name.find("kHz")?;
    let prefix = &name[..khz_pos];
    let digits: String = prefix
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    digits.parse::<u32>().ok().map(|khz| khz * 1000)
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
