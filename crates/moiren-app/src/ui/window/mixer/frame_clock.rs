//! Mixer-only frame demand. Native paint callbacks remain the renderer; this
//! clock only supplies bounded wakeups when compositor delivery falls behind.
use super::meter_frame::MixerMeterFrame;
#[cfg(windows)]
use gpui_kit::Task;
use gpui_kit::{Context, Window};
use std::time::Duration;
#[cfg(any(windows, test))]
use std::time::Instant;

#[cfg(windows)]
#[allow(unsafe_code)]
mod native;

#[cfg(any(windows, test))]
struct Cadence {
    period: Duration,
    next: Instant,
}

#[cfg(any(windows, test))]
impl Cadence {
    fn new(now: Instant, period: Duration) -> Self {
        Self {
            period,
            next: now + period,
        }
    }

    fn take(&mut self, now: Instant) -> bool {
        if now < self.next {
            return false;
        }
        // Keep phase after a late wake, but never replay missed frames in a
        // burst. The next wake is always a future deadline.
        let skipped = now.duration_since(self.next).as_nanos() / self.period.as_nanos();
        self.next += self.period * (skipped.min(u32::MAX as u128 - 1) as u32 + 1);
        true
    }
}

#[cfg(any(windows, test))]
fn usable_refresh_interval(refresh_interval: Option<Duration>) -> Option<Duration> {
    refresh_interval.filter(|period| !period.is_zero())
}

#[derive(Default)]
pub(in crate::ui::window) struct FrameClock {
    #[cfg(windows)]
    running: Option<(Duration, native::Wake, Task<()>)>,
    #[cfg(windows)]
    failed: bool,
    #[cfg(windows)]
    generation: u64,
}

impl FrameClock {
    #[cfg(all(windows, feature = "gpui-test"))]
    pub(in crate::ui::window) fn interrupt_worker(&self) {
        if let Some((_, wake, _)) = &self.running {
            wake.cancel();
        }
    }

    #[cfg(all(windows, feature = "gpui-test"))]
    pub(in crate::ui::window) fn is_running(&self) -> bool {
        self.running
            .as_ref()
            .is_some_and(|(_, _, task)| !task.is_ready())
    }
    pub fn stop(&mut self) {
        #[cfg(windows)]
        {
            self.generation = self.generation.wrapping_add(1);
            self.running = None;
            self.failed = false;
        }
    }

