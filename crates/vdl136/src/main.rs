use acars::decode::avlc::parse_avlc_frame;
use acars::demod::vdl2::Vdl2Channel;
use clap::{Parser, Subcommand};
use std::fs::{create_dir_all, File};
use std::io::{BufWriter, Write};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// VDL2 frontend for I/Q recordings.
#[derive(Debug, Parser)]
#[command(name = "vdl136", about = "VDL2 frontend for I/Q and SDR inputs")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Decode a raw I/Q recording (.rtl / .cu8 file).
    File {
        #[arg(short, long, help = "Path to I/Q recording file (Cu8 format)")]
        file: String,
        #[arg(
            long,
            default_value_t = 136_850_000,
            help = "Center frequency of the recording in Hz"
        )]
        center_freq: u32,
        #[arg(
            long,
            default_value_t = 1_050_000,
            help = "Sample rate of the recording in Hz"
        )]
        sample_rate: u32,
        #[arg(
            long,
            num_args = 1..,
            default_values_t = [136_875_000u32, 136_975_000u32],
            help = "VDL2 channel frequency (or frequencies) to decode in Hz"
        )]
        channel: Vec<u32>,
        #[arg(long, help = "Print demod/decode counters to stderr at end")]
        stats: bool,
        #[arg(
            long,
            help = "Write rejected AVLC frames as NDJSON (raw_frame_hex + parse error)"
        )]
        reject_log: Option<String>,
        #[arg(long, help = "Include frames with bad AVLC FCS in output JSON")]
        include_fcs_fail: bool,
        #[arg(long, help = "Write all AVLC candidates (ok/fail) as NDJSON")]
        candidate_log: Option<String>,
        #[arg(
            long,
            help = "Candidate log/output lower bound (seconds into recording)"
        )]
        window_start_sec: Option<f64>,
        #[arg(
            long,
            help = "Candidate log/output upper bound (seconds into recording)"
        )]
        window_end_sec: Option<f64>,
        #[arg(long, help = "Directory for per-channel demod trace NDJSON logs")]
        demod_trace_dir: Option<String>,
        #[arg(long, default_value_t = 3.2, help = "Preamble sync threshold")]
        sync_threshold: f32,
    },
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    match args.command {
        Command::File {
            file,
            center_freq,
            sample_rate,
            channel,
            stats,
            reject_log,
            include_fcs_fail,
            candidate_log,
            window_start_sec,
            window_end_sec,
            demod_trace_dir,
            sync_threshold,
        } => {
            decode_file(
                &file,
                center_freq,
                sample_rate,
                &channel,
                stats,
                reject_log.as_deref(),
                include_fcs_fail,
                candidate_log.as_deref(),
                window_start_sec,
                window_end_sec,
                demod_trace_dir.as_deref(),
                sync_threshold,
            )?;
        }
    }

    Ok(())
}

