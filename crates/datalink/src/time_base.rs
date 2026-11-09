//! Time base for frames demodulated from I/Q sources.
//!
//! - **File and stdin input** is only given an absolute timestamp when the
//!   capture start is known, either from an explicit `start_time` or from a
//!   capture filename that encodes it (GQRX, SDRuno). Otherwise frames carry
//!   only their offset into the recording, so processing time is never
//!   mistaken for reception time.
//! - **Live SDR input** is timestamped from the wall clock. Each received chunk
//!   is anchored to its arrival time, and the sample count is only used within
//!   a chunk. Samples lost between chunks (when decoding cannot keep up with
//!   the sample rate) therefore no longer make timestamps fall behind real
//!   time; [`TimeBase::lag_warning`] reports them instead.

use crate::event::TimeSource;
use crate::source::{Address, Source};
use std::time::{SystemTime, UNIX_EPOCH};

/// Absolute reference that sample offsets are resolved against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TimeBase {
    kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// No absolute time: offsets into the recording only.
    OffsetOnly,
    /// Known capture start (Unix seconds).
    Capture { start: f64 },
    /// Live input anchored to the wall clock.
    Wallclock(Live),
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Live {
    /// Wall-clock time of the first received sample (Unix seconds).
    run_start: Option<f64>,
    /// Seconds of samples received so far, by sample count.
    received: f64,
    /// Arrival time of the latest chunk and the sample-count offset at its end.
    anchor: Option<(f64, f64)>,
    /// Lag (seconds) at which the next warning is due.
    next_warning: f64,
}

/// Lag behind real time that triggers the first live-input warning, in seconds.
const FIRST_LAG_WARNING: f64 = 2.0;

pub(crate) fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default()
}

impl TimeBase {
    /// Wall-clock time base for live SDR input. It is anchored by
    /// [`TimeBase::on_chunk`] as samples arrive.
    pub(crate) fn wallclock() -> Self {
        Self {
            kind: Kind::Wallclock(Live {
                run_start: None,
                received: 0.0,
                anchor: None,
                next_warning: FIRST_LAG_WARNING,
            }),
        }
    }

    /// Time base anchored at a known capture start.
    pub(crate) fn capture(start_unix: f64) -> Self {
        Self {
            kind: Kind::Capture { start: start_unix },
        }
    }

    /// No absolute time: frames only carry their offset into the recording.
    pub(crate) fn offset_only() -> Self {
        Self {
            kind: Kind::OffsetOnly,
        }
    }

    /// Select the time base for a source: explicit `start_time`, then the
    /// capture filename for files, wall clock for SDRs, offset-only otherwise.
    pub(crate) fn for_source(src: &Source) -> anyhow::Result<Self> {
        if let Some(value) = src.start_time.as_deref() {
            return Ok(Self::capture(parse_start_time(value)?));
        }
        Ok(match &src.address {
            Address::File { file } if file == "-" => Self::offset_only(),
            Address::File { file } => crate::util::infer_capture_start(file)
                .map(Self::capture)
                .unwrap_or_else(Self::offset_only),
            #[allow(unreachable_patterns)]
            _ => Self::wallclock(),
        })
    }

    /// Record a received chunk of `samples` samples at `sample_rate`. Live time
    /// bases anchor to the current wall clock; others ignore it.
    pub(crate) fn on_chunk(&mut self, samples: usize, sample_rate: u32) {
        self.on_chunk_at(samples, sample_rate, unix_now());
    }

    fn on_chunk_at(&mut self, samples: usize, sample_rate: u32, now: f64) {
        if let Kind::Wallclock(live) = &mut self.kind {
            let duration = samples as f64 / f64::from(sample_rate.max(1));
            live.run_start.get_or_insert(now - duration);
            live.received += duration;
            live.anchor = Some((now, live.received));
        }
    }

    /// How far the received samples are behind real time, for live input.
    pub(crate) fn lag(&self) -> Option<f64> {
        match &self.kind {
            Kind::Wallclock(Live {
                run_start: Some(start),
                anchor: Some((now, _)),
                received,
                ..
            }) => Some((now - start - received).max(0.0)),
            _ => None,
        }
    }

    /// Return the current lag when it crosses the next warning threshold
    /// (2 s, then doubling), so callers can warn without flooding stderr.
    pub(crate) fn lag_warning(&mut self) -> Option<f64> {
        let lag = self.lag()?;
        let Kind::Wallclock(live) = &mut self.kind else {
            return None;
        };
        if lag < live.next_warning {
            return None;
        }
        while live.next_warning <= lag {
            live.next_warning *= 2.0;
        }
        Some(lag)
    }

    /// Record a received chunk (see [`TimeBase::on_chunk`]) and warn on stderr,
    /// with back-off, when live input falls behind real time.
    pub(crate) fn on_chunk_warn(&mut self, samples: usize, sample_rate: u32, prefix: &str) {
        self.on_chunk(samples, sample_rate);
        if let Some(lag) = self.lag_warning() {
            eprintln!(
                "{prefix}: live input is {lag:.1} s behind real time: samples are being lost, \
                 probably because decoding is slower than the sample rate. \
                 Timestamps follow sample arrival time."
            );
        }
    }

