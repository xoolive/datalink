mod source;

use acars::decode::avlc::parse_avlc_frame;
use acars::demod::resample::{maybe_resample, ResampleAdapter};
use acars::demod::vdl2::{Vdl2Channel, SYMBOL_RATE};
use clap::Parser;
use futures_util::StreamExt;
use redis::AsyncCommands;
use serde::Deserialize;
use serde_json::Value;
use source::{Address, Source, DEFAULT_CHUNK_SIZE};
use std::fs::{create_dir_all, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Default, Clone, Deserialize, Parser)]
#[command(about = "VDL2 frontend for I/Q and SDR inputs")]
pub(crate) struct Options {
    /// Dump a copy of decoded AVLC frames as JSONL
    #[arg(short, long)]
    output: Option<String>,
    /// Print demod/decode counters to stderr at end
    #[arg(long)]
    #[serde(default)]
    stats: bool,
    /// Write rejected AVLC frames as NDJSON (frame + parse error)
    #[arg(long)]
    reject_log: Option<String>,
    /// Include the full nested decoder output under raw_decode
    #[arg(long)]
    #[serde(default)]
    raw: bool,
    /// Include frames with bad AVLC FCS in output JSON
    #[arg(long)]
    #[serde(default)]
    include_fcs_fail: bool,
    /// Write all AVLC candidates (ok/fail) as NDJSON
    #[arg(long)]
    candidate_log: Option<String>,
    /// Candidate log/output lower bound (seconds into recording)
    #[arg(long)]
    window_start_sec: Option<f64>,
    /// Candidate log/output upper bound (seconds into recording)
    #[arg(long)]
    window_end_sec: Option<f64>,
    /// Directory for per-channel demod trace NDJSON logs
    #[arg(long)]
    demod_trace_dir: Option<String>,
    /// Preamble sync threshold
    #[arg(long)]
    #[serde(default)]
    sync_threshold: Option<f32>,
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
    /// VDL2 channel frequencies in Hz
    #[arg(long, num_args = 1..)]
    #[serde(default)]
    channel: Option<Vec<u32>>,
    /// Publish decoded application messages to Redis pub/sub topics
    #[arg(long, value_name = "REDIS URL")]
    #[serde(default)]
    redis_url: Option<String>,
    /// Retry interval (seconds) when publishing to Redis fails; 0 disables retry
    #[arg(long, default_value_t = 5)]
    #[serde(default)]
    redis_retry_interval: u64,
    /// Source URL: file://, rtlsdr://, airspy://, hackrf://, soapy://
    #[serde(default)]
    source: Option<Source>,
}

#[derive(Default)]
struct DecodeStats {
    demod_frames: u64,
    avlc_ok: u64,
    avlc_fcs_ok: u64,
    avlc_fcs_fail: u64,
    avlc_parse_fail: u64,
}

struct RedisPublisher {
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
        loop {
            match self.connection.publish::<_, _, ()>(topic, payload).await {
                Ok(()) => break,
                Err(err) if self.retry_interval.is_zero() => {
                    eprintln!("datalink vdl2: Redis publish to {topic} failed: {err}");
                    break;
                }
                Err(err) => {
                    eprintln!(
                        "datalink vdl2: Redis publish to {topic} failed: {err}; retrying in {}s",
                        self.retry_interval.as_secs()
                    );
                    tokio::time::sleep(self.retry_interval).await;
                }
            }
        }
    }
}

pub(crate) async fn run(cli: Options) -> anyhow::Result<()> {
    let mut options = Options::default();
    merge_cli(&mut options, cli)?;

    anyhow::ensure!(
        options.source.is_some(),
        "missing source; pass an explicit source such as file://capture.rtl, -, or rtlsdr://"
    );
    run_options(options, "vdl2").await
}

