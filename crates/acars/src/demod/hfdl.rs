//! Native HFDL demodulation building blocks.
//!
//! This module owns the reusable HFDL PHY pipeline.  The executable frontend is
//! responsible for reading files/SDRs and passing complex samples here.  The
//! implementation deliberately uses `desperado` for generic DSP blocks
//! (resampling, AGC, and symbol timing) and keeps HFDL-specific framing/FEC in
//! this crate.
//!
//! Current status: acquisition/diagnostics plus deterministic interleaver and
//! scrambler primitives are implemented.  Full CRC-valid MPDU emission still
//! requires the final soft PSK demapper + Viterbi integration.

use desperado::dsp::{agc::Agc, resampler::ComplexResampler, symsync::SymSync};
use num_complex::Complex;
use serde::{Deserialize, Serialize};

pub const SYMBOL_RATE: u32 = 1_800;
pub const SAMPLES_PER_SYMBOL: u32 = 3;
pub const DEMOD_RATE: u32 = SYMBOL_RATE * SAMPLES_PER_SYMBOL;
pub const SSB_CARRIER_OFFSET_HZ: f64 = 1_440.0;

const A_LEN: usize = 127;
const M1_LEN: usize = 127;
const M2_LEN: usize = 15;
const T_LEN: usize = 15;
const DATA_FRAME_LEN: usize = 30;
const DATA_FRAME_CNT_SINGLE_SLOT: usize = 72;
const DATA_FRAME_CNT_DOUBLE_SLOT: usize = 168;
const DEINTERLEAVER_ROWS: usize = 40;
const DEINTERLEAVER_POP_ROW_SHIFT: usize = 9;

