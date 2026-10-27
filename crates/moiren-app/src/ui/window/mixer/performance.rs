//! Opt-in native capture instrumentation; absent from production builds.
use std::time::{Duration, Instant};

#[cfg(feature = "ui-profiler")]
pub(in crate::ui::window) fn write_platform_frames(
    frames: &mut gpui_kit::profiler::FrameTimingCollector,
    path: &std::path::Path,
) -> anyhow::Result<()> {
    use gpui_kit::profiler::FrameEvent;
    let mut previous = None;
    let mut rows = Vec::new();
    for event in frames.collect_unseen() {
        match event {
            FrameEvent::Draw(frame) => rows.push(serde_json::json!({
                "event": "draw",
                "draw_ms": (frame.draw_end - frame.draw_start).as_secs_f64() * 1000.,
                "signal_wait_ms": frame.signal_at.map(|time| frame.draw_start.saturating_duration_since(time).as_secs_f64() * 1000.),
                "layout_ms": frame.phases.request_layout.as_secs_f64() * 1000.,
                "prepaint_ms": frame.phases.prepaint.as_secs_f64() * 1000.,
                "paint_ms": frame.phases.paint.as_secs_f64() * 1000.,
                "refresh_ms": frame.refresh_interval.map(|duration| duration.as_secs_f64() * 1000.),
            })),
            FrameEvent::Present(frame) => {
                rows.push(serde_json::json!({
                    "event": "present",
                    "gap_ms": previous.map(|time| frame.present_end.saturating_duration_since(time).as_secs_f64() * 1000.),
                    "present_ms": frame.present_duration().as_secs_f64() * 1000.,
                }));
                previous = Some(frame.present_end);
            }
        }
    }
    std::fs::write(path, serde_json::to_vec_pretty(&rows)?)?;
    Ok(())
}

#[derive(Default)]
pub(in crate::ui::window) struct MeterTrace {
    pub meter_paints: usize,
    pub strip_renders: usize,
    previous: Option<Instant>,
    cpu: Duration,
    gap: Duration,
    active: bool,
    frames: Vec<(f64, f64, usize)>,
}

impl MeterTrace {
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
        }
    }

    pub fn write(&self, path: &std::path::Path) -> anyhow::Result<()> {
        std::fs::write(path, serde_json::to_vec_pretty(&self.frames)?)?;
        let mut cpu: Vec<_> = self
            .frames
            .iter()
            .filter(|frame| frame.2 >= 8)
            .map(|frame| frame.1)
            .collect();
        let mut gaps: Vec<_> = self
            .frames
            .iter()
            .filter(|frame| frame.2 >= 8 && frame.0 > 0.)
            .map(|frame| frame.0)
            .collect();
        cpu.sort_by(f64::total_cmp);
        gaps.sort_by(f64::total_cmp);
        if !cpu.is_empty() && !gaps.is_empty() {
            eprintln!(
                "Native meter frames: {}, meter work p95={:.3}ms, frame interval p50={:.3}ms p95={:.3}ms, intervals >12.5ms={}",
                cpu.len(),
                cpu[(cpu.len() * 95 / 100).min(cpu.len() - 1)],
                gaps[gaps.len() / 2],
                gaps[(gaps.len() * 95 / 100).min(gaps.len() - 1)],
                gaps.iter().filter(|gap| **gap > 12.5).count()
            );
        }
        Ok(())
    }
}