async fn run_options(options: Options, stats_name: &str) -> anyhow::Result<()> {
    let mut output = if let Some(path) = options.output.as_deref() {
        Some(BufWriter::new(File::create(expanduser(path))?))
    } else {
        None
    };
    let mut reject_writer = if let Some(path) = options.reject_log.as_deref() {
        Some(BufWriter::new(File::create(expanduser(path))?))
    } else {
        None
    };
    let mut candidate_writer = if let Some(path) = options.candidate_log.as_deref() {
        Some(BufWriter::new(File::create(expanduser(path))?))
    } else {
        None
    };

    let mut redis = if let Some(url) = options.redis_url.as_deref() {
        Some(RedisPublisher::connect(url, options.redis_retry_interval).await?)
    } else {
        None
    };

    let mut total = DecodeStats::default();
    let src = options.source.as_ref().expect("source checked before run");
    let stats = decode_source(
        src,
        0,
        &options,
        output.as_mut(),
        reject_writer.as_mut(),
        candidate_writer.as_mut(),
        redis.as_mut(),
        None,
    )
    .await?;
    total.demod_frames += stats.demod_frames;
    total.avlc_ok += stats.avlc_ok;
    total.avlc_fcs_ok += stats.avlc_fcs_ok;
    total.avlc_fcs_fail += stats.avlc_fcs_fail;
    total.avlc_parse_fail += stats.avlc_parse_fail;

    if let Some(w) = output.as_mut() {
        w.flush()?;
    }
    if let Some(w) = reject_writer.as_mut() {
        w.flush()?;
    }
    if let Some(w) = candidate_writer.as_mut() {
        w.flush()?;
    }

    if options.stats {
        eprintln!(
            "{} stats: demod_frames={} avlc_ok={} avlc_fcs_ok={} avlc_fcs_fail={} avlc_parse_fail={}",
            stats_name, total.demod_frames, total.avlc_ok, total.avlc_fcs_ok, total.avlc_fcs_fail, total.avlc_parse_fail
        );
    }

    Ok(())
}

