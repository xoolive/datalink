//! Classic VHF ACARS MSK demodulator.
//!
//! This module turns complex baseband samples into candidate ACARS frame bytes.
//! It performs filtering, timing recovery, frame synchronization, and byte
//! extraction for 2,400 bit/s plain ACARS channels. Frame parsing itself remains
//! in [`crate::decode::acars`].

use std::f32::consts::PI;

use desperado::dsp::chebyshev::Chebyshev2Lpf;
use desperado::dsp::nco::Nco;

const INTRATE: u32 = 12_500;
const FLEN: usize = (INTRATE as usize / 1_200) + 1;
const MFLTOVER: usize = 12;
const FLENO: usize = FLEN * MFLTOVER + 1;

const SYN: u8 = 0x16;
const SOH: u8 = 0x01;
const ETX: u8 = 0x83;
const ETB: u8 = 0x97;
const DLE: u8 = 0x7F;

const MAX_LEN: usize = 240;
const MAX_PERR: usize = 4;

const PLL_G: f32 = 38e-4;
const PLL_C: f32 = 0.52;

#[derive(Debug, Clone)]
pub struct DemodFrame {
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AcarsState {
    WsSyn,
    Syn2,
    Soh1,
    Txt,
    Crc1,
    Crc2,
    End,
}

struct AcarsFramer {
    state: AcarsState,
    outbits: u8,
    nbits: i32,
    txt: Vec<u8>,
    crc: [u8; 2],
    parity_err: usize,
}

impl AcarsFramer {
    fn new() -> Self {
        Self {
            state: AcarsState::WsSyn,
            outbits: 0,
            nbits: 1,
            txt: Vec::with_capacity(256),
            crc: [0; 2],
            parity_err: 0,
        }
    }

    fn reset(&mut self) {
        self.state = AcarsState::WsSyn;
        self.nbits = 1;
    }

    fn push_bit(&mut self, bit_is_one: bool, msk_s: &mut u32) -> Option<Vec<u8>> {
        self.outbits >>= 1;
        if bit_is_one {
            self.outbits |= 0x80;
        }
        self.nbits -= 1;
        if self.nbits <= 0 {
            return self.consume_byte(self.outbits, msk_s);
        }
        None
    }

    fn consume_byte(&mut self, r: u8, msk_s: &mut u32) -> Option<Vec<u8>> {
        match self.state {
            AcarsState::WsSyn => {
                if r == SYN {
                    self.state = AcarsState::Syn2;
                    self.nbits = 8;
                } else if r == !SYN {
                    *msk_s ^= 2;
                    self.state = AcarsState::Syn2;
                    self.nbits = 8;
                } else {
                    self.nbits = 1;
                }
                None
            }
            AcarsState::Syn2 => {
                if r == SYN {
                    self.state = AcarsState::Soh1;
                    self.nbits = 8;
                } else if r == !SYN {
                    *msk_s ^= 2;
                    self.nbits = 8;
                } else {
                    self.reset();
                }
                None
            }
            AcarsState::Soh1 => {
                if r == SOH {
                    self.state = AcarsState::Txt;
                    self.txt.clear();
                    self.parity_err = 0;
                    self.nbits = 8;
                } else {
                    self.reset();
                }
                None
            }
            AcarsState::Txt => {
                self.txt.push(r);
                if (r.count_ones() & 1) == 0 {
                    self.parity_err += 1;
                    if self.parity_err > MAX_PERR {
                        self.reset();
                        return None;
                    }
                }

                if r == ETX || r == ETB {
                    self.state = AcarsState::Crc1;
                    self.nbits = 8;
                    return None;
                }

                if self.txt.len() > 20 && r == DLE {
                    if self.txt.len() >= 3 {
                        let len = self.txt.len() - 3;
                        self.crc[0] = self.txt[len];
                        self.crc[1] = self.txt[len + 1];
                        self.txt.truncate(len);
                        self.state = AcarsState::Crc2;
                        return self.finalize_frame();
                    }
                    self.reset();
                    return None;
                }

                if self.txt.len() > MAX_LEN {
                    self.reset();
                    return None;
                }

                self.nbits = 8;
                None
            }
            AcarsState::Crc1 => {
                self.crc[0] = r;
                self.state = AcarsState::Crc2;
                self.nbits = 8;
                None
            }
            AcarsState::Crc2 => {
                self.crc[1] = r;
                self.finalize_frame()
            }
            AcarsState::End => {
                self.reset();
                self.nbits = 8;
                None
            }
        }
    }

    fn finalize_frame(&mut self) -> Option<Vec<u8>> {
        if self.txt.len() < 13 {
            self.reset();
            return None;
        }
        let mut bytes = Vec::with_capacity(self.txt.len() + 3);
        bytes.extend_from_slice(&self.txt);
        bytes.push(self.crc[0]);
        bytes.push(self.crc[1]);
        bytes.push(DLE);
        self.state = AcarsState::End;
        self.nbits = 8;
        Some(bytes)
    }
}

struct MskDemod {
    h: [f32; FLENO],
    inb_re: [f32; FLEN],
    inb_im: [f32; FLEN],
    idx: usize,
    phi: f32,
    clk: f32,
    df: f32,
    s: u32,
    framer: AcarsFramer,
}

impl MskDemod {
    fn new() -> Self {
        let mut h = [0.0f32; FLENO];
        for (i, v) in h.iter_mut().enumerate() {
            let x = 2.0 * PI * 600.0 / INTRATE as f32 / MFLTOVER as f32
                * (i as f32 - (FLENO as f32 - 1.0) / 2.0);
            *v = x.cos().max(0.0);
        }

        Self {
            h,
            inb_re: [0.0; FLEN],
            inb_im: [0.0; FLEN],
            idx: 0,
            phi: 0.0,
            clk: 0.0,
            df: 0.0,
            s: 0,
            framer: AcarsFramer::new(),
        }
    }

