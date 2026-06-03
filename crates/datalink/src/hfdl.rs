use clap::{Parser, ValueEnum};
use desperado::dsp::resampler::ComplexResampler;
use rustfft::{num_complex::Complex, FftPlanner};
use serde_json::json;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::PathBuf;

const DEFAULT_HFDL_CHANNELS_KHZ: &[f64] = &[
    6529.0, 6532.0, 6535.0, 6559.0, 6565.0, 6589.0, 6596.0, 6619.0, 6628.0, 6646.0, 6652.0, 6661.0,
    6712.0, 8825.0, 8834.0, 8843.0, 8885.0, 8886.0, 8894.0, 8912.0, 8921.0, 8927.0, 8936.0, 8939.0,
    8942.0, 8948.0, 8957.0, 8977.0, 10027.0, 10030.0, 10060.0, 10063.0, 10066.0, 10081.0, 10084.0,
    10087.0, 10093.0, 11184.0, 11306.0, 11312.0, 11318.0, 11321.0, 11327.0, 11348.0, 11354.0,
    11384.0, 11387.0, 13264.0, 13270.0, 13276.0, 13303.0, 13312.0, 13315.0, 13321.0, 13324.0,
    13342.0, 13351.0,
];

const HFDL_SSB_CARRIER_OFFSET_HZ: f64 = 1440.0;
const FFT_SIZE: usize = 65_536;
const HFDL_SYMBOL_RATE: u32 = 1_800;
const HFDL_SPS: u32 = 3;
const HFDL_DEMOD_RATE: u32 = HFDL_SYMBOL_RATE * HFDL_SPS;
#[allow(clippy::excessive_precision)]
const HFDL_MATCHED_FILTER: [f32; 19] = [
    -0.0170974647427123,
    0.01148231492068473,
    0.03138375667422348,
    0.009454398851680437,
    -0.04161644170893816,
    -0.06451564801420356,
    -0.005495792933327306,
    0.1316404671361545,
    0.2759693160697777,
    0.3375901874933208,
    0.2759693160697777,
    0.1316404671361545,
    -0.005495792933327306,
    -0.06451564801420356,
    -0.04161644170893816,
    0.009454398851680437,
    0.03138375667422348,
    0.01148231492068473,
    -0.0170974647427123,
];

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum SampleFormat {
    U8,
    Cs16,
    Cf32,
}

