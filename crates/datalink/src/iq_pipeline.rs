//! Shared I/Q sample-processing loop for channelized demodulators.
//!
//! `IqPipeline` owns the repeated mechanics common to VDL2, VHF ACARS, and
//! merged receivers: feed raw complex samples through a resampler, update a
//! recording timestamp, run every channel demodulator, and return decoded frame
//! candidates with a [`FrameContext`].

use acars::demod::resample::ResampleAdapter;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
/// Timing and channel metadata attached to a demodulated frame candidate.
pub(crate) struct FrameContext {
    /// Index of the demodulator/channel that produced the frame.
    pub channel_index: usize,
    /// Number of output-rate samples processed since the start of the run.
    pub sample_index: u64,
    /// Seconds elapsed from the beginning of the recording or live run.
    pub seconds_into_recording: f64,
    /// Best-effort Unix timestamp derived from the run start plus elapsed samples.
    pub timestamp_unix: f64,
}

/// Stateful adapter that feeds one resampled I/Q stream into many demodulators.
pub(crate) struct IqPipeline<'a, D> {
    adapter: &'a mut ResampleAdapter,
    demods: &'a mut [D],
    sample_index: &'a mut u64,
    sample_rate: u32,
    run_start: SystemTime,
}

impl<'a, D> IqPipeline<'a, D> {
    pub(crate) fn new(
        adapter: &'a mut ResampleAdapter,
        demods: &'a mut [D],
        sample_index: &'a mut u64,
        sample_rate: u32,
        run_start: SystemTime,
    ) -> Self {
        Self {
            adapter,
            demods,
            sample_index,
            sample_rate,
            run_start,
        }
    }
}

/// Feed one chunk of complex samples through the pipeline and collect emitted frames.
pub(crate) fn collect_iq_frames<D, Frame, Frames, Process>(
    pipeline: &mut IqPipeline<'_, D>,
    re: f32,
    im: f32,
    mut process: Process,
) -> anyhow::Result<Vec<(FrameContext, Frame)>>
where
    Process: FnMut(&mut D, f32, f32) -> anyhow::Result<Frames>,
    Frames: IntoIterator<Item = Frame>,
{
    let mut out = Vec::new();
    for sample in pipeline.adapter.feed(re, im) {
        *pipeline.sample_index = pipeline.sample_index.saturating_add(1);
        let seconds_into_recording = *pipeline.sample_index as f64 / pipeline.sample_rate as f64;
        let frame_ts = pipeline.run_start + Duration::from_secs_f64(seconds_into_recording);
        let timestamp_unix = frame_ts
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or_default();

        for (channel_index, demod) in pipeline.demods.iter_mut().enumerate() {
            for frame in process(demod, sample.re, sample.im)? {
                out.push((
                    FrameContext {
                        channel_index,
                        sample_index: *pipeline.sample_index,
                        seconds_into_recording,
                        timestamp_unix,
                    },
                    frame,
                ));
            }
        }
    }
    Ok(out)
}