    fn process_amp(&mut self, amp: f32) -> Vec<Vec<u8>> {
        let mut out = Vec::new();

        let step = 1_800.0 / INTRATE as f32 * 2.0 * PI + self.df;
        self.phi += step;
        if self.phi >= 2.0 * PI {
            self.phi -= 2.0 * PI;
        }

        let (sp, cp) = self.phi.sin_cos();
        self.inb_re[self.idx] = amp * cp;
        self.inb_im[self.idx] = -amp * sp;
        self.idx = (self.idx + 1) % FLEN;

        self.clk += step;
        if self.clk < 3.0 * PI / 2.0 - step / 2.0 {
            return out;
        }
        self.clk -= 3.0 * PI / 2.0;

        let mut o = (MFLTOVER as f32 * (self.clk / step + 0.5)) as usize;
        if o > MFLTOVER {
            o = MFLTOVER;
        }

        let mut v_re = 0.0f32;
        let mut v_im = 0.0f32;
        for j in 0..FLEN {
            let k = (j + self.idx) % FLEN;
            let w = self.h[o + j * MFLTOVER];
            v_re += w * self.inb_re[k];
            v_im += w * self.inb_im[k];
        }

        let lvl = (v_re * v_re + v_im * v_im).sqrt();
        let inv = 1.0 / (lvl + 1e-8);
        v_re *= inv;
        v_im *= inv;

        let (vo, dphi) = if (self.s & 1) != 0 {
            let vo = v_im;
            let dphi = if vo >= 0.0 { -v_re } else { v_re };
            (vo, dphi)
        } else {
            let vo = v_re;
            let dphi = if vo >= 0.0 { v_im } else { -v_im };
            (vo, dphi)
        };

        let bit_metric = if (self.s & 2) != 0 { -vo } else { vo };
        self.s = self.s.wrapping_add(1);

        self.df = PLL_C * self.df + (1.0 - PLL_C) * PLL_G * dphi;

        if let Some(frame) = self.framer.push_bit(bit_metric > 0.0, &mut self.s) {
            out.push(frame);
        }

        out
    }
}

/// Per-channel classic ACARS (131 MHz family) demodulator state.
pub struct VhfChannel {
    lpf_re: Chebyshev2Lpf,
    lpf_im: Chebyshev2Lpf,
    downmix_nco: Option<Nco>,
    decim_factor: u32,
    decim_count: u32,
    acc_re: f32,
    acc_im: f32,
    msk: MskDemod,
}

impl VhfChannel {
    pub fn new(sample_rate: f32, offset_hz: f32) -> Self {
        let downmix_nco = if offset_hz.abs() > 1.0 {
            Some(Nco::new(offset_hz as f64, sample_rate as f64))
        } else {
            None
        };

        let cutoff_norm = 6_000.0 / sample_rate;
        let lpf_re = Chebyshev2Lpf::new(cutoff_norm, 0.5);
        let lpf_im = Chebyshev2Lpf::new(cutoff_norm, 0.5);
        let decim_factor = (sample_rate / INTRATE as f32).round().max(1.0) as u32;

        Self {
            lpf_re,
            lpf_im,
            downmix_nco,
            decim_factor,
            decim_count: 0,
            acc_re: 0.0,
            acc_im: 0.0,
            msk: MskDemod::new(),
        }
    }

    pub fn process_sample(&mut self, mut re: f32, mut im: f32) -> Vec<DemodFrame> {
        if let Some(nco) = self.downmix_nco.as_mut() {
            let (re2, im2) = nco.mix_down_complex(re, im);
            re = re2;
            im = im2;
            nco.step();
        }

        re = self.lpf_re.step(re);
        im = self.lpf_im.step(im);

        self.acc_re += re;
        self.acc_im += im;
        self.decim_count += 1;
        if self.decim_count < self.decim_factor {
            return Vec::new();
        }

        let inv = 1.0 / self.decim_count as f32;
        let re_avg = self.acc_re * inv;
        let im_avg = self.acc_im * inv;
        self.acc_re = 0.0;
        self.acc_im = 0.0;
        self.decim_count = 0;

        let amp = (re_avg * re_avg + im_avg * im_avg).sqrt();
        self.msk
            .process_amp(amp)
            .into_iter()
            .map(|bytes| DemodFrame { bytes })
            .collect()
    }

    pub fn process_sample_with_dm(
        &mut self,
        mut re: f32,
        mut im: f32,
    ) -> (Vec<DemodFrame>, Option<f32>) {
        if let Some(nco) = self.downmix_nco.as_mut() {
            let (re2, im2) = nco.mix_down_complex(re, im);
            re = re2;
            im = im2;
            nco.step();
        }

        re = self.lpf_re.step(re);
        im = self.lpf_im.step(im);

        self.acc_re += re;
        self.acc_im += im;
        self.decim_count += 1;
        if self.decim_count < self.decim_factor {
            return (Vec::new(), None);
        }

        let inv = 1.0 / self.decim_count as f32;
        let re_avg = self.acc_re * inv;
        let im_avg = self.acc_im * inv;
        self.acc_re = 0.0;
        self.acc_im = 0.0;
        self.decim_count = 0;

        let amp = (re_avg * re_avg + im_avg * im_avg).sqrt();
        let frames = self
            .msk
            .process_amp(amp)
            .into_iter()
            .map(|bytes| DemodFrame { bytes })
            .collect();
        (frames, Some(amp))
    }
}