impl SampleFormat {
    fn bytes_per_complex(self) -> usize {
        match self {
            Self::U8 => 2,
            Self::Cs16 => 4,
            Self::Cf32 => 8,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum HfdlMode {
    /// Native spectral activity scanner for HFDL channels.
    Scan,
    /// Native BPSK A-sequence preamble search diagnostic for one channel.
    Preamble,
    /// Native deterministic parser for raw HFDL PDU bytes supplied with --pdu-hex.
    ParsePdu,
}

#[derive(Debug, Parser)]
#[command(about = "Native HF Data Link experimental frontend")]
pub(crate) struct Options {
    /// I/Q file source, e.g. file://capture.raw or ~/capture.raw. Optional for --mode parse-pdu.
    source: Option<String>,

    /// Native operation mode
    #[arg(long, value_enum, default_value_t = HfdlMode::Scan)]
    mode: HfdlMode,

    /// Raw HFDL PDU bytes as hex for --mode parse-pdu
    #[arg(long)]
    pdu_hex: Option<String>,

    /// I/Q sample format
    #[arg(long, value_enum, default_value_t = SampleFormat::Cf32)]
    format: SampleFormat,

    /// Recording center frequency in Hz
    #[arg(long, default_value_t = 10_000_000)]
    center_freq: u32,

    /// Recording sample rate in samples/s
    #[arg(long, default_value_t = 8_000_000)]
    sample_rate: u32,

    /// HFDL channel frequencies, in Hz or kHz. Defaults to known channels within the recording bandwidth.
    #[arg(long, num_args = 1..)]
    channel: Option<Vec<f64>>,

    /// Start offset in seconds for scan/preamble modes
    #[arg(long, default_value_t = 0.0)]
    start_second: f64,

    /// Maximum seconds to scan from the file
    #[arg(long, default_value_t = 20.0)]
    max_seconds: f64,

    /// Detection threshold in dB above adjacent-channel estimate
    #[arg(long, default_value_t = 8.0)]
    threshold_db: f64,

    /// Emit every scanned window, not only detections
    #[arg(long)]
    all_windows: bool,
}

pub(crate) fn run(options: Options) -> anyhow::Result<()> {
    match options.mode {
        HfdlMode::ParsePdu => parse_pdu_mode(&options),
        HfdlMode::Preamble => preamble_mode(&options),
        HfdlMode::Scan => scan_mode(&options),
    }
}

fn parse_pdu_mode(options: &Options) -> anyhow::Result<()> {
    let hex = options
        .pdu_hex
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--pdu-hex is required with --mode parse-pdu"))?;
    let bytes = hex::decode(hex.split_whitespace().collect::<String>())?;
    let parsed = parse_hfdl_pdu(&bytes);
    println!("{}", serde_json::to_string_pretty(&parsed)?);
    Ok(())
}

fn preamble_mode(options: &Options) -> anyhow::Result<()> {
    let source = options
        .source
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("source is required for native HFDL preamble mode"))?;
    let path = normalize_source_path(source);
    let channels = channels_khz(options);
    anyhow::ensure!(
        channels.len() == 1,
        "preamble mode expects exactly one --channel for now"
    );
    let channel_khz = channels[0];
    let channel_hz = channel_khz * 1000.0 + HFDL_SSB_CARRIER_OFFSET_HZ;
    let offset_hz = channel_hz - options.center_freq as f64;
    let samples = read_complex_window(
        &path,
        options.format,
        options.sample_rate,
        options.start_second,
        options.max_seconds,
    )?;
    let mut mixed = Vec::with_capacity(samples.len());
    let mut phase = 0.0f64;
    let phase_step = -std::f64::consts::TAU * offset_hz / options.sample_rate as f64;
    for sample in samples {
        let osc = Complex::new(phase.cos() as f32, phase.sin() as f32);
        mixed.push(sample * osc);
        phase += phase_step;
        if phase.abs() > std::f64::consts::TAU {
            phase %= std::f64::consts::TAU;
        }
    }

    let mut resampler =
        ComplexResampler::new(options.sample_rate, HFDL_DEMOD_RATE).map_err(anyhow::Error::msg)?;
    let mut baseband = resampler.process(&mixed);
    apply_matched_filter(&mut baseband);
    let frame_hits = search_frame_sync(&baseband);
    let a_hits = if frame_hits.is_empty() {
        search_a_sequence(&baseband)
    } else {
        Vec::new()
    };
    eprintln!(
        "hfdl native preamble: channel={:.3} kHz samples={} demod_samples={} frame_hits={} a_hits={}",
        channel_khz,
        mixed.len(),
        baseband.len(),
        frame_hits.len(),
        a_hits.len()
    );
    for hit in frame_hits.iter().take(100) {
        println!(
            "{}",
            serde_json::to_string(&json!({
                "bearer": "hfdl",
                "event": "frame_sync_candidate",
                "channel_khz": channel_khz,
                "carrier_offset_hz": HFDL_SSB_CARRIER_OFFSET_HZ,
                "seconds_into_recording": options.start_second + hit.symbol_index as f64 / HFDL_SYMBOL_RATE as f64,
                "a1_correlation": hit.a1_correlation,
                "a2_correlation": hit.a2_correlation,
                "m1_correlation": hit.m1_correlation,
                "m2_correlation": hit.m2_correlation,
                "training_correlation": hit.training_correlation,
                "m1": hit.m1,
                "residual_hz": hit.residual_hz,
                "sample_phase": hit.sample_phase,
                "carrier_phase_rad": hit.carrier_phase,
            }))?
        );
    }
    for hit in a_hits.iter().take(100) {
        println!(
            "{}",
            serde_json::to_string(&json!({
                "bearer": "hfdl",
                "event": "preamble_a_candidate",
                "channel_khz": channel_khz,
                "carrier_offset_hz": HFDL_SSB_CARRIER_OFFSET_HZ,
                "seconds_into_recording": options.start_second + hit.symbol_index as f64 / HFDL_SYMBOL_RATE as f64,
                "correlation": hit.correlation,
                "residual_hz": hit.residual_hz,
                "sample_phase": hit.sample_phase,
                "carrier_phase_rad": hit.carrier_phase,
            }))?
        );
    }
    Ok(())
}

fn scan_mode(options: &Options) -> anyhow::Result<()> {
    let source = options
        .source
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("source is required for native HFDL scan mode"))?;
    let path = normalize_source_path(source);
    let channels = channels_khz(options);
    anyhow::ensure!(
        !channels.is_empty(),
        "no HFDL channels selected; pass --channel or use a wider/centered recording"
    );

    let mut reader = BufReader::new(File::open(&path)?);
    seek_to_second(
        &mut reader,
        options.format,
        options.sample_rate,
        options.start_second,
    )?;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let mut input = vec![Complex::new(0.0f32, 0.0f32); FFT_SIZE];
    let window: Vec<f32> = (0..FFT_SIZE)
        .map(|i| {
            let phase = std::f32::consts::TAU * i as f32 / (FFT_SIZE - 1) as f32;
            0.5 - 0.5 * phase.cos()
        })
        .collect();
    let mut raw = vec![0u8; FFT_SIZE * options.format.bytes_per_complex()];
    let max_blocks =
        ((options.max_seconds * options.sample_rate as f64) / FFT_SIZE as f64).ceil() as usize;
    let bin_hz = options.sample_rate as f64 / FFT_SIZE as f64;
    let mut summaries: Vec<ChannelSummary> = channels
        .iter()
        .map(|&khz| ChannelSummary::new(khz))
        .collect();

    for block_index in 0..max_blocks {
        if !read_complex_block(&mut reader, options.format, &mut raw, &mut input)? {
            break;
        }
        for (sample, gain) in input.iter_mut().zip(window.iter()) {
            sample.re *= gain;
            sample.im *= gain;
        }
        fft.process(&mut input);
        let seconds = block_index as f64 * FFT_SIZE as f64 / options.sample_rate as f64;
        let powers: Vec<f64> = input.iter().map(|v| v.norm_sqr() as f64 + 1e-30).collect();

        for (summary, &channel_khz) in summaries.iter_mut().zip(channels.iter()) {
            let carrier_hz = channel_khz * 1000.0 + HFDL_SSB_CARRIER_OFFSET_HZ;
            let offset_hz = carrier_hz - options.center_freq as f64;
            let center_power = band_power(&powers, offset_hz, bin_hz, 2);
            let adj_a = band_power(&powers, offset_hz - 30_000.0, bin_hz, 4);
            let adj_b = band_power(&powers, offset_hz + 30_000.0, bin_hz, 4);
            let noise = ((adj_a + adj_b) * 0.5).max(1e-30);
            let snr_db = 10.0 * (center_power / noise).log10();
            summary.observe(snr_db, seconds, options.threshold_db);
            if options.all_windows || snr_db >= options.threshold_db {
                println!(
                    "{}",
                    serde_json::to_string(&json!({
                        "bearer": "hfdl",
                        "event": if snr_db >= options.threshold_db { "activity" } else { "scan_window" },
                        "channel_khz": channel_khz,
                        "carrier_offset_hz": HFDL_SSB_CARRIER_OFFSET_HZ,
                        "seconds_into_recording": seconds,
                        "snr_db": snr_db,
                    }))?
                );
            }
        }
    }

    eprintln!(
        "hfdl native scan: channels={} max_seconds={} threshold_db={}",
        summaries.len(),
        options.max_seconds,
        options.threshold_db
    );
    for summary in summaries
        .iter()
        .filter(|s| s.best_snr_db >= options.threshold_db)
    {
        eprintln!(
            "hfdl activity: {:.3} kHz best_snr_db={:.1} at {:.3}s detections={}",
            summary.channel_khz, summary.best_snr_db, summary.best_seconds, summary.detections
        );
    }
    Ok(())
}

#[derive(Debug)]
struct ChannelSummary {
    channel_khz: f64,
    best_snr_db: f64,
    best_seconds: f64,
    detections: usize,
}

impl ChannelSummary {
    fn new(channel_khz: f64) -> Self {
        Self {
            channel_khz,
            best_snr_db: f64::NEG_INFINITY,
            best_seconds: 0.0,
            detections: 0,
        }
    }

