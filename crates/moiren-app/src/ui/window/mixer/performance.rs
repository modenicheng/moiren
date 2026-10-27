//! Opt-in native capture instrumentation; absent from production builds.
use std::time::{Duration, Instant};

#[cfg(feature = "ui-profiler")]
mod platform_trace;
#[cfg(feature = "ui-profiler")]
pub(in crate::ui::window) use platform_trace::PlatformTrace;

#[derive(Default)]
pub(in crate::ui::window) struct MeterTrace {
    pub meter_paints: usize,
    pub strip_renders: usize,
    previous: Option<Instant>,
    cpu: Duration,
    gap: Duration,
    active: bool,
    frames: Vec<(f64, f64, usize)>,
    dropped: usize,
    sample_counts: (usize, usize),
}

impl MeterTrace {
    pub fn start_sample(&mut self) {
        self.previous = None;
        self.cpu = Duration::ZERO;
        self.gap = Duration::ZERO;
        self.frames.clear();
        self.dropped = 0;
        self.sample_counts = (self.strip_renders, self.meter_paints);
    }

    pub fn begin(&mut self, active: bool) {
        self.active = active && std::env::var_os("MOIREN_CAPTURE_METERS").is_some();
        self.cpu = Duration::ZERO;
        if self.active {
            let now = Instant::now();
            self.gap = self
                .previous
                .map(|previous| now - previous)
                .unwrap_or_default();
            self.previous = Some(now);
        } else {
            self.previous = None;
        }
    }

    pub fn add(&mut self, elapsed: Duration) {
        if self.active {
            self.cpu += elapsed;
        }
    }

    pub fn finish(&mut self, visible: usize) {
        if self.active && self.frames.len() < 20_000 {
            self.frames.push((
                self.gap.as_secs_f64() * 1000.,
                self.cpu.as_secs_f64() * 1000.,
                visible,
            ));
        } else if self.active {
            self.dropped += 1;
        }
    }

    pub fn write(&self, path: &std::path::Path) -> anyhow::Result<()> {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": 2, "samples": self.frames, "dropped": self.dropped,
                "strip_renders": self.strip_renders - self.sample_counts.0,
                "meter_paints": self.meter_paints - self.sample_counts.1,
            }))?,
        )?;
        let mut cpu: Vec<_> = self
            .frames
            .iter()
            .filter(|frame| frame.2 > 0)
            .map(|frame| frame.1)
            .collect();
        let mut gaps: Vec<_> = self
            .frames
            .iter()
            .filter(|frame| frame.2 > 0 && frame.0 > 0.)
            .map(|frame| frame.0)
            .collect();
        cpu.sort_by(f64::total_cmp);
        gaps.sort_by(f64::total_cmp);
        if !cpu.is_empty() && !gaps.is_empty() {
            eprintln!(
                "Native meter frames: {}, meter work p95={:.3}ms, frame interval p50={:.3}ms p95={:.3}ms, intervals >12.5ms={}, dropped={}",
                cpu.len(),
                cpu[(cpu.len() * 95 / 100).min(cpu.len() - 1)],
                gaps[gaps.len() / 2],
                gaps[(gaps.len() * 95 / 100).min(gaps.len() - 1)],
                gaps.iter().filter(|gap| **gap > 12.5).count(),
                self.dropped
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_samples_do_not_retain_previous_viewport_or_silently_truncate() {
        let mut trace = MeterTrace {
            active: true,
            ..Default::default()
        };
        for _ in 0..20_001 {
            trace.finish(5);
        }
        assert_eq!(trace.dropped, 1);
        trace.start_sample();
        assert!(trace.frames.is_empty());
        assert!(trace.previous.is_none());
        assert_eq!(trace.dropped, 0);
    }
}