#[allow(clippy::excessive_precision)]
pub const MATCHED_FILTER: [f32; 19] = [
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

#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HfdlModulation {
    Bpsk = 1,
    Psk4 = 2,
    Psk8 = 3,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HfdlFrameParams {
    pub m1: usize,
    pub modulation: HfdlModulation,
    pub data_segment_count: usize,
    pub code_rate: usize,
    pub deinterleaver_column_shift: usize,
}

pub const FRAME_PARAMS: [HfdlFrameParams; 8] = [
    HfdlFrameParams {
        m1: 0,
        modulation: HfdlModulation::Bpsk,
        data_segment_count: DATA_FRAME_CNT_SINGLE_SLOT,
        code_rate: 4,
        deinterleaver_column_shift: 17,
    },
    HfdlFrameParams {
        m1: 1,
        modulation: HfdlModulation::Bpsk,
        data_segment_count: DATA_FRAME_CNT_SINGLE_SLOT,
        code_rate: 2,
        deinterleaver_column_shift: 17,
    },
    HfdlFrameParams {
        m1: 2,
        modulation: HfdlModulation::Psk4,
        data_segment_count: DATA_FRAME_CNT_SINGLE_SLOT,
        code_rate: 2,
        deinterleaver_column_shift: 17,
    },
    HfdlFrameParams {
        m1: 3,
        modulation: HfdlModulation::Psk8,
        data_segment_count: DATA_FRAME_CNT_SINGLE_SLOT,
        code_rate: 2,
        deinterleaver_column_shift: 17,
    },
    HfdlFrameParams {
        m1: 4,
        modulation: HfdlModulation::Bpsk,
        data_segment_count: DATA_FRAME_CNT_DOUBLE_SLOT,
        code_rate: 4,
        deinterleaver_column_shift: 23,
    },
    HfdlFrameParams {
        m1: 5,
        modulation: HfdlModulation::Bpsk,
        data_segment_count: DATA_FRAME_CNT_DOUBLE_SLOT,
        code_rate: 2,
        deinterleaver_column_shift: 23,
    },
    HfdlFrameParams {
        m1: 6,
        modulation: HfdlModulation::Psk4,
        data_segment_count: DATA_FRAME_CNT_DOUBLE_SLOT,
        code_rate: 2,
        deinterleaver_column_shift: 23,
    },
    HfdlFrameParams {
        m1: 7,
        modulation: HfdlModulation::Psk8,
        data_segment_count: DATA_FRAME_CNT_DOUBLE_SLOT,
        code_rate: 2,
        deinterleaver_column_shift: 23,
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreambleHit {
    pub correlation: f64,
    pub residual_hz: f64,
    pub sample_phase: usize,
    pub carrier_phase: f64,
    pub symbol_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameSyncHit {
    pub a1_correlation: f64,
    pub a2_correlation: f64,
    pub m1_correlation: f64,
    pub m2_correlation: f64,
    pub training_correlation: f64,
    pub m1: usize,
    pub residual_hz: f64,
    pub sample_phase: usize,
    pub carrier_phase: f64,
    pub symbol_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfdlDiagnostics {
    pub demod_sample_count: usize,
    pub symbol_count: usize,
    pub frame_hits: Vec<FrameSyncHit>,
    pub a_hits: Vec<PreambleHit>,
    pub pdu_candidates: Vec<HfdlPduCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfdlPduCandidate {
    pub m1: usize,
    pub byte_len: usize,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct HfdlDemodConfig {
    pub input_sample_rate: u32,
    pub center_freq_hz: f64,
    pub channel_khz: f64,
    pub use_symbol_sync: bool,
}

impl HfdlDemodConfig {
    pub fn carrier_offset_hz(&self) -> f64 {
        self.channel_khz * 1000.0 + SSB_CARRIER_OFFSET_HZ - self.center_freq_hz
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HfdlEvent {
    FrameSyncCandidate {
        bearer: String,
        channel_khz: f64,
        carrier_offset_hz: f64,
        seconds_into_recording: f64,
        a1_correlation: f64,
        a2_correlation: f64,
        m1_correlation: f64,
        m2_correlation: f64,
        training_correlation: f64,
        m1: usize,
        residual_hz: f64,
        sample_phase: usize,
        carrier_phase_rad: f64,
    },
    PreambleACandidate {
        bearer: String,
        channel_khz: f64,
        carrier_offset_hz: f64,
        seconds_into_recording: f64,
        correlation: f64,
        residual_hz: f64,
        sample_phase: usize,
        carrier_phase_rad: f64,
    },
    Activity {
        bearer: String,
        channel_khz: f64,
        carrier_offset_hz: f64,
        seconds_into_recording: f64,
        snr_db: f64,
    },
    ScanWindow {
        bearer: String,
        channel_khz: f64,
        carrier_offset_hz: f64,
        seconds_into_recording: f64,
        snr_db: f64,
    },
}

/// One-shot HFDL acquisition diagnostic for a single channel.
pub fn diagnose_channel(
    samples: &[Complex<f32>],
    config: &HfdlDemodConfig,
) -> Result<HfdlDiagnostics, String> {
    let mut baseband = channel_to_demod_rate(samples, config)?;
    let symbols = if config.use_symbol_sync {
        desperado_symbol_sync(&mut baseband)
    } else {
        apply_matched_filter(&mut baseband);
        decimate_fixed_phase(&baseband, 0)
    };
    let frame_hits = search_frame_sync_symbols(&symbols);
    let pdu_candidates = decode_frame_candidates(&symbols, &frame_hits);
    let a_hits = if frame_hits.is_empty() {
        search_a_sequence_symbols(&symbols)
    } else {
        Vec::new()
    };
    Ok(HfdlDiagnostics {
        demod_sample_count: baseband.len(),
        symbol_count: symbols.len(),
        frame_hits,
        a_hits,
        pdu_candidates,
    })
}

pub fn channel_to_demod_rate(
    samples: &[Complex<f32>],
    config: &HfdlDemodConfig,
) -> Result<Vec<Complex<f32>>, String> {
    let offset_hz = config.carrier_offset_hz();
    let mut mixed = Vec::with_capacity(samples.len());
    let mut phase = 0.0f64;
    let phase_step = -std::f64::consts::TAU * offset_hz / config.input_sample_rate as f64;
    for sample in samples {
        let osc = Complex::new(phase.cos() as f32, phase.sin() as f32);
        mixed.push(*sample * osc);
        phase += phase_step;
        if phase.abs() > std::f64::consts::TAU {
            phase %= std::f64::consts::TAU;
        }
    }

    let mut resampler = ComplexResampler::new(config.input_sample_rate, DEMOD_RATE)?;
    Ok(resampler.process(&mixed))
}

/// Run Desperado AGC + polyphase symbol synchronizer.
pub fn desperado_symbol_sync(samples: &mut [Complex<f32>]) -> Vec<Complex<f32>> {
    let mut agc = Agc::new(0.01);
    let mut sync = SymSync::new(
        usize::try_from(SAMPLES_PER_SYMBOL).unwrap(),
        16,
        &MATCHED_FILTER,
        0.01,
    );
    let mut symbols = Vec::with_capacity(samples.len() / SAMPLES_PER_SYMBOL as usize + 8);
    for sample in samples.iter() {
        let (i, q) = agc.execute(sample.re, sample.im);
        if let Some((si, sq)) = sync.push(i, q) {
            symbols.push(Complex::new(si, sq));
        }
    }
    symbols
}

pub fn apply_matched_filter(samples: &mut [Complex<f32>]) {
    let input = samples.to_vec();
    for idx in 0..samples.len() {
        let mut acc = Complex::new(0.0f32, 0.0f32);
        for (tap_idx, tap) in MATCHED_FILTER.iter().enumerate() {
            if idx >= tap_idx {
                acc += input[idx - tap_idx] * *tap;
            }
        }
        samples[idx] = acc;
    }
}

pub fn decimate_fixed_phase(samples: &[Complex<f32>], sample_phase: usize) -> Vec<Complex<f32>> {
    samples
        .iter()
        .skip(sample_phase)
        .step_by(SAMPLES_PER_SYMBOL as usize)
        .copied()
        .collect()
}

pub fn search_frame_sync_symbols(symbols: &[Complex<f32>]) -> Vec<FrameSyncHit> {
    let a_template = a_sequence_symbols();
    let m1_templates = m1_sequences();
    let t_template = training_sequence();
    let phases: Vec<f64> = (0..16)
        .map(|idx| std::f64::consts::PI * idx as f64 / 16.0)
        .collect();
    let needed = A_LEN + A_LEN + M1_LEN + M2_LEN + 9 * T_LEN;
    let mut hits = Vec::new();
    if symbols.len() < needed {
        return hits;
    }
    for residual_step in -40..=40 {
        let residual_hz = residual_step as f64 * 5.0;
        let phase_step = -std::f64::consts::TAU * residual_hz / SYMBOL_RATE as f64;
        let corrected: Vec<Complex<f32>> = symbols
            .iter()
            .enumerate()
            .map(|(idx, sample)| {
                let phase = phase_step * idx as f64;
                *sample * Complex::new(phase.cos() as f32, phase.sin() as f32)
            })
            .collect();
        for &carrier_phase in &phases {
            let hard = hard_bpsk(&corrected, carrier_phase);
            for idx in 0..=hard.len() - needed {
                let a1 = corr_abs(&a_template, &hard[idx..idx + A_LEN]);
                if a1 < 0.34 {
                    continue;
                }
                let a2_idx = idx + A_LEN;
                let a2 = corr_abs(&a_template, &hard[a2_idx..a2_idx + A_LEN]);
                if a2 < 0.28 {
                    continue;
                }
                let m1_idx = idx + 2 * A_LEN;
                let (m1, m1_corr) = m1_templates
                    .iter()
                    .enumerate()
                    .map(|(m1, tmpl)| (m1, corr_abs(tmpl, &hard[m1_idx..m1_idx + M1_LEN])))
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .unwrap();
                if m1_corr < 0.28 {
                    continue;
                }
                let m2_idx = m1_idx + M1_LEN;
                let m2_corr = corr_abs(&m1_templates[m1][..M2_LEN], &hard[m2_idx..m2_idx + M2_LEN]);
                if m2_corr < 0.20 {
                    continue;
                }
                let train_idx = m2_idx + M2_LEN;
                let training_correlation = (0..9)
                    .map(|seq| {
                        let start = train_idx + seq * T_LEN;
                        corr_abs(&t_template, &hard[start..start + T_LEN])
                    })
                    .sum::<f64>()
                    / 9.0;
                hits.push(FrameSyncHit {
                    a1_correlation: a1,
                    a2_correlation: a2,
                    m1_correlation: m1_corr,
                    m2_correlation: m2_corr,
                    training_correlation,
                    m1,
                    residual_hz,
                    sample_phase: 0,
                    carrier_phase,
                    symbol_index: idx,
                });
            }
        }
    }
    hits.sort_by(|a, b| sync_score(b).total_cmp(&sync_score(a)));
    hits.dedup_by(|a, b| {
        (a.symbol_index as isize - b.symbol_index as isize).abs() < 20
            && (a.residual_hz - b.residual_hz).abs() < 15.0
    });
    hits
}

pub fn decode_frame_candidates(
    symbols: &[Complex<f32>],
    hits: &[FrameSyncHit],
) -> Vec<HfdlPduCandidate> {
    hits.iter()
        .flat_map(|hit| decode_frame_candidate_variants(symbols, hit).unwrap_or_default())
        .collect()
}

pub fn decode_frame_candidate(
    symbols: &[Complex<f32>],
    hit: &FrameSyncHit,
) -> Result<HfdlPduCandidate, String> {
    decode_frame_candidate_variants(symbols, hit)?
        .into_iter()
        .next()
        .ok_or_else(|| "no candidate variants".into())
}

pub fn decode_frame_candidate_variants(
    symbols: &[Complex<f32>],
    hit: &FrameSyncHit,
) -> Result<Vec<HfdlPduCandidate>, String> {
    let params = *FRAME_PARAMS
        .get(hit.m1)
        .ok_or_else(|| format!("invalid M1 {}", hit.m1))?;
    let preamble_len = A_LEN + A_LEN + M1_LEN + M2_LEN + 9 * T_LEN;
    let data_start = hit.symbol_index + preamble_len;
    let frame_symbols = params.data_segment_count * (DATA_FRAME_LEN + T_LEN);
    if symbols.len() < data_start + frame_symbols {
        return Err("not enough symbols for full HFDL frame".into());
    }

    let mut data_symbols = Vec::with_capacity(params.data_segment_count * DATA_FRAME_LEN);
    let phase_step = -std::f64::consts::TAU * hit.residual_hz / SYMBOL_RATE as f64;
    let carrier = Complex::new(
        hit.carrier_phase.cos() as f32,
        (-hit.carrier_phase).sin() as f32,
    );
    let training = training_sequence();
    for seg in 0..params.data_segment_count {
        let seg_start = data_start + seg * (DATA_FRAME_LEN + T_LEN);
        let train_start = seg_start + DATA_FRAME_LEN;
        let mut train_sum = Complex::new(0.0f32, 0.0f32);
        if symbols.len() >= train_start + T_LEN {
            for (n, expected) in training.iter().enumerate() {
                let idx = train_start + n;
                let phase = phase_step * idx as f64;
                let residual = Complex::new(phase.cos() as f32, phase.sin() as f32);
                train_sum += symbols[idx] * residual * carrier * (*expected as f32);
            }
        }
        let train_phase = if train_sum.norm_sqr() > 1e-6 {
            -train_sum.arg()
        } else {
            0.0
        };
        let train_rot = Complex::new(train_phase.cos(), train_phase.sin());
        for (idx, symbol) in symbols
            .iter()
            .enumerate()
            .skip(seg_start)
            .take(DATA_FRAME_LEN)
        {
            let phase = phase_step * idx as f64;
            let residual = Complex::new(phase.cos() as f32, phase.sin() as f32);
            data_symbols.push(*symbol * residual * carrier * train_rot);
        }
    }

    let soft = soft_demod_symbols(&data_symbols, params.modulation, hit.m1 & 1);
    let deinterleaved = deinterleave_soft_bits(&soft, params);
    let viterbi_input = if params.code_rate == 4 {
        deinterleaved
            .chunks_exact(2)
            .map(|pair| (pair[0] & pair[1]) + ((pair[0] ^ pair[1]) >> 1))
            .collect()
    } else {
        deinterleaved
    };
    let decoded_bits = viterbi_decode_27(&viterbi_input)?;
    let mut variants = Vec::new();
    for bit_offset in 0..8 {
        if bit_offset >= decoded_bits.len() {
            break;
        }
        let mut bytes = bits_to_bytes_msb(&decoded_bits[bit_offset..]);
        for byte in &mut bytes {
            *byte = byte.reverse_bits();
        }
        variants.push(HfdlPduCandidate {
            m1: hit.m1,
            byte_len: bytes.len(),
            bytes,
        });
    }
    Ok(variants)
}

pub fn soft_demod_symbols(
    symbols: &[Complex<f32>],
    modulation: HfdlModulation,
    bitmask: usize,
) -> Vec<u8> {
    let mut descrambler = HfdlDescrambler::new();
    let mut out = Vec::with_capacity(symbols.len() * modulation as usize);
    for symbol in symbols {
        let flip = if descrambler.advance() != 0 {
            -1.0
        } else {
            1.0
        } * if bitmask & 1 != 0 { -1.0 } else { 1.0 };
        let s = *symbol * flip;
        match modulation {
            HfdlModulation::Bpsk => out.push(soft_from_metric(-s.re)),
            HfdlModulation::Psk4 => {
                out.push(soft_from_metric(-s.re));
                out.push(soft_from_metric(-s.im));
            }
            HfdlModulation::Psk8 => {
                // First-pass independent-axis/diagonal soft metrics.  This keeps
                // the pipeline complete while golden vectors tune exact HFDL 8PSK
                // labelling.
                out.push(soft_from_metric(-s.re));
                out.push(soft_from_metric(-s.im));
                out.push(soft_from_metric(-(s.re.abs() - s.im.abs())));
            }
        }
    }
    out
}

fn soft_from_metric(v: f32) -> u8 {
    ((v * 96.0 + 127.5).round().clamp(0.0, 255.0)) as u8
}

pub fn deinterleave_soft_bits(soft: &[u8], params: HfdlFrameParams) -> Vec<u8> {
    let mut deinterleaver = HfdlDeinterleaver::new(params);
    for &bit in soft.iter().take(deinterleaver.len()) {
        deinterleaver.push(bit);
    }
    (0..deinterleaver.len())
        .map(|_| deinterleaver.pop())
        .collect()
}

pub fn viterbi_decode_27(symbols: &[u8]) -> Result<Vec<u8>, String> {
    if !symbols.len().is_multiple_of(2) {
        return Err("Viterbi input must contain pairs of soft symbols".into());
    }
    let nbits = symbols.len() / 2;
    const STATES: usize = 64;
    const INF: u32 = u32::MAX / 4;
    let mut metrics = [INF; STATES];
    metrics[0] = 0;
    let mut decisions = vec![[0usize; STATES]; nbits];

    for (t, pair) in symbols.chunks_exact(2).enumerate() {
        let mut next = [INF; STATES];
        for (state, &metric) in metrics.iter().enumerate() {
            if metric >= INF {
                continue;
            }
            for bit in 0..=1usize {
                let reg = ((state << 1) | bit) & 0x7f;
                let next_state = reg & 0x3f;
                let e0 = parity(reg & 0x6d) as u8;
                let e1 = parity(reg & 0x4f) as u8;
                let branch = soft_distance(pair[0], e0) + soft_distance(pair[1], e1);
                let cand = metric.saturating_add(branch);
                if cand < next[next_state] {
                    next[next_state] = cand;
                    decisions[t][next_state] = state;
                }
            }
        }
        metrics = next;
    }

    let mut state = 0usize;
    let mut bits = vec![0u8; nbits];
    for t in (0..nbits).rev() {
        let prev = decisions[t][state];
        bits[t] = (state & 1) as u8;
        state = prev;
    }
    Ok(bits)
}

fn parity(mut x: usize) -> usize {
    let mut p = 0;
    while x != 0 {
        p ^= x & 1;
        x >>= 1;
    }
    p
}

fn soft_distance(symbol: u8, expected: u8) -> u32 {
    if expected == 0 {
        symbol as u32
    } else {
        255 - symbol as u32
    }
}

fn bits_to_bytes_msb(bits: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; bits.len().div_ceil(8)];
    for (idx, bit) in bits.iter().enumerate() {
        if *bit != 0 {
            out[idx / 8] |= 0x80 >> (idx % 8);
        }
    }
    out
}

pub fn search_a_sequence_symbols(symbols: &[Complex<f32>]) -> Vec<PreambleHit> {
    let template = a_sequence_symbols();
    let phases: Vec<f64> = (0..16)
        .map(|idx| std::f64::consts::PI * idx as f64 / 16.0)
        .collect();
    let mut hits = Vec::new();
    if symbols.len() < template.len() {
        return hits;
    }
    for residual_step in -24..=24 {
        let residual_hz = residual_step as f64 * 5.0;
        let phase_step = -std::f64::consts::TAU * residual_hz / SYMBOL_RATE as f64;
        let corrected: Vec<Complex<f32>> = symbols
            .iter()
            .enumerate()
            .map(|(idx, sample)| {
                let phase = phase_step * idx as f64;
                *sample * Complex::new(phase.cos() as f32, phase.sin() as f32)
            })
            .collect();
        for &carrier_phase in &phases {
            let hard = hard_bpsk(&corrected, carrier_phase);
            for idx in 0..=hard.len() - template.len() {
                let corr = corr_abs(&template, &hard[idx..idx + template.len()]);
                if corr >= 0.40 {
                    hits.push(PreambleHit {
                        correlation: corr,
                        residual_hz,
                        sample_phase: 0,
                        carrier_phase,
                        symbol_index: idx,
                    });
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

fn hard_bpsk(symbols: &[Complex<f32>], carrier_phase: f64) -> Vec<i8> {
    let rot = Complex::new(carrier_phase.cos() as f32, (-carrier_phase).sin() as f32);
    symbols
        .iter()
        .map(|s| if (*s * rot).re >= 0.0 { 1 } else { -1 })
        .collect()
}

fn sync_score(hit: &FrameSyncHit) -> f64 {
    hit.a1_correlation
        + hit.a2_correlation
        + hit.m1_correlation
        + hit.m2_correlation
        + hit.training_correlation
}

fn corr_abs(template: &[i8], observed: &[i8]) -> f64 {
    let matches = template
        .iter()
        .zip(observed)
        .filter(|(a, b)| a == b)
        .count() as f64;
    (2.0 * matches / template.len() as f64 - 1.0).abs()
}

pub fn training_sequence() -> Vec<i8> {
    vec![1, 1, 1, -1, 1, 1, -1, -1, 1, -1, 1, -1, -1, -1, -1]
}

pub fn m1_sequences() -> Vec<Vec<i8>> {
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
            (0..M1_LEN)
                .map(|idx| {
                    if m1_bits[(shift + idx) % M1_LEN] != 0 {
                        1
                    } else {
                        -1
                    }
                })
                .collect()
        })
        .collect()
}

pub fn a_sequence_symbols() -> Vec<i8> {
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
    let mut out = Vec::with_capacity(A_LEN);
    for byte in octets {
        for bit in (0..8).rev() {
            out.push(if (byte >> bit) & 1 != 0 { 1 } else { -1 });
            if out.len() == A_LEN {
                return out;
            }
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct HfdlDescrambler {
    reg: u32,
    seq_pos: usize,
}

impl Default for HfdlDescrambler {
    fn default() -> Self {
        Self::new()
    }
}

impl HfdlDescrambler {
    pub fn new() -> Self {
        Self {
            reg: 0x4a80,
            seq_pos: 0,
        }
    }

    pub fn reset(&mut self) {
        self.reg = 0x4a80;
        self.seq_pos = 0;
    }

    /// Advance the HFDL 15-bit scrambler and return the phase-flip bit.
    pub fn advance(&mut self) -> u8 {
        const LIQUID_HFDL_SEQ: [u8; 120] = [
            0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 0, 1, 1, 0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0,
            1, 0, 1, 0, 0, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 1, 0, 1, 0, 1, 1, 1,
            1, 0, 1, 1, 1, 1, 1, 1, 0, 0, 1, 1, 0, 1, 0, 1, 1, 0, 1, 0, 1, 0, 1, 1, 1, 0, 1, 1, 0,
            0, 1, 0, 0, 1, 1, 0, 0, 1, 0, 1, 1, 0, 1, 1, 1, 0, 0, 0, 1, 0, 0, 0, 1, 1, 0, 1, 1, 0,
            1, 0, 0, 0,
        ];
        let bit = LIQUID_HFDL_SEQ[self.seq_pos];
        self.seq_pos = (self.seq_pos + 1) % LIQUID_HFDL_SEQ.len();
        bit
    }
}

#[derive(Debug, Clone)]
pub struct HfdlDeinterleaver {
    table: Vec<u8>,
    row: usize,
    col: usize,
    columns: usize,
    push_column_shift: usize,
}

impl HfdlDeinterleaver {
    pub fn new(params: HfdlFrameParams) -> Self {
        let columns = params.data_segment_count * DATA_FRAME_LEN * params.modulation as usize
            / DEINTERLEAVER_ROWS;
        Self {
            table: vec![0; columns * DEINTERLEAVER_ROWS],
            row: 0,
            col: 0,
            columns,
            push_column_shift: params.deinterleaver_column_shift,
        }
    }

    pub fn reset(&mut self) {
        self.table.fill(0);
        self.row = 0;
        self.col = 0;
    }

    pub fn push(&mut self, val: u8) {
        self.table[self.row * self.columns + self.col] = val;
        self.row += 1;
        if self.row == DEINTERLEAVER_ROWS {
            self.row = 0;
            self.col = (self.col + 1) % self.columns;
        }
        self.col = (self.col + self.columns - self.push_column_shift % self.columns) % self.columns;
    }

    pub fn pop(&mut self) -> u8 {
        let ret = self.table[self.row * self.columns + self.col];
        self.row = (self.row + DEINTERLEAVER_POP_ROW_SHIFT) % DEINTERLEAVER_ROWS;
        if self.row == 0 {
            self.col = (self.col + 1) % self.columns;
        }
        ret
    }

    pub fn len(&self) -> usize {
        self.table.len()
    }
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_sequence_lengths() {
        assert_eq!(a_sequence_symbols().len(), 127);
        assert_eq!(m1_sequences().len(), 8);
        assert!(m1_sequences().iter().all(|s| s.len() == 127));
        assert_eq!(training_sequence().len(), 15);
    }

    #[test]
    fn deinterleaver_sizes_match_hfdl_modes() {
        assert_eq!(HfdlDeinterleaver::new(FRAME_PARAMS[0]).len(), 2160);
        assert_eq!(HfdlDeinterleaver::new(FRAME_PARAMS[3]).len(), 6480);
        assert_eq!(HfdlDeinterleaver::new(FRAME_PARAMS[7]).len(), 15120);
    }

    #[test]
    fn viterbi_roundtrip_hard_symbols() {
        let bits: [u8; 12] = [1, 0, 1, 1, 0, 0, 1, 0, 1, 0, 0, 0];
        let mut state = 0usize;
        let mut soft = Vec::new();
        for bit in bits {
            let reg = ((state << 1) | bit as usize) & 0x7f;
            state = reg & 0x3f;
            for poly in [0x6d, 0x4f] {
                soft.push(if parity(reg & poly) == 0 { 0 } else { 255 });
            }
        }
        // Tail to state zero, matching decoder chainback.
        for _ in 0..6 {
            let reg = (state << 1) & 0x7f;
            state = reg & 0x3f;
            for poly in [0x6d, 0x4f] {
                soft.push(if parity(reg & poly) == 0 { 0 } else { 255 });
            }
        }
        let decoded = viterbi_decode_27(&soft).unwrap();
        assert_eq!(&decoded[..bits.len()], bits);
    }
}