    fn observe(&mut self, snr_db: f64, seconds: f64, threshold_db: f64) {
        if snr_db > self.best_snr_db {
            self.best_snr_db = snr_db;
            self.best_seconds = seconds;
        }
        if snr_db >= threshold_db {
            self.detections += 1;
        }
    }
}

#[derive(Debug, Clone)]
struct PreambleHit {
    correlation: f64,
    residual_hz: f64,
    sample_phase: usize,
    carrier_phase: f64,
    symbol_index: usize,
}

#[derive(Debug, Clone)]
struct FrameSyncHit {
    a1_correlation: f64,
    a2_correlation: f64,
    m1_correlation: f64,
    m2_correlation: f64,
    training_correlation: f64,
    m1: usize,
    residual_hz: f64,
    sample_phase: usize,
    carrier_phase: f64,
    symbol_index: usize,
}

fn read_complex_window(
    path: &str,
    format: SampleFormat,
    sample_rate: u32,
    start_second: f64,
    max_seconds: f64,
) -> anyhow::Result<Vec<Complex<f32>>> {
    let mut reader = BufReader::new(File::open(path)?);
    seek_to_second(&mut reader, format, sample_rate, start_second)?;
    let count = (sample_rate as f64 * max_seconds).ceil() as usize;
    let mut raw = vec![0u8; count * format.bytes_per_complex()];
    let mut filled = 0usize;
    while filled < raw.len() {
        let n = reader.read(&mut raw[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    raw.truncate(filled - (filled % format.bytes_per_complex()));
    let mut out = vec![Complex::new(0.0f32, 0.0f32); raw.len() / format.bytes_per_complex()];
    decode_complex_bytes(format, &raw, &mut out);
    Ok(out)
}

fn seek_to_second<R: Seek>(
    reader: &mut R,
    format: SampleFormat,
    sample_rate: u32,
    start_second: f64,
) -> anyhow::Result<()> {
    if start_second <= 0.0 {
        return Ok(());
    }
    let byte_offset =
        (start_second * sample_rate as f64).round() as u64 * format.bytes_per_complex() as u64;
    reader.seek(SeekFrom::Start(byte_offset))?;
    Ok(())
}

fn apply_matched_filter(samples: &mut [Complex<f32>]) {
    let input = samples.to_vec();
    for idx in 0..samples.len() {
        let mut acc = Complex::new(0.0f32, 0.0f32);
        for (tap_idx, tap) in HFDL_MATCHED_FILTER.iter().enumerate() {
            if idx >= tap_idx {
                acc += input[idx - tap_idx] * *tap;
            }
        }
        samples[idx] = acc;
    }
}

fn search_frame_sync(samples: &[Complex<f32>]) -> Vec<FrameSyncHit> {
    let a_template = a_sequence_symbols();
    let m1_templates = m1_sequences();
    let phases: Vec<f64> = (0..16)
        .map(|idx| std::f64::consts::PI * idx as f64 / 16.0)
        .collect();
    let mut hits = Vec::new();
    for residual_step in -40..=40 {
        let residual_hz = residual_step as f64 * 5.0;
        let phase_step = -std::f64::consts::TAU * residual_hz / HFDL_DEMOD_RATE as f64;
        for sample_phase in 0..HFDL_SPS as usize {
            let symbols: Vec<Complex<f32>> = samples
                .iter()
                .skip(sample_phase)
                .step_by(HFDL_SPS as usize)
                .enumerate()
                .map(|(idx, sample)| {
                    let phase = phase_step * (idx * HFDL_SPS as usize + sample_phase) as f64;
                    *sample * Complex::new(phase.cos() as f32, phase.sin() as f32)
                })
                .collect();
            let t_template = training_sequence();
            let needed = 127 + 127 + 127 + 15 + 9 * 15;
            if symbols.len() < needed {
                continue;
            }
            for &carrier_phase in &phases {
                let rot = Complex::new(carrier_phase.cos() as f32, (-carrier_phase).sin() as f32);
                let hard: Vec<i8> = symbols
                    .iter()
                    .map(|s| if (*s * rot).re >= 0.0 { 1 } else { -1 })
                    .collect();
                for idx in 0..=hard.len() - needed {
                    let a1 = corr_abs(&a_template, &hard[idx..idx + 127]);
                    if a1 < 0.34 {
                        continue;
                    }
                    let a2_idx = idx + 127;
                    let a2 = corr_abs(&a_template, &hard[a2_idx..a2_idx + 127]);
                    if a2 < 0.28 {
                        continue;
                    }
                    let m1_idx = idx + 254;
                    let (m1, m1_corr) = m1_templates
                        .iter()
                        .enumerate()
                        .map(|(m1, tmpl)| (m1, corr_abs(tmpl, &hard[m1_idx..m1_idx + 127])))
                        .max_by(|a, b| a.1.total_cmp(&b.1))
                        .unwrap();
                    if m1_corr < 0.28 {
                        continue;
                    }
                    let m2_idx = idx + 381;
                    let m2_corr = corr_abs(&m1_templates[m1][..15], &hard[m2_idx..m2_idx + 15]);
                    if m2_corr < 0.20 {
                        continue;
                    }
                    let train_idx = m2_idx + 15;
                    let mut training_corr = 0.0;
                    for seq in 0..9 {
                        let start = train_idx + seq * 15;
                        training_corr += corr_abs(&t_template, &hard[start..start + 15]);
                    }
                    training_corr /= 9.0;
                    hits.push(FrameSyncHit {
                        a1_correlation: a1,
                        a2_correlation: a2,
                        m1_correlation: m1_corr,
                        m2_correlation: m2_corr,
                        training_correlation: training_corr,
                        m1,
                        residual_hz,
                        sample_phase,
                        carrier_phase,
                        symbol_index: idx,
                    });
                }
            }
        }
    }
    hits.sort_by(|a, b| {
        let ac = a.a1_correlation
            + a.a2_correlation
            + a.m1_correlation
            + a.m2_correlation
            + a.training_correlation;
        let bc = b.a1_correlation
            + b.a2_correlation
            + b.m1_correlation
            + b.m2_correlation
            + b.training_correlation;
        bc.total_cmp(&ac)
    });
    hits.dedup_by(|a, b| {
        (a.symbol_index as isize - b.symbol_index as isize).abs() < 20
            && (a.residual_hz - b.residual_hz).abs() < 15.0
    });
    hits
}

fn corr_abs(template: &[i8], observed: &[i8]) -> f64 {
    let matches = template
        .iter()
        .zip(observed)
        .filter(|(a, b)| a == b)
        .count() as f64;
    (2.0 * matches / template.len() as f64 - 1.0).abs()
}

fn search_a_sequence(samples: &[Complex<f32>]) -> Vec<PreambleHit> {
    let template = a_sequence_symbols();
    let phases: Vec<f64> = (0..16)
        .map(|idx| std::f64::consts::PI * idx as f64 / 16.0)
        .collect();
    let mut hits = Vec::new();
    for residual_step in -24..=24 {
        let residual_hz = residual_step as f64 * 5.0;
        let phase_step = -std::f64::consts::TAU * residual_hz / HFDL_DEMOD_RATE as f64;
        for sample_phase in 0..HFDL_SPS as usize {
            let symbols: Vec<Complex<f32>> = samples
                .iter()
                .skip(sample_phase)
                .step_by(HFDL_SPS as usize)
                .enumerate()
                .map(|(idx, sample)| {
                    let phase = phase_step * (idx * HFDL_SPS as usize + sample_phase) as f64;
                    *sample * Complex::new(phase.cos() as f32, phase.sin() as f32)
                })
                .collect();
            if symbols.len() < template.len() {
                continue;
            }
            for &carrier_phase in &phases {
                let rot = Complex::new(carrier_phase.cos() as f32, (-carrier_phase).sin() as f32);
                let hard: Vec<i8> = symbols
                    .iter()
                    .map(|s| if (*s * rot).re >= 0.0 { 1 } else { -1 })
                    .collect();
                for idx in 0..=hard.len() - template.len() {
                    let corr = template
                        .iter()
                        .zip(&hard[idx..idx + template.len()])
                        .filter(|(a, b)| a == b)
                        .count() as f64;
                    let corr = (2.0 * corr / template.len() as f64 - 1.0).abs();
                    if corr >= 0.40 {
                        hits.push(PreambleHit {
                            correlation: corr,
                            residual_hz,
                            sample_phase,
                            carrier_phase,
                            symbol_index: idx,
                        });
                    }
                }
            }
        }
    }
    hits.sort_by(|a, b| b.correlation.total_cmp(&a.correlation));
    hits.dedup_by(|a, b| {
        (a.symbol_index as isize - b.symbol_index as isize).abs() < 10
            && (a.residual_hz - b.residual_hz).abs() < 10.0
    });
    hits
}

fn training_sequence() -> Vec<i8> {
    vec![1, 1, 1, -1, 1, 1, -1, -1, 1, -1, 1, -1, -1, -1, -1]
}

fn m1_sequences() -> Vec<Vec<i8>> {
    let m1_bits = [
        0, 1, 1, 1, 0, 1, 1, 0, 1, 1, 1, 1, 0, 1, 0, 0, 0, 1, 0, 1, 1, 0, 0, 1, 0, 1, 1, 1, 1, 1,
        0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 1, 1, 0, 1, 1, 0, 0, 0, 1, 1, 1, 0, 0, 1, 1, 1,
        0, 1, 0, 1, 1, 1, 0, 0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 0, 0, 0, 1, 0, 1, 0, 1, 0, 1, 1, 0, 1,
        0, 0, 1, 0, 0, 1, 0, 1, 0, 0, 1, 1, 1, 1, 0, 0, 1, 0, 0, 0, 1, 1, 0, 1, 0, 1, 0, 0, 0, 0,
        1, 1, 1, 1, 1, 1, 1,
    ];
    let shifts = [72usize, 82, 113, 123, 61, 103, 93, 9];
    shifts
        .iter()
        .map(|&shift| {
            (0..127)
                .map(|idx| {
                    if m1_bits[(shift + idx) % 127] != 0 {
                        1
                    } else {
                        -1
                    }
                })
                .collect()
        })
        .collect()
}

fn a_sequence_symbols() -> Vec<i8> {
    let octets = [
        0b01011011u8,
        0b10111100,
        0b01110100,
        0b01010111,
        0b00000011,
        0b11011001,
        0b10001001,
        0b00111001,
        0b11110010,
        0b00001000,
        0b11010101,
        0b00110110,
        0b10010100,
        0b00101100,
        0b00110010,
        0b11111110,
    ];
    let mut out = Vec::with_capacity(127);
    for byte in octets {
        for bit in (0..8).rev() {
            out.push(if (byte >> bit) & 1 != 0 { 1 } else { -1 });
            if out.len() == 127 {
                return out;
            }
        }
    }
    out
}

fn read_complex_block<R: Read>(
    reader: &mut R,
    format: SampleFormat,
    raw: &mut [u8],
    out: &mut [Complex<f32>],
) -> anyhow::Result<bool> {
    let mut filled = 0usize;
    while filled < raw.len() {
        let n = reader.read(&mut raw[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    if filled < raw.len() {
        return Ok(false);
    }

    decode_complex_bytes(format, raw, out);
    Ok(true)
}

fn decode_complex_bytes(format: SampleFormat, raw: &[u8], out: &mut [Complex<f32>]) {
    match format {
        SampleFormat::U8 => {
            for (idx, chunk) in raw.chunks_exact(2).enumerate() {
                out[idx].re = (chunk[0] as f32 - 127.5) / 128.0;
                out[idx].im = (chunk[1] as f32 - 127.5) / 128.0;
            }
        }
        SampleFormat::Cs16 => {
            for (idx, chunk) in raw.chunks_exact(4).enumerate() {
                let i = i16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 32768.0;
                let q = i16::from_le_bytes([chunk[2], chunk[3]]) as f32 / 32768.0;
                out[idx].re = i;
                out[idx].im = q;
            }
        }
        SampleFormat::Cf32 => {
            for (idx, chunk) in raw.chunks_exact(8).enumerate() {
                out[idx].re = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                out[idx].im = f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
            }
        }
    }
}

fn band_power(powers: &[f64], offset_hz: f64, bin_hz: f64, radius_bins: isize) -> f64 {
    let len = powers.len() as isize;
    let center = (offset_hz / bin_hz).round() as isize;
    let mut sum = 0.0;
    let mut count = 0usize;
    for delta in -radius_bins..=radius_bins {
        let signed = center + delta;
        let idx = if signed >= 0 { signed } else { len + signed };
        if (0..len).contains(&idx) {
            sum += powers[idx as usize];
            count += 1;
        }
    }
    sum / count.max(1) as f64
}

fn parse_hfdl_pdu(buf: &[u8]) -> serde_json::Value {
    if buf.is_empty() {
        return json!({ "bearer": "hfdl", "parse_ok": false, "error": "empty PDU" });
    }
    if buf[0] & 1 != 0 {
        parse_mpdu(buf)
    } else {
        parse_spdu(buf)
    }
}

fn parse_spdu(buf: &[u8]) -> serde_json::Value {
    let fcs_ok = hfdl_fcs_ok(buf, 64);
    json!({
        "bearer": "hfdl",
        "pdu": "spdu",
        "parse_ok": buf.len() >= 66,
        "fcs_ok": fcs_ok,
        "len": buf.len(),
        "ground_station_id": buf.get(1).map(|v| v & 0x7f),
    })
}

fn parse_mpdu(buf: &[u8]) -> serde_json::Value {
    let downlink = buf[0] & 0x02 != 0;
    if downlink {
        parse_downlink_mpdu(buf)
    } else {
        parse_uplink_mpdu(buf)
    }
}

fn parse_downlink_mpdu(buf: &[u8]) -> serde_json::Value {
    if buf.len() < 8 {
        return json!({ "bearer": "hfdl", "pdu": "mpdu", "direction": "downlink", "parse_ok": false, "error": "too short" });
    }
    let lpdu_count = ((buf[0] >> 2) & 0x0f) as usize;
    let header_len = 6 + lpdu_count;
    let fcs_ok = hfdl_fcs_ok(buf, header_len);
    let lpdu_lengths: Vec<usize> = buf
        .get(6..6 + lpdu_count)
        .unwrap_or_default()
        .iter()
        .map(|v| *v as usize + 1)
        .collect();
    json!({
        "bearer": "hfdl",
        "pdu": "mpdu",
        "direction": "downlink",
        "parse_ok": buf.len() >= header_len + 2,
        "fcs_ok": fcs_ok,
        "len": buf.len(),
        "src_aircraft_id": buf[2],
        "dst_ground_station_id": buf[1] & 0x7f,
        "lpdu_count": lpdu_count,
        "lpdu_lengths": lpdu_lengths,
    })
}

fn parse_uplink_mpdu(buf: &[u8]) -> serde_json::Value {
    if buf.len() < 5 {
        return json!({ "bearer": "hfdl", "pdu": "mpdu", "direction": "uplink", "parse_ok": false, "error": "too short" });
    }
    let aircraft_count = (((buf[0] & 0x70) >> 4) + 1) as usize;
    let mut pos = 3usize;
    let mut aircraft = Vec::new();
    for _ in 0..aircraft_count {
        if pos + 2 > buf.len() {
            break;
        }
        let aircraft_id = buf[pos];
        let lpdu_count = (buf[pos + 1] & 0x0f) as usize;
        pos += 2;
        let lengths: Vec<usize> = buf
            .get(pos..pos + lpdu_count)
            .unwrap_or_default()
            .iter()
            .map(|v| *v as usize + 1)
            .collect();
        pos += lpdu_count;
        aircraft.push(json!({ "aircraft_id": aircraft_id, "lpdu_count": lpdu_count, "lpdu_lengths": lengths }));
    }
    let fcs_ok = hfdl_fcs_ok(buf, pos);
    json!({
        "bearer": "hfdl",
        "pdu": "mpdu",
        "direction": "uplink",
        "parse_ok": buf.len() >= pos + 2,
        "fcs_ok": fcs_ok,
        "len": buf.len(),
        "src_ground_station_id": buf[1] & 0x7f,
        "aircraft": aircraft,
    })
}

fn hfdl_fcs_ok(buf: &[u8], header_len: usize) -> Option<bool> {
    if buf.len() < header_len + 2 {
        return None;
    }
    let got = u16::from_le_bytes([buf[header_len], buf[header_len + 1]]);
    let expected = crc16_ccitt_reflected(&buf[..header_len], 0xffff) ^ 0xffff;
    Some(got == expected)
}

fn crc16_ccitt_reflected(data: &[u8], init: u16) -> u16 {
    let mut crc = init;
    for &byte in data {
        crc ^= byte as u16;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0x8408;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

fn channels_khz(options: &Options) -> Vec<f64> {
    if let Some(channels) = &options.channel {
        return channels.iter().copied().map(to_khz).collect();
    }

    let center_khz = options.center_freq as f64 / 1000.0;
    let usable_half_bw_khz = options.sample_rate as f64 * 0.40 / 1000.0;
    let lo = center_khz - usable_half_bw_khz;
    let hi = center_khz + usable_half_bw_khz;
    DEFAULT_HFDL_CHANNELS_KHZ
        .iter()
        .copied()
        .filter(|freq| *freq >= lo && *freq <= hi)
        .collect()
}

fn to_khz(freq: f64) -> f64 {
    if freq > 100_000.0 {
        freq / 1000.0
    } else {
        freq
    }
}

fn normalize_source_path(source: &str) -> String {
    let path = source.strip_prefix("file://").unwrap_or(source);
    expanduser(path).to_string_lossy().into_owned()
}

fn expanduser(path: &str) -> PathBuf {
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
    fn crc_check_accepts_constructed_spdu() {
        let mut pdu = vec![0u8; 66];
        pdu[0] = 0x00;
        pdu[1] = 12;
        let fcs = crc16_ccitt_reflected(&pdu[..64], 0xffff) ^ 0xffff;
        pdu[64..66].copy_from_slice(&fcs.to_le_bytes());
        let parsed = parse_hfdl_pdu(&pdu);
        assert_eq!(parsed["pdu"], "spdu");
        assert_eq!(parsed["fcs_ok"], true);
    }

    #[test]
    fn auto_channels_cover_10mhz_capture() {
        let options = Options {
            source: Some("dummy".into()),
            mode: HfdlMode::Scan,
            pdu_hex: None,
            format: SampleFormat::Cf32,
            center_freq: 10_000_000,
            sample_rate: 8_000_000,
            channel: None,
            start_second: 0.0,
            max_seconds: 1.0,
            threshold_db: 8.0,
            all_windows: false,
        };
        let channels = channels_khz(&options);
        assert!(channels.contains(&10081.0));
        assert!(channels.contains(&11387.0));
        assert!(!channels.contains(&6529.0));
    }
}