fn merge_cli(options: &mut Options, cli: Options) -> anyhow::Result<()> {
    if cli.output.is_some() {
        options.output = cli.output;
    }
    if cli.stats {
        options.stats = true;
    }
    if cli.reject_log.is_some() {
        options.reject_log = cli.reject_log;
    }
    if cli.raw {
        options.raw = true;
    }
    if cli.include_fcs_fail {
        options.include_fcs_fail = true;
    }
    if cli.candidate_log.is_some() {
        options.candidate_log = cli.candidate_log;
    }
    if cli.window_start_sec.is_some() {
        options.window_start_sec = cli.window_start_sec;
    }
    if cli.window_end_sec.is_some() {
        options.window_end_sec = cli.window_end_sec;
    }
    if cli.demod_trace_dir.is_some() {
        options.demod_trace_dir = cli.demod_trace_dir;
    }
    if cli.sync_threshold.is_some() {
        options.sync_threshold = cli.sync_threshold;
    }
    if cli.redis_url.is_some() {
        options.redis_url = cli.redis_url;
    }
    if cli.redis_retry_interval != 5 {
        options.redis_retry_interval = cli.redis_retry_interval;
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
    let options = Options {
        raw,
        source: Some(src.clone()),
        ..Options::default()
    };
    let mut out = Vec::new();
    decode_source(&src, 0, &options, None, None, None, None, Some(&mut out)).await?;
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
async fn decode_source(
    src: &Source,
    source_index: usize,
    options: &Options,
    mut output: Option<&mut BufWriter<File>>,
    mut reject_writer: Option<&mut BufWriter<File>>,
    mut candidate_writer: Option<&mut BufWriter<File>>,
    mut redis: Option<&mut RedisPublisher>,
    mut collect: Option<&mut Vec<Value>>,
) -> anyhow::Result<DecodeStats> {
    if let Address::File { file } = &src.address {
        if file.to_ascii_lowercase().ends_with(".wav") {
            return decode_wav_source(src, file, source_index, options, output, redis, collect)
                .await;
        }
    }

    let center_freq = src.center_freq();
    let raw_sample_rate = src.sample_rate();
    let channels = src.channels();
    let source_label = src.label();
    let sync_threshold = options.sync_threshold.unwrap_or(3.2);

    // Compute the nearest valid VDL2 demod rate (integer multiple of SYMBOL_RATE * SPS = 105 000)
    // and set up a transparent resampler if the source rate is not already valid.
    let vdl2_decimated_rate = SYMBOL_RATE * 10; // 105_000
    let (sample_rate, resample_rs) = maybe_resample(raw_sample_rate, vdl2_decimated_rate);
    let mut adapter = ResampleAdapter::new(resample_rs);
    if sample_rate != raw_sample_rate {
        eprintln!(
            "datalink vdl2: resampling {:.3} MHz → {:.3} MHz for VDL2 demod",
            raw_sample_rate as f64 / 1e6,
            sample_rate as f64 / 1e6
        );
    }

    let mut demods: Vec<Vdl2Channel> = channels
        .iter()
        .map(|&ch_freq| {
            let mut d = Vdl2Channel::new(
                sample_rate as f32,
                ch_freq as f32 - center_freq as f32,
                ch_freq as f32,
            );
            d.set_sync_threshold(sync_threshold);
            d
        })
        .collect();

    if let Some(dir) = options.demod_trace_dir.as_deref() {
        create_dir_all(dir)?;
        for (idx, d) in demods.iter_mut().enumerate() {
            let path = format!("{dir}/src_{source_index}_ch_{}.ndjson", channels[idx]);
            d.enable_trace(&path, options.window_start_sec, options.window_end_sec)?;
        }
    }

    let mut stream = open_source(src).await?;
    let run_start = SystemTime::now();
    let mut sample_index: u64 = 0;
    let mut stats = DecodeStats::default();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result?;
        for raw_sample in &chunk {
            // Feed through the resampler (passthrough if rate is already valid)
            for sample in adapter.feed(raw_sample.re, raw_sample.im) {
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
                        match parse_avlc_frame(&demod_frame.bytes) {
                            Ok(avlc) => {
                                stats.avlc_ok += 1;
                                if avlc.fcs_ok {
                                    stats.avlc_fcs_ok += 1;
                                } else {
                                    stats.avlc_fcs_fail += 1;
                                }
                                if let Some(w) = candidate_writer.as_mut() {
                                    if in_window(
                                        seconds_into_recording,
                                        options.window_start_sec,
                                        options.window_end_sec,
                                    ) {
                                        let lcf_key = match &avlc.lcf {
                                            acars::decode::avlc::AvlcLcf::I { .. } => "I",
                                            acars::decode::avlc::AvlcLcf::S { .. } => "S",
                                            acars::decode::avlc::AvlcLcf::U { .. } => "U",
                                        };
                                        let payload = avlc
                                            .payload
                                            .as_ref()
                                            .map(|p| match p {
                                                acars::decode::avlc::AvlcPayload::Acars(_) => {
                                                    "Acars"
                                                }
                                                acars::decode::avlc::AvlcPayload::X25(_) => "X25",
                                                acars::decode::avlc::AvlcPayload::Xid(_) => "Xid",
                                                acars::decode::avlc::AvlcPayload::Unknown(_) => {
                                                    "Unknown"
                                                }
                                            })
                                            .unwrap_or("None");
                                        let cand = serde_json::json!({"source": source_label, "source_index": source_index, "sample_index": sample_index, "seconds_into_recording": seconds_into_recording, "channel_mhz": channel_hz(channels[idx]), "parse_ok": true, "fcs_ok": avlc.fcs_ok, "src": avlc.src.icao24, "dst": avlc.dst.icao24, "role": avlc.role, "lcf": lcf_key, "payload_class": payload, "frame": bytes_to_hex(&demod_frame.bytes)});
                                        writeln!(w, "{}", serde_json::to_string(&cand)?)?;
                                    }
                                }
                                if !options.include_fcs_fail && !avlc.fcs_ok {
                                    continue;
                                }
                                if !in_window(
                                    seconds_into_recording,
                                    options.window_start_sec,
                                    options.window_end_sec,
                                ) {
                                    continue;
                                }
                                let _snr_db = demod_frame.signal_dbfs - demod_frame.noise_dbfs;
                                let channel_hz = channels[idx] as u64;
                                let mut obj = serde_json::to_value(&avlc)?;
                                if let serde_json::Value::Object(ref mut m) = obj {
                                    m.insert("timestamp".into(), timestamp_unix.into());
                                    m.insert(
                                        "frame".into(),
                                        bytes_to_hex(&demod_frame.bytes).into(),
                                    );
                                    m.insert(
                                        "metadata".into(),
                                        serde_json::json!({
                                            "bearer": "vdl2",
                                            "channel_mhz": channel_hz as f64 / 1_000_000.0,
                                        }),
                                    );
                                }
                                let obj =
                                    acars::decode::compact::compact_avlc_value(obj, options.raw);
                                let topic = redis_topic_for_record(&obj);
                                if let Some(values) = collect.as_mut() {
                                    values.push(obj);
                                } else {
                                    let line = serde_json::to_string(&obj)?;
                                    println!("{line}");
                                    if let Some(w) = output.as_mut() {
                                        writeln!(w, "{line}")?;
                                    }
                                    if let (Some(redis), Some(topic)) =
                                        (redis.as_deref_mut(), topic)
                                    {
                                        redis.publish(topic, &line).await;
                                    }
                                }
                            }
                            Err(err) => {
                                stats.avlc_parse_fail += 1;
                                if let Some(w) = candidate_writer.as_mut() {
                                    if in_window(
                                        seconds_into_recording,
                                        options.window_start_sec,
                                        options.window_end_sec,
                                    ) {
                                        let cand = serde_json::json!({"source": source_label, "source_index": source_index, "sample_index": sample_index, "seconds_into_recording": seconds_into_recording, "channel_mhz": channel_hz(channels[idx]), "parse_ok": false, "parse_error": err.to_string(), "frame": bytes_to_hex(&demod_frame.bytes)});
                                        writeln!(w, "{}", serde_json::to_string(&cand)?)?;
                                    }
                                }
                                if let Some(w) = reject_writer.as_mut() {
                                    if in_window(
                                        seconds_into_recording,
                                        options.window_start_sec,
                                        options.window_end_sec,
                                    ) {
                                        let reject = serde_json::json!({"source": source_label, "source_index": source_index, "sample_index": sample_index, "seconds_into_recording": seconds_into_recording, "channel_mhz": channel_hz(channels[idx]), "frame_len": demod_frame.bytes.len(), "parse_error": err.to_string(), "frame": bytes_to_hex(&demod_frame.bytes)});
                                        writeln!(w, "{}", serde_json::to_string(&reject)?)?;
                                    }
                                }
                            }
                        }
                    } // end for demod_frame
                } // end for (idx, d)
            } // end for sample in adapter.feed
        } // end for raw_sample in chunk
    }
    Ok(stats)
}

