//! Capture-only frame scheduling evidence, kept off the production render path.
use gpui_kit::{App, WindowId, profiler};
use profiler::journal::{ForegroundEvent, ForegroundJournalEntry, IntervalBoundary};
use std::time::{Duration, Instant};

pub(in crate::ui::window) struct PlatformTrace {
    began: Instant,
    window_id: WindowId,
    frames: profiler::FrameTimingCollector,
    journal: profiler::journal::ForegroundJournalCollector,
    frame_events: Vec<profiler::FrameEvent>,
    entries: Vec<ForegroundJournalEntry>,
    lost: u64,
}

impl PlatformTrace {
    pub fn new(cx: &App, window_id: WindowId) -> Self {
        Self {
            began: Instant::now(),
            window_id,
            frames: profiler::FrameTimingCollector::new(),
            journal: cx.foreground_journal().collector(),
            frame_events: Vec::new(),
            entries: Vec::new(),
            lost: 0,
        }
    }

    pub fn collect(&mut self) {
        self.frame_events
            .extend(self.frames.collect_unseen().into_iter().filter(|event| {
                (match event {
                    profiler::FrameEvent::Draw(frame) => frame.window_id,
                    profiler::FrameEvent::Present(frame) => frame.window_id,
                }) == self.window_id
            }));
        let drained = self.journal.collect_unseen();
        self.entries.extend(drained.entries);
        self.lost += drained.lost;
    }

