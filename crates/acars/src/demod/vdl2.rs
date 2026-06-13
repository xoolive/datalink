//! VDL Mode 2 demodulator — converts I/Q samples into AVLC frame bytes.
//!
//! # Pipeline
//!
//! ```text
//! Cu8 file (1 050 000 sps)
//!   → desperado IqRead                 [Vec<Complex<f32>>]
//!   → optional downmix (NCO × sample)  [Complex<f32>]
//!   → Chebyshev 2-pole LPF             [Complex<f32>]
//!   → decimation ×10 → 105 000 sps    [Complex<f32>]
//!   → D8PSK phase-difference decode    [3 bits / symbol]
//!   → frame sync (16-symbol preamble)  [bit stream]
//!   → header FEC (5-bit syndrome)
//!   → LFSR descramble (IV 0x6959)
//!   → RS(255,249) FEC + deinterleave
//!   → HDLC bit-destuff
//!   → AVLC frame bytes  Vec<u8>
//! ```
//!

use std::f32::consts::PI;
use std::fs::File;
use std::io::{BufWriter, Write};

use desperado::dsp::chebyshev::Chebyshev2Lpf;
use desperado::dsp::downsample::EveryN;
use desperado::dsp::nco::Nco;
use serde::Serialize;

pub const SYMBOL_RATE: u32 = 10_500;
/// Samples per symbol in the decimated domain.
const SPS: u32 = 10;
/// Bits per symbol (D8PSK → 2³ = 8 constellation points).
const BPS: u32 = 3;
/// Constellation size.
const ARITY: usize = 8;
/// Number of symbols in the VDL2 preamble.
const PREAMBLE_SYMS: usize = 16;
/// Sliding look-behind buffer for frame sync.
const SYNC_BUFLEN: usize = PREAMBLE_SYMS * SPS as usize;
/// Frame-sync checks are performed every SYNC_SKIP decimated samples.
const SYNC_SKIP: u32 = 3;
const DEFAULT_SYNC_THRESHOLD: f32 = 4.0;
const MAG_LP: f32 = 0.9;
const NF_LP: f32 = 0.85;

const HDRFECLEN: u32 = 5;
const TRLEN: u32 = 17;
const HEADER_LEN: u32 = 3 + TRLEN + HDRFECLEN; // = 25 bits
const LFSR_IV: u16 = 0x6959;
const RS_N: usize = 255;
const RS_K: usize = 249;

const INP_LPF_CUTOFF_HZ: f32 = 8_000.0;
const INP_LPF_RIPPLE: f32 = 0.5; // percent

// D8PSK preamble phases in cumulative radians.

const PR_PHASE: [f32; PREAMBLE_SYMS] = [
    0.0 * PI / 4.0,
    3.0 * PI / 4.0,
    -3.0 * PI / 4.0,
    1.0 * PI / 4.0,
    1.0 * PI / 4.0,
    2.0 * PI / 4.0,
    0.0 * PI / 4.0,
    4.0 * PI / 4.0,
    -3.0 * PI / 4.0,
    4.0 * PI / 4.0,
    -2.0 * PI / 4.0,
    3.0 * PI / 4.0,
    1.0 * PI / 4.0,
    -2.0 * PI / 4.0,
    -3.0 * PI / 4.0,
    0.0 * PI / 4.0,
];

/// Gray-code mapping for D8PSK symbol index → 3-bit value.
const GRAYCODE: [u8; ARITY] = [0, 1, 3, 2, 6, 7, 5, 4];

// Header FEC parity-check matrix and syndrome correction table.

#[allow(clippy::unusual_byte_groupings)]
const H: [u32; HDRFECLEN as usize] = [
    0b000_0000_0111_1111_1111_1100_00,
    0b001_1111_1000_0111_1111_1010_00,
    0b110_0011_1001_1000_0111_1001_00,
    0b110_1101_1010_1001_1001_1000_10,
    0b011_0100_1111_0010_1010_1000_01,
];

const SYNDTABLE: [u32; 32] = [
    0x00000000, 0x00000001, 0x00000002, 0x00800004, 0x00000004, 0x00800002, 0x01000000, 0x00800000,
    0x00000008, 0x00400000, 0x00200000, 0x00100000, 0x00080000, 0x01100000, 0x00040000, 0x00020000,
    0x00000010, 0x00010000, 0x00804000, 0x00008000, 0x00808000, 0x00004000, 0x00002000, 0x01010000,
    0x00001000, 0x00000800, 0x00000400, 0x00000200, 0x00000100, 0x00000080, 0x00000040, 0x00000020,
];

// Reed-Solomon RS(255,249) decoder.
// Ported from Phil Karn's libfec (LGPL), parameters: GF(2^8) poly=0x187,
// fcr=120, prim=1, nroots=6, pad=0.

const RS_NROOTS: usize = RS_N - RS_K; // 6
const RS_FCR: usize = 120;
const RS_PRIM: usize = 1;
const RS_IPRIM: usize = 1; // since prim=1
const RS_GF_POLY: u32 = 0x187;
const RS_A0: usize = RS_N; // sentinel for log(0) = -∞