async fn decode_wav_source(
    src: &Source,
    file: &str,
    _source_index: usize,
    options: &Options,
    mut output: Option<&mut BufWriter<File>>,
    mut redis: Option<&mut RedisPublisher>,
    mut collect: Option<&mut Vec<Value>>,
) -> anyhow::Result<DecodeStats> {
    let mut reader = hound::WavReader::open(expanduser(file))?;
    let spec = reader.spec();
    anyhow::ensure!(spec.channels == 2, "VDL2 WAV input must be stereo I/Q");
    anyhow::ensure!(
        spec.sample_format == hound::SampleFormat::Int && spec.bits_per_sample == 16,
        "VDL2 WAV input currently supports 16-bit PCM stereo I/Q"
    );
    let raw_sample_rate = spec.sample_rate;
    let center_freq = src
        .center_freq
        .or_else(|| infer_sdruno_center_freq(file))
        .unwrap_or_else(|| src.center_freq());
    let channels = src.channels();
    let sync_threshold = options.sync_threshold.unwrap_or(3.2);
    let vdl2_decimated_rate = SYMBOL_RATE * 10;
    let (sample_rate, resample_rs) = maybe_resample(raw_sample_rate, vdl2_decimated_rate);
    let mut adapter = ResampleAdapter::new(resample_rs);
    let mut demods: Vec<Vdl2Channel> = channels
        .iter()
        .map(|&ch_freq| {
            let mut d = Vdl2Channel::new(
                sample_rate as f32,
                ch_freq as f32 - center_freq as f32,
                ch_freq as f32,
            );
            d.set_sync_threshold(sync_threshold);
            d
        })
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
                    match parse_avlc_frame(&demod_frame.bytes) {
                        Ok(avlc) => {
                            stats.avlc_ok += 1;
                            if avlc.fcs_ok {
                                stats.avlc_fcs_ok += 1;
                            } else {
                                stats.avlc_fcs_fail += 1;
                            }
                            if !options.include_fcs_fail && !avlc.fcs_ok {
                                continue;
                            }
                            let channel_hz = channels[idx] as u64;
                            let mut obj = serde_json::to_value(&avlc)?;
                            if let serde_json::Value::Object(ref mut m) = obj {
                                m.insert("timestamp".into(), timestamp_unix.into());
                                m.insert("frame".into(), bytes_to_hex(&demod_frame.bytes).into());
                                m.insert("metadata".into(), serde_json::json!({"bearer":"vdl2", "channel_mhz": channel_hz as f64 / 1_000_000.0}));
                            }
                            let obj = acars::decode::compact::compact_avlc_value(obj, options.raw);
                            let topic = redis_topic_for_record(&obj);
                            if let Some(values) = collect.as_mut() {
                                values.push(obj);
                            } else {
                                let line = serde_json::to_string(&obj)?;
                                println!("{line}");
                                if let Some(w) = output.as_mut() {
                                    writeln!(w, "{line}")?;
                                }
                                if let (Some(redis), Some(topic)) = (redis.as_deref_mut(), topic) {
                                    redis.publish(topic, &line).await;
                                }
                            }
                        }
                        Err(_) => stats.avlc_parse_fail += 1,
                    }
                }
            }
        }
    }
    Ok(stats)
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

