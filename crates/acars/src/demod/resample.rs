//! Transparent resampling for IQ sources whose native sample rate is not a
//! clean integer multiple of the demodulator's required decimated rate.
//!
//! Both the VDL2 and ACARS-131 demods compute their own integer decimation
//! factor from the sample rate passed to them. This module computes the
//! nearest valid target rate and wraps the sample stream in a
//! [`desperado::dsp::resampler::ComplexResampler`] when needed.
//!
//! A rate is "valid" if `input_rate / target_rate` is close enough to an
//! integer that the integer-rounding error is within `MAX_PPM` parts per
//! million.

use desperado::dsp::resampler::ComplexResampler;
use num_complex::Complex;

const MAX_PPM: f64 = 200.0; // tolerate up to 200 ppm rounding error before resampling

/// Return the rate to feed the demodulator, plus an optional resampler.
///
/// - `input_rate` — actual sample rate of the source (e.g. 1 800 000)
/// - `demod_target` — fundamental decimated rate of the demodulator
///   (e.g. 105 000 for VDL2 or 12 500 for ACARS-131)
///
/// If `input_rate` is already a valid integer multiple of `demod_target`
/// (within tolerance) both values returned are `input_rate` and `None`.
/// Otherwise a [`ComplexResampler`] is returned that converts to
/// `resampled_rate`.
pub fn maybe_resample(input_rate: u32, demod_target: u32) -> (u32, Option<ComplexResampler>) {
    let ratio = input_rate as f64 / demod_target as f64;
    let n = ratio.round() as u32;
    if n == 0 {
        // demod_target > input_rate — just pass through, demodulator handles it
        return (input_rate, None);
    }
    let valid_rate = n * demod_target;
    let ppm = ((input_rate as f64 - valid_rate as f64) / valid_rate as f64).abs() * 1e6;
    if ppm <= MAX_PPM {
        // Already clean enough — tell the demod the rounded rate
        return (valid_rate, None);
    }
    // Need to resample: pick the nearest integer multiple of demod_target
    let resampler = ComplexResampler::new(input_rate, valid_rate)
        .expect("ComplexResampler construction should not fail for non-zero rates");
    (valid_rate, Some(resampler))
}

/// Stateful resampler adapter that wraps the conversion for the per-sample
/// processing loop.  Call [`ResampleAdapter::feed`] with each raw IQ sample
/// and iterate over the returned output slice.
pub struct ResampleAdapter {
    resampler: Option<ComplexResampler>,
    buf: Vec<Complex<f32>>,
    scratch: [Complex<f32>; 1],
}

impl ResampleAdapter {
    /// Create from the result of [`maybe_resample`].
    pub fn new(resampler: Option<ComplexResampler>) -> Self {
        Self {
            resampler,
            buf: Vec::with_capacity(8),
            scratch: [Complex::new(0.0, 0.0)],
        }
    }

    /// Feed one raw sample, receive 0 or more output samples.
    ///
    /// Returns a slice into an internal buffer that is valid until the next
    /// call to `feed`. The caller must process the slice before calling `feed`
    /// again.
    #[inline]
    pub fn feed(&mut self, re: f32, im: f32) -> &[Complex<f32>] {
        match &mut self.resampler {
            None => {
                self.scratch[0] = Complex::new(re, im);
                &self.scratch
            }
            Some(r) => {
                self.scratch[0] = Complex::new(re, im);
                self.buf = r.process(&self.scratch);
                &self.buf
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vdl2_exact_rate_no_resample() {
        let (rate, rs) = maybe_resample(1_050_000, 105_000);
        assert_eq!(rate, 1_050_000);
        assert!(rs.is_none());
    }

    #[test]
    fn vdl2_gqrx_1_8m_needs_resample() {
        let (rate, rs) = maybe_resample(1_800_000, 105_000);
        // 1_800_000 / 105_000 = 17.14 → nearest = 17 → valid = 1_785_000
        assert_eq!(rate, 1_785_000);
        assert!(rs.is_some());
    }

    #[test]
    fn acars131_exact_rate_no_resample() {
        let (rate, rs) = maybe_resample(1_050_000, 12_500);
        assert_eq!(rate, 1_050_000);
        assert!(rs.is_none());
    }

    #[test]
    fn acars131_gqrx_1_8m_needs_resample() {
        let (rate, rs) = maybe_resample(1_800_000, 12_500);
        // 1_800_000 / 12_500 = 144 → exact, no resample needed
        assert_eq!(rate, 1_800_000);
        assert!(rs.is_none());
    }

    #[test]
    fn resample_adapter_passthrough() {
        let mut a = ResampleAdapter::new(None);
        let out = a.feed(1.0, 0.5);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].re, 1.0);
        assert_eq!(out[0].im, 0.5);
    }
}