struct RsDecoder {
    alpha_to: [u8; 256],
    index_of: [u8; 256],
}

#[allow(clippy::needless_range_loop)]
impl RsDecoder {
    fn new() -> Self {
        let mut alpha_to = [0u8; 256];
        let mut index_of = [RS_A0 as u8; 256]; // default sentinel
        let mut sr: u32 = 1;
        for i in 0..RS_N {
            index_of[sr as usize] = i as u8;
            alpha_to[i] = sr as u8;
            sr <<= 1;
            if sr & 256 != 0 {
                sr ^= RS_GF_POLY & 0xFF;
            }
            sr &= 0xFF;
        }
        // alpha_to[255] = 0 (alpha^nn = alpha^0 mod ... actually 0 element)
        alpha_to[RS_A0] = 0;
        index_of[0] = RS_A0 as u8; // log(0) = -inf

        Self { alpha_to, index_of }
    }

    #[inline]
    fn modnn(x: usize) -> usize {
        let mut x = x;
        while x >= RS_N {
            x -= RS_N;
            x = (x >> 8) + (x & RS_N);
        }
        x
    }

    /// Decode and correct `data` (RS_N bytes: RS_K data + RS_NROOTS parity).
    /// `erasures` is a list of known erased positions.
    /// Returns the number of corrected symbols, or `None` on uncorrectable error.
    fn decode(&self, data: &mut [u8; RS_N], erasures: &[usize]) -> Option<usize> {
        let a = &self.alpha_to;
        let ix = &self.index_of;
        let no_eras = erasures.len();

        let modnn = |x: usize| Self::modnn(x);

        // Form syndromes: evaluate data(x) at roots of g(x).
        let mut s = [RS_A0 as u8; RS_NROOTS];
        for i in 0..RS_NROOTS {
            s[i] = data[0];
        }
        for j in 1..RS_N {
            for i in 0..RS_NROOTS {
                if s[i] == 0 {
                    s[i] = data[j];
                } else {
                    s[i] = data[j] ^ a[modnn(ix[s[i] as usize] as usize + (RS_FCR + i * RS_PRIM))];
                }
            }
        }

        // Convert to index form and check for any non-zero syndrome.
        let mut syn_error = false;
        for i in 0..RS_NROOTS {
            syn_error |= s[i] != 0;
            s[i] = ix[s[i] as usize];
        }
        if !syn_error {
            return Some(0);
        }

        let a0 = RS_A0;
        let mut lambda = [0u8; RS_NROOTS + 1];
        lambda[0] = 1;

        // Init lambda to erasure locator polynomial.
        if no_eras > 0 {
            lambda[1] = a[modnn(RS_PRIM * (RS_N - 1 - erasures[0]))];
            for i in 1..no_eras {
                let u = modnn(RS_PRIM * (RS_N - 1 - erasures[i]));
                // C: for (j = i+1; j > 0; j--) — iterate from i+1 down to 1
                for j in (1..=(i + 1)).rev() {
                    let tmp = ix[lambda[j - 1] as usize] as usize;
                    if tmp != a0 {
                        lambda[j] ^= a[modnn(u + tmp)];
                    }
                }
            }
        }

        let mut b: Vec<u8> = lambda.iter().map(|&x| ix[x as usize]).collect();

        // Berlekamp-Massey algorithm.
        let mut el = no_eras;
        let mut r = no_eras;
        while {
            r += 1;
            r <= RS_NROOTS
        } {
            // Compute discrepancy.
            let mut discr_r = 0u8;
            for i in 0..r {
                let li = lambda[i];
                let si = s[r - i - 1];
                if li != 0 && si != a0 as u8 {
                    discr_r ^= a[modnn(ix[li as usize] as usize + si as usize)];
                }
            }
            let discr_r_idx = ix[discr_r as usize] as usize;

            if discr_r_idx == a0 {
                b.insert(0, a0 as u8);
                b.truncate(RS_NROOTS + 1);
            } else {
                let mut t: Vec<u8> = vec![lambda[0]];
                for i in 0..RS_NROOTS {
                    let bi = b[i] as usize;
                    t.push(if bi != a0 {
                        lambda[i + 1] ^ a[modnn(discr_r_idx + bi)]
                    } else {
                        lambda[i + 1]
                    });
                }
                // C: if (2 * el <= r + no_eras - 1) — equivalent to 2*el+1 <= r+no_eras
                if 2 * el < r + no_eras {
                    el = r + no_eras - el;
                    for i in 0..=RS_NROOTS {
                        b[i] = if lambda[i] == 0 {
                            a0 as u8
                        } else {
                            modnn(ix[lambda[i] as usize] as usize + RS_N - discr_r_idx) as u8
                        };
                    }
                } else {
                    b.insert(0, a0 as u8);
                    b.truncate(RS_NROOTS + 1);
                }
                lambda.copy_from_slice(&t[..RS_NROOTS + 1]);
            }
        }

        // Convert lambda to index form and find degree.
        let mut deg_lambda = 0usize;
        let mut lambda_idx = [0usize; RS_NROOTS + 1];
        for i in 0..=RS_NROOTS {
            lambda_idx[i] = ix[lambda[i] as usize] as usize;
            if lambda_idx[i] != a0 {
                deg_lambda = i;
            }
        }

        // Chien search for roots.
        // C: for (i=1, k=IPRIM-1; i<=NN; i++, k=MODNN(k+IPRIM)) — k is updated AFTER body.
        let mut reg: Vec<usize> = lambda_idx[1..].to_vec();
        let mut root = [0usize; RS_NROOTS];
        let mut loc = [0usize; RS_NROOTS];
        let mut count = 0usize;
        let mut k = RS_IPRIM.wrapping_sub(1); // k starts at IPRIM-1 = 0
        for i in 1..=RS_N {
            // body uses k before updating
            let mut q = 1u8;
            for j in (1..=deg_lambda).rev() {
                if reg[j - 1] != a0 {
                    reg[j - 1] = modnn(reg[j - 1] + j);
                    q ^= a[reg[j - 1]];
                }
            }
            if q == 0 {
                root[count] = i;
                loc[count] = k;
                count += 1;
                if count == deg_lambda {
                    break;
                }
            }
            // Update k after the body so the loop state matches the symbol-search ordering.
            k = modnn(k + RS_IPRIM);
        }
        if deg_lambda != count {
            return None; // uncorrectable
        }

        // Compute omega (error evaluator polynomial).
        let deg_omega = deg_lambda - 1;
        let mut omega = [a0; RS_NROOTS + 1];
        for i in 0..=deg_omega {
            let mut tmp = 0u8;
            for j in (0..=i).rev() {
                let si = s[i - j];
                let lj = lambda_idx[j];
                if si != a0 as u8 && lj != a0 {
                    tmp ^= a[modnn(si as usize + lj)];
                }
            }
            omega[i] = ix[tmp as usize] as usize;
        }

        // Forney algorithm: compute error values.
        for j in (0..count).rev() {
            let mut num1 = 0u8;
            for i in (0..=deg_omega).rev() {
                if omega[i] != a0 {
                    num1 ^= a[modnn(omega[i] + i * root[j])];
                }
            }
            let num2 = a[modnn(root[j] * (RS_FCR.wrapping_sub(1)) + RS_N)];
            let mut den = 0u8;
            let lim = (deg_lambda.min(RS_NROOTS - 1)) & !1;
            let mut ii = lim as isize;
            while ii >= 0 {
                if lambda_idx[ii as usize + 1] != a0 {
                    den ^= a[modnn(lambda_idx[ii as usize + 1] + ii as usize * root[j])];
                }
                ii -= 2;
            }
            if num1 != 0 {
                data[loc[j]] ^= a[modnn(
                    ix[num1 as usize] as usize + ix[num2 as usize] as usize + RS_N
                        - ix[den as usize] as usize,
                )];
            }
        }
        Some(count)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DemodState {
    Init,
    Sync,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DecState {
    Idle,
    Header,
    Data,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Vdl2Event {
    SyncFound {
        sample_index: u64,
        seconds_into_recording: f64,
    },
    DeinterleaveDataError {
        sample_index: u64,
        seconds_into_recording: f64,
        datalen_bits: u32,
    },
    DeinterleaveFecError {
        sample_index: u64,
        seconds_into_recording: f64,
        datalen_bits: u32,
    },
    RsDecodeError {
        sample_index: u64,
        seconds_into_recording: f64,
        datalen_bits: u32,
        block: usize,
    },
    BurstDecoded {
        sample_index: u64,
        seconds_into_recording: f64,
        datalen_bits: u32,
        datalen_octets: u32,
        num_blocks: u32,
        raw_frames: usize,
    },
}

/// A decoded AVLC frame together with physical-layer metadata.
pub struct DemodFrame {
    /// Raw frame bytes (including 2-byte FCS).
    pub bytes: Vec<u8>,
    /// Average signal power in dBFS.
    pub signal_dbfs: f32,
    /// Estimated noise floor in dBFS.
    pub noise_dbfs: f32,
    /// Frequency error in parts-per-million.
    pub ppm_error: f32,
}

struct DemodTrace {
    writer: BufWriter<File>,
    window_start_sec: Option<f64>,
    window_end_sec: Option<f64>,
}

/// Per-channel VDL2 demodulator state.
pub struct Vdl2Channel {
    // Lowpass filters
    lpf_re: Chebyshev2Lpf,
    lpf_im: Chebyshev2Lpf,

    // Optional downmix NCO
    downmix_nco: Option<Nco>,

    // Decimation
    decimate: EveryN,

    // Phase circular buffer for frame sync
    syncbuf: Vec<f32>,
    syncbuf_idx: usize,
    sclk: i32,
    pherr: [f32; 3],
    prev_dphi: f32,
    prev_phi: f32,
    dphi: f32,

    // Noise-floor estimation
    mag_lp: f32,
    mag_nf: f32,
    nfcnt: u32,

    // Frame power
    frame_pwr: f32,
    frame_pwr_cnt: u32,

    // State machine
    demod_state: DemodState,
    dec_state: DecState,

    // Bit accumulation (one u8 per bit, value 0 or 1)
    bs: Vec<u8>,
    bs_descrambler_pos: usize,
    requested_bits: u32,

    // Burst parameters
    lfsr: u16,
    datalen: u32,
    datalen_octets: u32,
    num_blocks: u32,
    fec_octets: u32,
    last_block_len_octets: u32,

    // Pre-computed linear regression constants for preamble sync
    lr_x: [f32; PREAMBLE_SYMS],
    lr_denom: f32,

    // RS decoder (shared, initialised once)
    rs: RsDecoder,

    // Channel frequency (Hz) — used for ppm calculation.
    freq_hz: f32,
    sample_rate_hz: f32,
    sample_index: u64,
    trace: Option<DemodTrace>,
    sync_threshold: f32,
}

#[allow(clippy::needless_range_loop)]
impl Vdl2Channel {
    /// Create a new channel demodulator.
    ///
    /// * `sample_rate` — input sample rate in Hz (e.g. 1 050 000).
    /// * `offset_hz`   — frequency offset = channel_freq − center_freq.
    /// * `freq_hz`     — absolute channel frequency in Hz (used for ppm computation).
    pub fn new(sample_rate: f32, offset_hz: f32, freq_hz: f32) -> Self {
        let lpf_re = Chebyshev2Lpf::new(INP_LPF_CUTOFF_HZ / sample_rate, INP_LPF_RIPPLE);
        let lpf_im = Chebyshev2Lpf::new(INP_LPF_CUTOFF_HZ / sample_rate, INP_LPF_RIPPLE);

        let oversample = (sample_rate / (SYMBOL_RATE * SPS) as f32).round() as u32;
        let decimate = EveryN::new(oversample);
        let downmix_nco = if offset_hz.abs() > 1.0 {
            Some(Nco::new(offset_hz as f64, sample_rate as f64))
        } else {
            None
        };

        // Pre-compute linear regression X values and denominator.
        let mut lr_x = [0.0f32; PREAMBLE_SYMS];
        let mut mean_x = 0.0f32;
        for i in 0..PREAMBLE_SYMS {
            mean_x += i as f32;
        }
        mean_x /= PREAMBLE_SYMS as f32;
        let mut lr_denom = 0.0f32;
        for i in 0..PREAMBLE_SYMS {
            lr_x[i] = i as f32 - mean_x;
            lr_denom += lr_x[i] * lr_x[i];
        }

        Self {
            lpf_re,
            lpf_im,
            downmix_nco,
            decimate,
            syncbuf: vec![0.0; SYNC_BUFLEN],
            syncbuf_idx: 0,
            sclk: 0,
            pherr: [1000.0; 3],
            prev_dphi: 0.0,
            prev_phi: 0.0,
            dphi: 0.0,
            mag_lp: 0.0,
            mag_nf: 0.0001,
            nfcnt: 0,
            frame_pwr: 0.0,
            frame_pwr_cnt: 0,
            demod_state: DemodState::Init,
            dec_state: DecState::Idle,
            bs: Vec::with_capacity(32768),
            bs_descrambler_pos: 0,
            requested_bits: HEADER_LEN,
            lfsr: LFSR_IV,
            datalen: 0,
            datalen_octets: 0,
            num_blocks: 0,
            fec_octets: 0,
            last_block_len_octets: 0,
            lr_x,
            lr_denom,
            rs: RsDecoder::new(),
            freq_hz,
            sample_rate_hz: sample_rate,
            sample_index: 0,
            trace: None,
            sync_threshold: DEFAULT_SYNC_THRESHOLD,
        }
    }

    pub fn set_sync_threshold(&mut self, threshold: f32) {
        self.sync_threshold = threshold;
    }

    pub fn enable_trace(
        &mut self,
        path: &str,
        window_start_sec: Option<f64>,
        window_end_sec: Option<f64>,
    ) -> std::io::Result<()> {
        self.trace = Some(DemodTrace {
            writer: BufWriter::new(File::create(path)?),
            window_start_sec,
            window_end_sec,
        });
        Ok(())
    }

    /// Feed one raw I/Q sample.
    ///
    /// Returns a list of decoded frames (possibly empty).
    pub fn process_sample(&mut self, mut re: f32, mut im: f32) -> Vec<DemodFrame> {
        self.sample_index = self.sample_index.saturating_add(1);
        // Optional downmix.
        if let Some(nco) = self.downmix_nco.as_mut() {
            let (re2, im2) = nco.mix_down_complex(re, im);
            re = re2;
            im = im2;
            nco.step();
        }

        // Chebyshev LPF (direct form I, 2-pole).
        let lp_re = self.lpf_re.step(re);
        let lp_im = self.lpf_im.step(im);

        // Decimation.
        if !self.decimate.keep() {
            return Vec::new();
        }

        self.demod(lp_re, lp_im)
    }

    /// Process one decimated sample through the demodulator state machine.
    fn demod(&mut self, re: f32, im: f32) -> Vec<DemodFrame> {
        if self.dec_state == DecState::Idle {
            self.decoder_reset();
        }

        match self.demod_state {
            DemodState::Init => {
                // Update phase ring buffer.
                self.syncbuf_idx = (self.syncbuf_idx + 1) % SYNC_BUFLEN;
                self.syncbuf[self.syncbuf_idx] = im.atan2(re);

                self.sclk += 1;
                if self.sclk < SYNC_SKIP as i32 {
                    return Vec::new();
                }
                self.sclk = 0;

                // Update noise-floor estimate.
                let mag = (re * re + im * im).sqrt();
                self.mag_lp = self.mag_lp * MAG_LP + mag * (1.0 - MAG_LP);
                self.nfcnt += 1;
                if self.nfcnt == 1000 {
                    self.nfcnt = 0;
                    self.mag_nf =
                        NF_LP * self.mag_nf + (1.0 - NF_LP) * self.mag_lp.min(self.mag_nf) + 0.0001;
                }

                if self.got_sync() {
                    self.trace_event(Vdl2Event::SyncFound {
                        sample_index: self.sample_index,
                        seconds_into_recording: self.seconds_into_recording(),
                    });
                    self.demod_state = DemodState::Sync;
                }
                Vec::new()
            }

            DemodState::Sync => {
                self.sclk += 1;
                if self.sclk < SPS as i32 {
                    return Vec::new();
                }
                self.sclk = 0;

                let phi = im.atan2(re);
                let mut dphi = phi - self.prev_phi - self.dphi;
                // Wrap to [0, 2π).
                while dphi < 0.0 {
                    dphi += 2.0 * PI;
                }
                while dphi >= 2.0 * PI {
                    dphi -= 2.0 * PI;
                }
                dphi /= PI / 4.0;
                let idx = (dphi.round() as usize) % ARITY;
                let bits = GRAYCODE[idx];

                // Update frame power.
                let sym_pwr = re * re + im * im;
                self.frame_pwr = (self.frame_pwr * self.frame_pwr_cnt as f32 + sym_pwr)
                    / (self.frame_pwr_cnt + 1) as f32;
                self.frame_pwr_cnt += 1;

                self.prev_phi = phi;

                // Append 3 bits MSB-first to the bit stream.
                for j in (0..BPS as i32).rev() {
                    self.bs.push((bits >> j) & 1);
                }

                if self.bs.len() >= self.requested_bits as usize {
                    return self.decode_burst();
                }
                Vec::new()
            }
        }
    }

    /// Check whether the last SYNC_BUFLEN phases match the VDL2 preamble.
    /// Returns true and sets dphi/sclk if a valid preamble is found.
    fn got_sync(&mut self) -> bool {
        let buf = &self.syncbuf;
        let idx = self.syncbuf_idx;

        // Build error vector: measured phase − expected preamble phase.
        let mut errvec = [0.0f32; PREAMBLE_SYMS];
        let mut unwrap = 0.0f32;
        errvec[0] = buf[(idx + SPS as usize) % SYNC_BUFLEN] - PR_PHASE[0];
        let mut errvec_mean = errvec[0];
        let mut prev_err = errvec[0];

        for i in 1..PREAMBLE_SYMS {
            let cur_err = buf[(idx + (i + 1) * SPS as usize) % SYNC_BUFLEN] - PR_PHASE[i];
            let errdiff = cur_err - prev_err;
            prev_err = cur_err;
            if errdiff > PI {
                unwrap -= 2.0 * PI;
            } else if errdiff < -PI {
                unwrap += 2.0 * PI;
            }
            errvec[i] = cur_err + unwrap;
            errvec_mean += errvec[i];
        }
        errvec_mean /= PREAMBLE_SYMS as f32;
        for e in &mut errvec {
            *e -= errvec_mean;
        }

        // Linear regression: estimate frequency offset.
        let mut freq_err = 0.0f32;
        for i in 0..PREAMBLE_SYMS {
            freq_err += self.lr_x[i] * errvec[i];
        }
        freq_err /= self.lr_denom;

        // Compute residual phase error squared.
        let mut pherr0 = 0.0f32;
        for i in 0..PREAMBLE_SYMS {
            let e = errvec[i] - freq_err * self.lr_x[i];
            pherr0 += e * e;
        }

        if self.pherr[1] < self.sync_threshold && pherr0 > self.pherr[1] {
            // Preamble found. Use parabolic interpolation on the three pherr samples
            // to find the sub-sample timing of the pherr minimum (= sync point).
            // Called with x=0 (sclk was reset to 0 before got_sync), d=SYNC_SKIP.
            let vertex_x =
                calc_para_vertex(0.0, SYNC_SKIP as i32, self.pherr[2], self.pherr[1], pherr0);
            self.sclk = (-vertex_x).round() as i32;
            self.dphi = self.prev_dphi;
            // prev_phi = phase at the sync point (sclk samples back from now).
            let sp = (self.syncbuf_idx as isize - self.sclk as isize)
                .rem_euclid(SYNC_BUFLEN as isize) as usize;
            self.prev_phi = self.syncbuf[sp];
            self.pherr = [1000.0; 3];
            return true;
        }

        self.pherr[2] = self.pherr[1];
        self.pherr[1] = pherr0;
        self.prev_dphi = freq_err;
        false
    }

    fn decoder_reset(&mut self) {
        self.dec_state = DecState::Header;
        self.requested_bits = HEADER_LEN;
        self.bs.clear();
        self.bs_descrambler_pos = 0;
        self.lfsr = LFSR_IV;
        self.frame_pwr = 0.0;
        self.frame_pwr_cnt = 0;
    }

    fn demod_reset(&mut self) {
        self.decoder_reset();
        self.sclk = 0;
        self.demod_state = DemodState::Init;
        self.pherr = [1000.0; 3];
    }

    /// Decode the accumulated bit stream.  Called when `requested_bits` bits
    /// have been collected.  Returns any complete decoded frames.
    fn decode_burst(&mut self) -> Vec<DemodFrame> {
        match self.dec_state {
            DecState::Header => {
                // Descramble with LFSR.
                self.lfsr_descramble();
                // Read HEADER_LEN bits as a word, MSB first.
                let header = self.bs_read_word_msbfirst(HEADER_LEN as usize);
                let Some(mut header) = header else {
                    self.demod_reset();
                    return Vec::new();
                };

                // Force reserved symbol bits to 0 (improve FEC chance).
                header &= (1 << (TRLEN + HDRFECLEN)) - 1;

                // FEC correction (syndrome decode).
                let syndrome = self.decode_header(&mut header);
                // Check reserved bits are still zero.
                if header & ((1 << (TRLEN + HDRFECLEN)) - 1) != header {
                    self.demod_reset();
                    return Vec::new();
                }

                header >>= HDRFECLEN;
                let trlen_mask = (1u32 << TRLEN) - 1;
                self.datalen = reverse_bits_u32(header & trlen_mask, TRLEN);

                const MAX_FRAME_LENGTH: u32 = 0x3FFF;
                const MAX_FRAME_LENGTH_CORRECTED: u32 = 0x1FFF;
                if (syndrome != 0 && self.datalen > MAX_FRAME_LENGTH_CORRECTED)
                    || self.datalen > MAX_FRAME_LENGTH
                {
                    self.demod_reset();
                    return Vec::new();
                }

                self.datalen_octets = self.datalen.div_ceil(8);
                self.num_blocks = self.datalen_octets / RS_K as u32;
                self.fec_octets = self.num_blocks * RS_NROOTS as u32;
                self.last_block_len_octets = self.datalen_octets % RS_K as u32;
                if self.last_block_len_octets != 0 {
                    self.num_blocks += 1;
                }
                self.fec_octets += get_fec_octetcount(self.last_block_len_octets) as u32;

                if self.fec_octets == 0 {
                    self.demod_reset();
                    return Vec::new();
                }

                self.requested_bits = HEADER_LEN + 8 * (self.datalen_octets + self.fec_octets);
                self.dec_state = DecState::Data;
                Vec::new()
            }

            DecState::Data => {
                // Descramble continuation.
                self.lfsr_descramble();

                let datalen_oct = self.datalen_octets as usize;
                let fec_oct = self.fec_octets as usize;

                let data_start = HEADER_LEN as usize;
                let fec_start = data_start + datalen_oct * 8;
                let Some(data_bytes) = self.bs_read_bytes_lsbfirst(data_start, datalen_oct) else {
                    self.demod_reset();
                    return Vec::new();
                };
                let Some(fec_bytes) = self.bs_read_bytes_lsbfirst(fec_start, fec_oct) else {
                    self.demod_reset();
                    return Vec::new();
                };

                // Deinterleave data+fec into RS blocks.
                let nb = self.num_blocks as usize;
                let last_len = self.last_block_len_octets as usize;

                let mut rs_tab = vec![[0u8; RS_N]; nb];
                if deinterleave(&data_bytes, nb, RS_N, &mut rs_tab, RS_K, 0).is_err() {
                    self.trace_event(Vdl2Event::DeinterleaveDataError {
                        sample_index: self.sample_index,
                        seconds_into_recording: self.seconds_into_recording(),
                        datalen_bits: self.datalen,
                    });
                    self.demod_reset();
                    return Vec::new();
                }
                let fec_rows = if get_fec_octetcount(last_len as u32) == 0 {
                    nb - 1
                } else {
                    nb
                };
                if deinterleave(&fec_bytes, fec_rows, RS_N, &mut rs_tab, RS_NROOTS, RS_K).is_err() {
                    self.trace_event(Vdl2Event::DeinterleaveFecError {
                        sample_index: self.sample_index,
                        seconds_into_recording: self.seconds_into_recording(),
                        datalen_bits: self.datalen,
                    });
                    self.demod_reset();
                    return Vec::new();
                }

                // RS verify and correct each block.
                let mut corrected_data: Vec<u8> = Vec::with_capacity(datalen_oct);
                for r in 0..nb {
                    let n_fec = if r != nb - 1 {
                        RS_NROOTS
                    } else {
                        get_fec_octetcount(last_len as u32)
                    };
                    let erasure_cnt = RS_NROOTS - n_fec;
                    let erasures: Vec<usize> = (RS_K + n_fec..RS_N).take(erasure_cnt).collect();

                    if self.rs.decode(&mut rs_tab[r], &erasures).is_none() {
                        self.trace_event(Vdl2Event::RsDecodeError {
                            sample_index: self.sample_index,
                            seconds_into_recording: self.seconds_into_recording(),
                            datalen_bits: self.datalen,
                            block: r,
                        });
                        self.demod_reset();
                        return Vec::new();
                    }
                    let take = if r != nb - 1 { RS_K } else { last_len.max(1) };
                    corrected_data.extend_from_slice(&rs_tab[r][..take]);
                }

                // Truncate to datalen bits.
                if (self.datalen as usize) < corrected_data.len() * 8 {
                    corrected_data.truncate((self.datalen as usize).div_ceil(8));
                }

                // Build corrected bit stream for HDLC destuffing.
                let mut bit_stream: Vec<u8> = Vec::with_capacity(corrected_data.len() * 8);
                for byte in &corrected_data {
                    for j in 0..8 {
                        bit_stream.push((byte >> j) & 1);
                    }
                }
                if (self.datalen as usize) < bit_stream.len() {
                    bit_stream.truncate(self.datalen as usize);
                }

                // HDLC bit-destuffing: extract one or more AVLC frames.
                let raw_frames = hdlc_destuff(&bit_stream);
                self.trace_event(Vdl2Event::BurstDecoded {
                    sample_index: self.sample_index,
                    seconds_into_recording: self.seconds_into_recording(),
                    datalen_bits: self.datalen,
                    datalen_octets: self.datalen_octets,
                    num_blocks: self.num_blocks,
                    raw_frames: raw_frames.len(),
                });

                // Capture signal metadata before reset clears frame_pwr.
                let signal_dbfs = 10.0 * self.frame_pwr.max(1e-20_f32).log10();
                let noise_dbfs = 20.0 * (self.mag_nf + 0.001_f32).log10();
                let ppm_error =
                    self.dphi * SYMBOL_RATE as f32 / (2.0 * PI * self.freq_hz) * 1_000_000.0;

                self.demod_reset();

                raw_frames
                    .into_iter()
                    .map(|bytes| DemodFrame {
                        bytes,
                        signal_dbfs,
                        noise_dbfs,
                        ppm_error,
                    })
                    .collect()
            }

            DecState::Idle => {
                self.demod_reset();
                Vec::new()
            }
        }
    }

    fn seconds_into_recording(&self) -> f64 {
        self.sample_index as f64 / self.sample_rate_hz as f64
    }

    fn trace_event(&mut self, event: Vdl2Event) {
        let sec = self.seconds_into_recording();
        if let Some(t) = self.trace.as_mut() {
            if let Some(s) = t.window_start_sec {
                if sec < s {
                    return;
                }
            }
            if let Some(e) = t.window_end_sec {
                if sec > e {
                    return;
                }
            }
            let _ = writeln!(t.writer, "{}", serde_json::to_string(&event).unwrap());
        }
    }

    /// LFSR descramble (x^15 + x + 1, initial value set before calling).
    fn lfsr_descramble(&mut self) {
        for i in self.bs_descrambler_pos..self.bs.len() {
            let bit = (self.lfsr ^ (self.lfsr >> 14)) & 1;
            self.lfsr = (self.lfsr >> 1) | (bit << 14);
            self.bs[i] ^= bit as u8;
        }
        self.bs_descrambler_pos = self.bs.len();
    }

    /// Header FEC: apply parity-check matrix, look up syndrome correction.
    /// Returns syndrome (0 = no error).
    fn decode_header(&self, r: &mut u32) -> u32 {
        let mut syndrome = 0u32;
        for (i, &h) in H.iter().enumerate() {
            let row = *r & h;
            syndrome |= parity(row) << (HDRFECLEN as usize - 1 - i);
        }
        *r ^= SYNDTABLE[syndrome as usize];
        syndrome
    }

    /// Read `nbits` from the start of `bs`, assembling MSB-first.
    fn bs_read_word_msbfirst(&self, nbits: usize) -> Option<u32> {
        if self.bs.len() < nbits {
            return None;
        }
        let mut word = 0u32;
        for i in 0..nbits {
            word |= (self.bs[i] as u32) << (nbits - 1 - i);
        }
        Some(word)
    }

    /// Read `n` bytes starting at bit offset `start_bit` in `bs`, LSB-first per byte.
    fn bs_read_bytes_lsbfirst(&self, start_bit: usize, n: usize) -> Option<Vec<u8>> {
        let end = start_bit + n * 8;
        if self.bs.len() < end {
            return None;
        }
        let mut out = vec![0u8; n];
        for (i, byte) in out.iter_mut().enumerate() {
            for j in 0..8 {
                *byte |= self.bs[start_bit + i * 8 + j] << j;
            }
        }
        Some(out)
    }
}

/// Fit a parabola through three equally-spaced points and return the x-coordinate
/// of its vertex.
///
/// Points are at x-coordinates `x - 2*d`, `x - d`, `x` with y-values `y1`, `y2`, `y3`.
fn calc_para_vertex(x: f32, d: i32, y1: f32, y2: f32, y3: f32) -> f32 {
    let d = d as f32;
    let denom = d * 2.0 * d * (-d);
    let a = (x * (y2 - y1) + (x - d) * (y1 - y3) + (x - 2.0 * d) * (y3 - y2)) / denom;
    let b = (x * x * (y1 - y2)
        + (x - d) * (x - d) * (y3 - y1)
        + (x - 2.0 * d) * (x - 2.0 * d) * (y2 - y3))
        / denom;
    -b / (2.0 * a)
}

/// Bit-reverse `v` using only the lowest `nbits` bits.
fn reverse_bits_u32(mut v: u32, nbits: u32) -> u32 {
    let mut r = v;
    let mut s: i32 = 31;
    v >>= 1;
    while v != 0 {
        r <<= 1;
        r |= v & 1;
        v >>= 1;
        s -= 1;
    }
    r <<= s;
    r >> (32 - nbits)
}

/// Bit parity (1 if odd number of set bits, 0 otherwise).
fn parity(mut v: u32) -> u32 {
    let mut p = 0u32;
    while v != 0 {
        p ^= 1;
        v &= v - 1;
    }
    p
}

/// Number of FEC octets for a block whose data portion is `len` octets.
fn get_fec_octetcount(len: u32) -> usize {
    match len {
        0..=2 => 0,
        3..=30 => 2,
        31..=67 => 4,
        _ => RS_NROOTS,
    }
}

/// Deinterleave `src` bytes into `rs_tab[row][offset..offset+fillwidth]`
/// column-by-column.
fn deinterleave(
    src: &[u8],
    rows: usize,
    cols: usize,
    rs_tab: &mut [[u8; RS_N]],
    fillwidth: usize,
    offset: usize,
) -> Result<(), ()> {
    if rows == 0 || cols == 0 || fillwidth == 0 {
        return Err(());
    }
    if fillwidth + offset > cols {
        return Err(());
    }
    let last_row_len_raw = src.len() % fillwidth;
    let last_row_len = if last_row_len_raw == 0 {
        fillwidth
    } else {
        last_row_len_raw
    };
    let mut row = 0usize;
    let mut col = offset;
    let last_row_end = last_row_len + offset;
    for (i, &b) in src.iter().enumerate() {
        if col >= cols {
            return Err(());
        }
        if row == rows - 1 && col >= last_row_end {
            rs_tab[row][col] = 0x00;
            row = 0;
            col += 1;
            if col >= cols {
                return Err(());
            }
        }
        rs_tab[row][col] = b;
        row += 1;
        if row == rows {
            row = 0;
            col += 1;
        }
        let _ = i;
    }
    Ok(())
}

/// HDLC bit-destuffing: remove stuffed zeros (after 5 consecutive ones) and
/// extract frames delimited by the flag sequence 0111_1110 (0x7E).
///
/// Returns a list of frame byte vectors (each without the flag bytes).
fn hdlc_destuff(bits: &[u8]) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    let mut start = 0usize;

    'outer: while start < bits.len() {
        let mut ones = 0u32;
        let mut frame_bits: Vec<u8> = Vec::new();
        let mut i = start;
        let mut found_start_flag = false;

        while i < bits.len() {
            let b = bits[i];
            i += 1;

            if b == 0 && ones == 5 {
                // Stuffed zero — discard.
                ones = 0;
                continue;
            } else if b == 1 {
                ones += 1;
                if ones > 6 {
                    start = i;
                    continue 'outer;
                }
            }

            frame_bits.push(b);

            if b == 0 {
                if ones == 6 {
                    // Flag sequence 0x7E.
                    if !found_start_flag {
                        // Opening flag; discard what we collected so far.
                        frame_bits.clear();
                        found_start_flag = true;
                    } else {
                        // Closing flag.
                        if frame_bits.len() > 8 {
                            let frame_bits_clean = &frame_bits[..frame_bits.len() - 8];
                            if frame_bits_clean.len().is_multiple_of(8) {
                                let bytes = bits_to_bytes_lsbfirst(frame_bits_clean);
                                if !bytes.is_empty() {
                                    frames.push(bytes);
                                }
                            }
                        }
                        start = i;
                        continue 'outer;
                    }
                }
                ones = 0;
            }
        }
        break;
    }
    frames
}

/// Convert a bit slice (LSB first per byte) to a byte vector.
fn bits_to_bytes_lsbfirst(bits: &[u8]) -> Vec<u8> {
    bits.chunks(8)
        .map(|chunk| {
            let mut b = 0u8;
            for (j, &bit) in chunk.iter().enumerate() {
                b |= bit << j;
            }
            b
        })
        .collect()
}