fn redis_topic_for_record(record: &Value) -> Option<&'static str> {
    if record.get("path").and_then(Value::as_str) == Some("acars") {
        return Some("datalink-acars");
    }
    if record.get("path").and_then(Value::as_str) == Some("avlc") {
        return Some("datalink-vdl2");
    }
    Some("datalink-other")
}

async fn open_source(src: &Source) -> anyhow::Result<desperado::IqAsyncSource> {
    use desperado::IqAsyncSource;
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
            Ok(IqAsyncSource::from_device_config(&desperado::DeviceConfig::RtlSdr(cfg)).await?)
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
            Ok(IqAsyncSource::from_device_config(&desperado::DeviceConfig::Airspy(cfg)).await?)
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
            Ok(IqAsyncSource::from_device_config(&desperado::DeviceConfig::HackRf(cfg)).await?)
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
            Ok(IqAsyncSource::from_device_config(&desperado::DeviceConfig::Soapy(cfg)).await?)
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

#[cfg(feature = "airspy")]
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

fn channel_hz(hz: u32) -> f64 {
    hz as f64 / 1_000_000.0
}

fn in_window(seconds: f64, start: Option<f64>, end: Option<f64>) -> bool {
    if let Some(s) = start {
        if seconds < s {
            return false;
        }
    }
    if let Some(e) = end {
        if seconds > e {
            return false;
        }
    }
    true
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join("")
}