    /// Resolve a sample-count offset (seconds into the recording or run) to
    /// the event's `timestamp` and `time_offset`.
    ///
    /// For live input, the offset is measured back from the end of the latest
    /// chunk, whose arrival time is known, and `time_offset` is the wall-clock
    /// time since the first sample.
    pub(crate) fn resolve(&self, offset_s: f64) -> (Option<f64>, f64) {
        match &self.kind {
            Kind::OffsetOnly => (None, offset_s),
            Kind::Capture { start } => (Some(start + offset_s), offset_s),
            Kind::Wallclock(live) => match (live.run_start, live.anchor) {
                (Some(start), Some((arrival, end_offset))) => {
                    let timestamp = arrival - (end_offset - offset_s).max(0.0);
                    (Some(timestamp), (timestamp - start).max(0.0))
                }
                _ => {
                    let now = unix_now();
                    (Some(now), offset_s)
                }
            },
        }
    }

    pub(crate) fn time_source(&self) -> Option<TimeSource> {
        match self.kind {
            Kind::OffsetOnly => None,
            Kind::Capture { .. } => Some(TimeSource::Capture),
            Kind::Wallclock(_) => Some(TimeSource::Wallclock),
        }
    }
}

/// Parse a capture start time given as RFC 3339 (`2026-06-03T08:30:11Z`) or
/// Unix epoch seconds (`1780475411.5`).
pub(crate) fn parse_start_time(value: &str) -> anyhow::Result<f64> {
    let value = value.trim();
    if let Ok(epoch) = value.parse::<f64>() {
        anyhow::ensure!(epoch.is_finite(), "invalid start time {value:?}");
        return Ok(epoch);
    }
    let dt = chrono::DateTime::parse_from_rfc3339(value).map_err(|err| {
        anyhow::anyhow!(
            "invalid start time {value:?} ({err}); expected RFC 3339 such as \
             2026-06-03T08:30:11Z or Unix epoch seconds"
        )
    })?;
    Ok(dt.timestamp() as f64 + f64::from(dt.timestamp_subsec_nanos()) * 1e-9)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_start_time_formats() {
        assert_eq!(parse_start_time("1780475411").unwrap(), 1_780_475_411.0);
        assert_eq!(
            parse_start_time("2026-06-03T08:30:11Z").unwrap(),
            1_780_475_411.0
        );
        assert_eq!(
            parse_start_time("2026-06-03T10:30:11.5+02:00").unwrap(),
            1_780_475_411.5
        );
        assert!(parse_start_time("yesterday").is_err());
    }

    #[test]
    fn file_without_start_has_no_absolute_time() {
        let src: Source = "file:///tmp/capture.cu8".parse().unwrap();
        let base = TimeBase::for_source(&src).unwrap();
        assert_eq!(base.resolve(1.0).0, None);
        assert_eq!(base.time_source(), None);
    }

    #[test]
    fn stdin_has_no_absolute_time() {
        let src: Source = "-".parse().unwrap();
        assert_eq!(TimeBase::for_source(&src).unwrap(), TimeBase::offset_only());
    }

    #[test]
    fn gqrx_filename_gives_capture_time() {
        let src: Source = "gqrx_20260603_083011_134000000_8000000_fc.raw"
            .parse()
            .unwrap();
        let base = TimeBase::for_source(&src).unwrap();
        assert_eq!(base.resolve(2.5), (Some(1_780_475_413.5), 2.5));
        assert_eq!(base.time_source(), Some(TimeSource::Capture));
    }

    #[test]
    fn explicit_start_time_overrides_filename() {
        let src: Source =
            "gqrx_20260603_083011_134000000_8000000_fc.raw?start_time=2026-06-03T09:00:00Z"
                .parse()
                .unwrap();
        let base = TimeBase::for_source(&src).unwrap();
        assert_eq!(base.resolve(0.0).0, Some(1_780_477_200.0));
    }

    #[test]
    fn live_time_follows_chunk_arrival_not_sample_count() {
        let mut base = TimeBase::wallclock();
        // 1 s chunks at 1 kS/s arriving on time.
        base.on_chunk_at(1_000, 1_000, 1_000.0);
        base.on_chunk_at(1_000, 1_000, 1_001.0);
        assert_eq!(base.resolve(1.5), (Some(1_000.5), 1.5));
        assert_eq!(base.lag(), Some(0.0));
        // Ten seconds of samples are lost before the next chunk arrives: a frame
        // in it is still stamped with real time, and the lag is reported.
        base.on_chunk_at(1_000, 1_000, 1_012.0);
        let (timestamp, offset) = base.resolve(2.5);
        assert_eq!(timestamp, Some(1_011.5));
        assert_eq!(offset, 12.5);
        assert_eq!(base.lag(), Some(10.0));
        assert_eq!(base.time_source(), Some(TimeSource::Wallclock));
    }

    #[test]
    fn lag_warnings_back_off() {
        let mut base = TimeBase::wallclock();
        base.on_chunk_at(1_000, 1_000, 1_000.0);
        assert_eq!(base.lag_warning(), None);
        base.on_chunk_at(1_000, 1_000, 1_004.0); // 3 s behind
        assert_eq!(base.lag_warning(), Some(3.0));
        base.on_chunk_at(1_000, 1_000, 1_005.5); // 3.5 s: below next threshold (4 s)
        assert_eq!(base.lag_warning(), None);
        base.on_chunk_at(1_000, 1_000, 1_008.0); // 5 s
        assert_eq!(base.lag_warning(), Some(5.0));
    }

    #[test]
    fn non_live_time_bases_ignore_chunks() {
        let mut base = TimeBase::capture(100.0);
        base.on_chunk_at(1_000, 1_000, 5_000.0);
        assert_eq!(base.resolve(1.0), (Some(101.0), 1.0));
        assert_eq!(base.lag(), None);
    }
}