fn decode_file(
    path: &str,
    center_freq: u32,
    sample_rate: u32,
    channels: &[u32],
    print_stats: bool,
    reject_log: Option<&str>,
    include_fcs_fail: bool,
    candidate_log: Option<&str>,
    window_start_sec: Option<f64>,
    window_end_sec: Option<f64>,
    demod_trace_dir: Option<&str>,
    sync_threshold: f32,
) -> anyhow::Result<()> {
    use desperado::iqread::IqRead;
    use desperado::IqFormat;

    // Each VDL2 channel gets its own demodulator.
    let mut demods: Vec<Vdl2Channel> = channels
        .iter()
        .map(|&ch_freq| {
            let offset_hz = ch_freq as f32 - center_freq as f32;
            let mut d = Vdl2Channel::new(sample_rate as f32, offset_hz, ch_freq as f32);
            d.set_sync_threshold(sync_threshold);
            d
        })
        .collect();

    if let Some(dir) = demod_trace_dir {
        create_dir_all(dir)?;
        for (idx, d) in demods.iter_mut().enumerate() {
            let path = format!("{dir}/ch_{}.ndjson", channels[idx]);
            d.enable_trace(&path, window_start_sec, window_end_sec)?;
        }
    }

    let reader = IqRead::from_file(path, center_freq, sample_rate, 65536, IqFormat::Cu8)?;
    let run_start = SystemTime::now();
    let mut sample_index: u64 = 0;
    let mut stats = DecodeStats::default();
    let mut reject_writer = if let Some(path) = reject_log {
        Some(BufWriter::new(File::create(path)?))
    } else {
        None
    };
    let mut candidate_writer = if let Some(path) = candidate_log {
        Some(BufWriter::new(File::create(path)?))
    } else {
        None
    };

    for chunk_result in reader {
        let chunk = chunk_result?;
        for sample in &chunk {
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
                                    window_start_sec,
                                    window_end_sec,
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
                                            acars::decode::avlc::AvlcPayload::Acars(_) => "Acars",
                                            acars::decode::avlc::AvlcPayload::X25(_) => "X25",
                                            acars::decode::avlc::AvlcPayload::Xid(_) => "Xid",
                                            acars::decode::avlc::AvlcPayload::Unknown(_) => {
                                                "Unknown"
                                            }
                                        })
                                        .unwrap_or("None");
                                    let cand = serde_json::json!({
                                        "sample_index": sample_index,
                                        "seconds_into_recording": seconds_into_recording,
                                        "channel_mhz": channel_hz(channels[idx]),
                                        "parse_ok": true,
                                        "fcs_ok": avlc.fcs_ok,
                                        "src": avlc.src.addr,
                                        "dst": avlc.dst.addr,
                                        "cr": avlc.cr,
                                        "lcf": lcf_key,
                                        "payload_class": payload,
                                        "raw_frame_hex": bytes_to_hex(&demod_frame.bytes),
                                    });
                                    writeln!(w, "{}", serde_json::to_string(&cand)?)?;
                                }
                            }

                            if !include_fcs_fail && !avlc.fcs_ok {
                                continue;
                            }
                            if !in_window(seconds_into_recording, window_start_sec, window_end_sec)
                            {
                                continue;
                            }
                            let snr_db = demod_frame.signal_dbfs - demod_frame.noise_dbfs;
                            let channel_hz = channels[idx] as u64;
                            let mut obj = serde_json::to_value(&avlc)?;
                            if let serde_json::Value::Object(ref mut m) = obj {
                                m.insert("signal_dbfs".into(), demod_frame.signal_dbfs.into());
                                m.insert("noise_dbfs".into(), demod_frame.noise_dbfs.into());
                                m.insert("snr_db".into(), snr_db.into());
                                m.insert("ppm_error".into(), demod_frame.ppm_error.into());
                                m.insert(
                                    "channel_mhz".into(),
                                    (channel_hz as f64 / 1_000_000.0).into(),
                                );
                                m.insert("sample_index".into(), sample_index.into());
                                m.insert(
                                    "seconds_into_recording".into(),
                                    seconds_into_recording.into(),
                                );
                                m.insert("timestamp_unix".into(), timestamp_unix.into());
                                m.insert(
                                    "raw_frame_hex".into(),
                                    bytes_to_hex(&demod_frame.bytes).into(),
                                );
                            }
                            println!("{}", serde_json::to_string(&obj)?);
                        }
                        Err(err) => {
                            stats.avlc_parse_fail += 1;
                            if let Some(w) = candidate_writer.as_mut() {
                                if in_window(
                                    seconds_into_recording,
                                    window_start_sec,
                                    window_end_sec,
                                ) {
                                    let cand = serde_json::json!({
                                        "sample_index": sample_index,
                                        "seconds_into_recording": seconds_into_recording,
                                        "channel_mhz": channel_hz(channels[idx]),
                                        "parse_ok": false,
                                        "parse_error": err.to_string(),
                                        "raw_frame_hex": bytes_to_hex(&demod_frame.bytes),
                                    });
                                    writeln!(w, "{}", serde_json::to_string(&cand)?)?;
                                }
                            }
                            if let Some(w) = reject_writer.as_mut() {
                                if in_window(
                                    seconds_into_recording,
                                    window_start_sec,
                                    window_end_sec,
                                ) {
                                    let reject = serde_json::json!({
                                        "sample_index": sample_index,
                                        "seconds_into_recording": seconds_into_recording,
                                        "channel_mhz": channel_hz(channels[idx]),
                                        "frame_len": demod_frame.bytes.len(),
                                        "parse_error": err.to_string(),
                                        "raw_frame_hex": bytes_to_hex(&demod_frame.bytes),
                                    });
                                    writeln!(w, "{}", serde_json::to_string(&reject)?)?;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some(w) = reject_writer.as_mut() {
        w.flush()?;
    }
    if let Some(w) = candidate_writer.as_mut() {
        w.flush()?;
    }

    if print_stats {
        eprintln!(
            "vdl136 stats: demod_frames={} avlc_ok={} avlc_fcs_ok={} avlc_fcs_fail={} avlc_parse_fail={}",
            stats.demod_frames,
            stats.avlc_ok,
            stats.avlc_fcs_ok,
            stats.avlc_fcs_fail,
            stats.avlc_parse_fail
        );
    }

    Ok(())
}

#[derive(Default)]
struct DecodeStats {
    demod_frames: u64,
    avlc_ok: u64,
    avlc_fcs_ok: u64,
    avlc_fcs_fail: u64,
    avlc_parse_fail: u64,
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
