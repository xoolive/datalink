use acars::decode::acars::{parse_acars_frame, MessageDirection};
use acars::demod::acars131::Acars131Channel;
use clap::Parser;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Parser)]
#[command(name = "acars131", about = "Classic VHF ACARS frontend")]
struct Args {
    #[arg(short, long, help = "Path to I/Q recording file")]
    file: String,
    #[arg(
        long,
        default_value = "cu8",
        help = "I/Q sample format: cu8, cs8, cs16, cf32"
    )]
    format: String,
    #[arg(
        long,
        default_value_t = 131_700_000,
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
        default_values_t = [131_525_000u32, 131_725_000u32, 131_825_000u32],
        help = "ACARS channel frequency (or frequencies) to decode in Hz"
    )]
    channel: Vec<u32>,
    #[arg(long, help = "Print demod/decode counters to stderr at end")]
    stats: bool,
    #[arg(long, help = "Dump 12.5 kHz per-channel demod input as float WAV")]
    dump_demod_wav: Option<String>,
}

fn main() -> anyhow::Result<()> {
    use desperado::iqread::IqRead;
    use desperado::IqFormat;

    let args = Args::parse();
    let iq_format: IqFormat = args
        .format
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid --format: {e}"))?;

    let mut demods: Vec<Acars131Channel> = args
        .channel
        .iter()
        .map(|&ch_freq| {
            let offset_hz = ch_freq as f32 - args.center_freq as f32;
            Acars131Channel::new(args.sample_rate as f32, offset_hz)
        })
        .collect();

    let mut demod_wav = if let Some(path) = args.dump_demod_wav.as_deref() {
        let spec = hound::WavSpec {
            channels: args.channel.len() as u16,
            sample_rate: 12_500,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        Some(hound::WavWriter::create(path, spec)?)
    } else {
        None
    };

    let reader = IqRead::from_file(
        &args.file,
        args.center_freq,
        args.sample_rate,
        65536,
        iq_format,
    )?;

    let run_start = SystemTime::now();
    let mut sample_index: u64 = 0;
    let mut demod_frames = 0u64;
    let mut parsed_ok = 0u64;
    let mut parse_fail = 0u64;

    for chunk_result in reader {
        let chunk = chunk_result?;
        for sample in &chunk {
            sample_index = sample_index.saturating_add(1);
            let seconds_into_recording = sample_index as f64 / args.sample_rate as f64;
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
                    demod_frames += 1;
                    match parse_acars_frame(&demod_frame.bytes, MessageDirection::Unknown) {
                        Ok(message) => {
                            parsed_ok += 1;
                            let mut obj = serde_json::to_value(&message)?;
                            if let serde_json::Value::Object(ref mut m) = obj {
                                let channel_hz = args.channel[idx] as u64;
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
                        Err(_) => {
                            parse_fail += 1;
                        }
                    }
                }
            }
        }
    }

    if let Some(writer) = demod_wav.take() {
        writer.finalize()?;
    }

    if args.stats {
        eprintln!(
            "acars131 stats: demod_frames={} parsed_ok={} parse_fail={}",
            demod_frames, parsed_ok, parse_fail
        );
    }

    Ok(())
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut s, "{:02X}", b);
    }
    s
}
