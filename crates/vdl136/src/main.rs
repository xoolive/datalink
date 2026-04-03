use acars::decode::acars::{parse_acars_frame, MessageDirection};
use acars::decode::adsc::parse_adsc_app_text;
use acars::decode::avlc::parse_avlc_frame;
use clap::{Parser, Subcommand, ValueEnum};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod demod;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Direction {
    Unknown,
    Uplink,
    Downlink,
}

/// Decode a hex-encoded ACARS or ADS-C payload from the command line.
#[derive(Debug, Parser)]
#[command(name = "vdl136", about = "Decode VDL2 / ACARS / ADS-C payloads")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Decode a hex ACARS frame.
    Acars {
        #[arg(help = "Hex-encoded ACARS frame bytes")]
        hex: String,
        #[arg(short, long, value_enum, default_value_t = Direction::Unknown)]
        direction: Direction,
    },
    /// Decode a hex AVLC frame (including 2-byte FCS).
    Avlc {
        #[arg(help = "Hex-encoded AVLC frame bytes (with FCS)")]
        hex: String,
    },
    /// Decode an ADS-C application-layer text payload.
    Adsc {
        #[arg(help = "ADS-C hex payload text")]
        payload: String,
    },
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
            default_values_t = [136_975_000u32],
            help = "VDL2 channel frequency (or frequencies) to decode in Hz"
        )]
        channel: Vec<u32>,
    },
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    match args.command {
        Command::Acars { hex, direction } => {
            let bytes = hex::decode(hex.trim())?;
            let dir = match direction {
                Direction::Unknown => MessageDirection::Unknown,
                Direction::Uplink => MessageDirection::GroundToAir,
                Direction::Downlink => MessageDirection::AirToGround,
            };
            let message = parse_acars_frame(&bytes, dir)?;
            println!("{}", serde_json::to_string_pretty(&message)?);
        }

        Command::Avlc { hex } => {
            let bytes = hex::decode(hex.trim())?;
            let frame = parse_avlc_frame(&bytes)?;
            let mut obj = serde_json::to_value(&frame)?;
            if let serde_json::Value::Object(ref mut m) = obj {
                m.insert("raw_frame_hex".into(), bytes_to_hex(&bytes).into());
            }
            println!("{}", serde_json::to_string_pretty(&obj)?);
        }

        Command::Adsc { payload } => {
            let adsc = parse_adsc_app_text(payload.trim())?;
            println!("{}", serde_json::to_string_pretty(&adsc)?);
        }

        Command::File {
            file,
            center_freq,
            sample_rate,
            channel,
        } => {
            decode_file(&file, center_freq, sample_rate, &channel)?;
        }
    }

    Ok(())
}

fn decode_file(
    path: &str,
    center_freq: u32,
    sample_rate: u32,
    channels: &[u32],
) -> anyhow::Result<()> {
    use desperado::iqread::IqRead;
    use desperado::IqFormat;

    // Each VDL2 channel gets its own demodulator.
    let mut demods: Vec<demod::Vdl2Channel> = channels
        .iter()
        .map(|&ch_freq| {
            let offset_hz = ch_freq as f32 - center_freq as f32;
            demod::Vdl2Channel::new(sample_rate as f32, offset_hz, ch_freq as f32)
        })
        .collect();

    let reader = IqRead::from_file(path, center_freq, sample_rate, 65536, IqFormat::Cu8)?;
    let run_start = SystemTime::now();
    let mut sample_index: u64 = 0;

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
                    if let Ok(avlc) = parse_avlc_frame(&demod_frame.bytes) {
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
                }
            }
        }
    }

    Ok(())
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join("")
}
