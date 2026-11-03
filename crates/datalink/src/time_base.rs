//! Time base for frames demodulated from I/Q sources.
//!
//! Live SDR input is timestamped from the wall clock at the start of the run.
//! File and stdin input is only given an absolute timestamp when the capture
//! start is known, either from an explicit `start_time` or from a capture
//! filename that encodes it (Gqrx, SDRuno). Otherwise frames carry only their
//! offset into the recording, so processing time is never mistaken for
//! reception time.

use crate::event::TimeSource;
use crate::source::{Address, Source};
use std::time::{SystemTime, UNIX_EPOCH};

/// Absolute reference that sample offsets are added to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TimeBase {
    start_unix: Option<f64>,
    source: Option<TimeSource>,
}

impl TimeBase {
    /// Wall-clock time base starting now, used for live SDR input.
    pub(crate) fn wallclock_now() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or_default();
        Self {
            start_unix: Some(now),
            source: Some(TimeSource::Wallclock),
        }
    }

    /// Time base anchored at a known capture start.
    pub(crate) fn capture(start_unix: f64) -> Self {
        Self {
            start_unix: Some(start_unix),
            source: Some(TimeSource::Capture),
        }
    }

    /// No absolute time: frames only carry their offset into the recording.
    pub(crate) fn offset_only() -> Self {
        Self {
            start_unix: None,
            source: None,
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
            _ => Self::wallclock_now(),
        })
    }

    /// Unix timestamp for a frame `offset_s` seconds into the recording.
    pub(crate) fn timestamp(&self, offset_s: f64) -> Option<f64> {
        self.start_unix.map(|start| start + offset_s)
    }

    pub(crate) fn time_source(&self) -> Option<TimeSource> {
        self.source
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
        assert_eq!(base.timestamp(1.0), None);
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
        assert_eq!(base.timestamp(2.5), Some(1_780_475_413.5));
        assert_eq!(base.time_source(), Some(TimeSource::Capture));
    }

    #[test]
    fn explicit_start_time_overrides_filename() {
        let src: Source =
            "gqrx_20260603_083011_134000000_8000000_fc.raw?start_time=2026-06-03T09:00:00Z"
                .parse()
                .unwrap();
        let base = TimeBase::for_source(&src).unwrap();
        assert_eq!(base.timestamp(0.0), Some(1_780_477_200.0));
    }
}