    /// Only a successfully initialized clock replaces native animation demand.
    pub fn sync(
        &mut self,
        active: bool,
        refresh_interval: Option<Duration>,
        window: &Window,
        cx: &mut Context<MixerMeterFrame>,
    ) -> bool {
        #[cfg(windows)]
        {
            if !active || !native::has_window(window) {
                self.stop();
                return false;
            }
            let Some(period) = usable_refresh_interval(refresh_interval) else {
                self.stop();
                return false;
            };
            if self.failed
                || self
                    .running
                    .as_ref()
                    .is_some_and(|(_, _, task)| task.is_ready())
            {
                self.running = None;
                self.failed = true;
                return false;
            }
            if self
                .running
                .as_ref()
                .is_some_and(|(running_period, _, _)| *running_period != period)
            {
                self.stop();
            }
            if self.running.is_none() {
                let Ok((wake, receiver)) = native::Wake::start(period) else {
                    self.failed = true;
                    return false;
                };
                let generation = self.generation;
                let task = cx.spawn_in(window, async move |this, cx| {
                    while receiver.recv().await.is_ok() {
                        if !this
                            .update_in(cx, |view, window, cx| {
                                if view.clock.generation != generation {
                                    return false;
                                }
                                if !view.frame_active(cx) {
                                    return false;
                                }
                                #[cfg(feature = "gpui-test")]
                                if let Some(shell) = view.shell.upgrade() {
                                    shell.read(cx).mixer.trace.borrow_mut().clock_ticks += 1;
                                }
                                cx.notify();
                                native::request_paint(window);
                                true
                            })
                            .unwrap_or(false)
                        {
                            return;
                        }
                    }
                    // A worker failure must wake the view once so rendering
                    // can fall back to GPUI RAF instead of freezing silently.
                    let _ = this.update_in(cx, |view, window, cx| {
                        if view.clock.generation == generation {
                            view.clock.failed = true;
                            view.clock.running = None;
                            cx.notify();
                            native::request_paint(window);
                        }
                    });
                });
                self.running = Some((period, wake, task));
            }
            true
        }
        #[cfg(not(windows))]
        {
            let _ = (active, refresh_interval, window, cx);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cadence_does_not_render_early_or_burst_after_a_stall() {
        let began = Instant::now();
        let period = Duration::from_secs(1) / 240;
        let mut cadence = Cadence::new(began, period);
        assert!(!cadence.take(began + period / 2));
        assert!(cadence.take(began + period));
        assert!(!cadence.take(began + period));
        assert!(cadence.take(began + period * 10 + period / 2));
        assert_eq!(cadence.next, began + period * 11);
        assert!(!cadence.take(began + period * 10 + period / 2));
    }

    #[test]
    fn cadence_uses_the_reported_display_refresh_interval() {
        let began = Instant::now();
        for hz in [60, 120, 144, 240, 360] {
            let period = Duration::from_secs(1) / hz;
            let mut cadence = Cadence::new(began, period);
            assert!(!cadence.take(began + period - Duration::from_nanos(1)));
            for tick in 1..=hz {
                assert!(cadence.take(began + period * tick + Duration::from_micros(100)));
            }
            assert_eq!(cadence.next, began + period * (hz + 1));
        }
    }

    #[test]
    fn unknown_refresh_interval_uses_gpui_animation_fallback() {
        assert_eq!(usable_refresh_interval(None), None);
        assert_eq!(usable_refresh_interval(Some(Duration::ZERO)), None);
        assert_eq!(
            usable_refresh_interval(Some(Duration::from_millis(8))),
            Some(Duration::from_millis(8))
        );
    }

    #[cfg(all(windows, feature = "gpui-test"))]
    #[gpui_kit::test]
    fn display_refresh_change_cancels_the_old_clock(cx: &mut gpui_kit::TestAppContext) {
        use crate::ui::window::{Page, test_harness};
        use gpui_kit::test::TestWindowExt;

        let (shell, cx, runtime) = test_harness::test_shell(cx);
        cx.simulate_display_refresh_interval_change(
            gpui_kit::DisplayId::new(1),
            Some(Duration::from_secs(1) / 240),
        );
        cx.run_until_parked();
        shell.update(cx, |shell, cx| {
            shell.presentation.page = Page::Mixer;
            shell.state.update(cx, |state, _| {
                std::sync::Arc::make_mut(&mut state.snapshot).phase =
                    crate::service::SessionPhase::Running;
            });
            cx.notify();
        });
        cx.update(|window, cx| window.render_frame(cx));

        let view = shell.read_with(cx, |shell, _| shell.mixer_frame.clone());
        view.update(cx, |view, _| {
            let old_period = Duration::from_secs(1) / 240;
            let (wake, _) = native::Wake::start(old_period).unwrap();
            view.clock.running = Some((old_period, wake, Task::ready(())));
        });
        let generation = view.read_with(cx, |view, _| view.clock.generation);

        cx.simulate_display_refresh_interval_change(
            gpui_kit::DisplayId::new(1),
            Some(Duration::from_secs(1) / 60),
        );
        cx.run_until_parked();

        assert!(view.read_with(cx, |view, _| view.clock.running.is_none()));
        assert!(view.read_with(cx, |view, _| view.clock.generation > generation));
        test_harness::finish_service(runtime);
    }

    #[cfg(all(windows, feature = "gpui-test"))]
    #[gpui_kit::test]
    fn inactive_shell_notifications_cancel_clock_without_a_meter_render(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use crate::ui::window::{Page, test_harness};
        let (shell, cx, runtime) = test_harness::test_shell(cx);
        let view = shell.read_with(cx, |shell, _| shell.mixer_frame.clone());
        for inactive in 0..5 {
            shell.update(cx, |shell, cx| {
                shell.presentation.page = Page::Mixer;
                shell.presentation.visible = true;
                shell.presentation.minimized = false;
                shell.settings_open = false;
                shell.state.update(cx, |state, _| {
                    std::sync::Arc::make_mut(&mut state.snapshot).phase =
                        crate::service::SessionPhase::Running;
                });
                cx.notify();
            });
            view.update(cx, |view, _| {
                let period = Duration::from_millis(16);
                let (wake, _receiver) = native::Wake::start(period).unwrap();
                view.clock.running = Some((period, wake, Task::ready(())));
            });
            shell.update(cx, |shell, cx| {
                match inactive {
                    0 => shell.set_page(Page::Graph, cx),
                    1 => shell.presentation.visible = false,
                    2 => shell.presentation.minimized = true,
                    3 => shell.settings_open = true,
                    _ => shell.state.update(cx, |state, _| {
                        std::sync::Arc::make_mut(&mut state.snapshot).phase =
                            crate::service::SessionPhase::Idle;
                    }),
                }
                cx.notify();
            });
            assert!(view.read_with(cx, |view, _| view.clock.running.is_none()));
        }
        test_harness::finish_service(runtime);
    }
}