    pub fn write(mut self, path: &std::path::Path) -> anyhow::Result<()> {
        let elapsed = self.began.elapsed();
        self.collect();
        let at = |time: Instant| ms(time.saturating_duration_since(self.began));
        let mut previous = None;
        let frames: Vec<_> = self.frame_events.into_iter().filter(|event| {
            (match event {
                profiler::FrameEvent::Draw(frame) => frame.window_id,
                profiler::FrameEvent::Present(frame) => frame.window_id,
            }) == self.window_id
        }).map(|event| match event {
            profiler::FrameEvent::Draw(frame) => serde_json::json!({
                "event": "draw", "at_ms": at(frame.draw_start),
                "draw_ms": ms(frame.draw_end - frame.draw_start),
                "signal_wait_ms": frame.signal_at.map(|time| ms(frame.draw_start.saturating_duration_since(time))),
                "dirty_wait_ms": frame.dirty_at.map(|time| ms(frame.draw_start.saturating_duration_since(time))),
                "invalidations": frame.invalidations,
                "layout_ms": ms(frame.phases.request_layout),
                "prepaint_ms": ms(frame.phases.prepaint),
                "paint_ms": ms(frame.phases.paint),
                "refresh_ms": frame.refresh_interval.map(ms),
            }),
            profiler::FrameEvent::Present(frame) => {
                let row = serde_json::json!({
                    "event": "present", "at_ms": at(frame.present_end),
                    "gap_ms": previous.map(|time| ms(frame.present_end.saturating_duration_since(time))),
                    "present_ms": ms(frame.present_duration()),
                });
                previous = Some(frame.present_end);
                row
            }
        }).collect();
        let journal: Vec<_> = self.entries.into_iter().filter_map(|entry| match entry {
            ForegroundJournalEntry::PlatformSignal(signal) if signal.window_id == self.window_id => Some(serde_json::json!({
                "event": "signal", "at_ms": at(signal.handled_at),
                "signal_ms": signal.signal_at.map(at), "source": format!("{:?}", signal.source),
            })),
            ForegroundJournalEntry::ReentrantFrameSkipped { at: time, window_id } if window_id == self.window_id => Some(serde_json::json!({
                "event": "reentrant", "at_ms": at(time),
            })),
            ForegroundJournalEntry::Boundary(IntervalBoundary::FrameSkipped(frame)) if frame.window_id == self.window_id => Some(serde_json::json!({
                "event": "skipped", "at_ms": at(frame.at), "reason": format!("{:?}", frame.reason),
            })),
            ForegroundJournalEntry::Event(ForegroundEvent::TaskPoll(task)) => Some(serde_json::json!({
                "event": "task", "at_ms": at(task.start), "duration_ms": ms(task.poll_duration()),
                "location": task.location.to_string(),
            })),
            ForegroundJournalEntry::Event(ForegroundEvent::View(view)) => Some(serde_json::json!({
                "event": "view", "at_ms": at(view.start), "duration_ms": ms(view.end - view.start),
                "self_ms": ms(view.self_duration), "name": view.name, "phase": view.phase.as_str(),
            })),
            ForegroundJournalEntry::Event(ForegroundEvent::SmallPolls(polls)) => Some(serde_json::json!({
                "event": "small_polls", "at_ms": at(polls.since), "until_ms": at(polls.until),
                "count": polls.summary.count, "duration_ms": ms(polls.summary.total),
            })),
            _ => None,
        }).collect();
        let draws = frames
            .iter()
            .filter(|frame| frame["event"] == "draw")
            .count();
        let presents = frames
            .iter()
            .filter(|frame| frame["event"] == "present")
            .count();
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": 2, "window_id": self.window_id.as_u64(),
                "elapsed_ms": ms(elapsed), "journal_lost": self.lost,
                "draws": draws, "presents": presents,
                "render_fps": draws as f64 / elapsed.as_secs_f64(),
                "present_fps": presents as f64 / elapsed.as_secs_f64(),
                "target_hz": 240, "target_budget_ms": 1000. / 240.,
                "dwm": composition_timing(),
                "frames": frames, "journal": journal,
            }))?,
        )?;
        Ok(())
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn composition_timing() -> serde_json::Value {
    use windows::Win32::{
        Foundation::HWND,
        Graphics::Dwm::{DWM_TIMING_INFO, DwmGetCompositionTimingInfo},
    };
    let mut timing = DWM_TIMING_INFO {
        cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32,
        ..Default::default()
    };
    // SAFETY: timing is a writable, correctly sized API output buffer. This
    // read-only query never changes the user's display mode.
    if unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut timing) }.is_err() {
        return serde_json::Value::Null;
    }
    // Copy packed fields before serde takes references to the JSON values.
    let (refresh_numerator, refresh_denominator) = (
        timing.rateRefresh.uiNumerator,
        timing.rateRefresh.uiDenominator,
    );
    let (compose_numerator, compose_denominator) = (
        timing.rateCompose.uiNumerator,
        timing.rateCompose.uiDenominator,
    );
    serde_json::json!({
        "refresh_numerator": refresh_numerator, "refresh_denominator": refresh_denominator,
        "compose_numerator": compose_numerator, "compose_denominator": compose_denominator,
    })
}

#[cfg(not(windows))]
fn composition_timing() -> serde_json::Value {
    serde_json::Value::Null
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui_kit::test]
    fn platform_trace_keeps_only_the_capture_window(cx: &mut gpui_kit::TestAppContext) {
        let target = gpui_kit::WindowId::from(1);
        let unrelated = gpui_kit::WindowId::from(2);
        let mut trace = cx.update(|cx| PlatformTrace::new(cx, target));
        let frame = |window_id, time| {
            profiler::FrameEvent::Present(profiler::PresentTiming {
                window_id,
                present_start: time,
                present_end: time,
                animation_interval: None,
            })
        };
        trace.frame_events.extend([
            frame(unrelated, trace.began),
            frame(target, trace.began + Duration::from_millis(4)),
            frame(target, trace.began + Duration::from_millis(8)),
        ]);
        let path =
            std::env::temp_dir().join(format!("moiren-platform-trace-{}.json", std::process::id()));
        trace.write(&path).unwrap();
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(report["frames"].as_array().unwrap().len(), 2);
        assert!(report["frames"][0]["gap_ms"].is_null());
        assert_eq!(report["frames"][1]["gap_ms"], 4.);
    }
}
