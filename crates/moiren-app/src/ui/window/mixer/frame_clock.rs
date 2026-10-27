//! Mixer-only frame demand. Native paint callbacks remain the renderer; this
//! clock only supplies bounded wakeups when compositor delivery falls behind.
use super::meter_frame::MixerMeterFrame;
#[cfg(windows)]
use gpui_kit::Task;
use gpui_kit::{Context, Window};
#[cfg(any(windows, test))]
use std::time::{Duration, Instant};

#[cfg(windows)]
#[allow(unsafe_code)]
mod native;

#[cfg(any(windows, test))]
const PERIOD: Duration = Duration::from_nanos(4_166_667);

#[cfg(any(windows, test))]
struct Cadence {
    next: Instant,
}

#[cfg(any(windows, test))]
impl Cadence {
    fn new(now: Instant) -> Self {
        Self { next: now + PERIOD }
    }

    fn take(&mut self, now: Instant) -> bool {
        if now < self.next {
            return false;
        }
        // Keep phase after a late wake, but never replay missed frames in a
        // burst. The next wake is always a future deadline.
        let skipped = now.duration_since(self.next).as_nanos() / PERIOD.as_nanos();
        self.next += PERIOD * (skipped.min(u32::MAX as u128 - 1) as u32 + 1);
        true
    }
}

#[derive(Default)]
pub(in crate::ui::window) struct FrameClock {
    #[cfg(windows)]
    running: Option<(native::Wake, Task<()>)>,
    #[cfg(windows)]
    failed: bool,
}

impl FrameClock {
    #[cfg(all(windows, feature = "gpui-test"))]
    pub(in crate::ui::window) fn interrupt_worker(&self) {
        if let Some((wake, _)) = &self.running {
            wake.cancel();
        }
    }

    #[cfg(all(windows, feature = "gpui-test"))]
    pub(in crate::ui::window) fn is_running(&self) -> bool {
        self.running
            .as_ref()
            .is_some_and(|(_, task)| !task.is_ready())
    }
    pub fn stop(&mut self) {
        #[cfg(windows)]
        {
            self.running = None;
            self.failed = false;
        }
    }

    /// Only a successfully initialized clock replaces native animation demand.
    pub fn sync(
        &mut self,
        active: bool,
        window: &Window,
        cx: &mut Context<MixerMeterFrame>,
    ) -> bool {
        #[cfg(windows)]
        {
            if !active || !native::has_window(window) {
                self.stop();
                return false;
            }
            if self.failed
                || self
                    .running
                    .as_ref()
                    .is_some_and(|(_, task)| task.is_ready())
            {
                self.running = None;
                self.failed = true;
                return false;
            }
            if self.running.is_none() {
                let Ok((wake, receiver)) = native::Wake::start() else {
                    self.failed = true;
                    return false;
                };
                let task = cx.spawn_in(window, async move |this, cx| {
                    while receiver.recv().await.is_ok() {
                        if !this
                            .update_in(cx, |view, window, cx| {
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
                        view.clock.failed = true;
                        cx.notify();
                        native::request_paint(window);
                    });
                });
                self.running = Some((wake, task));
            }
            true
        }
        #[cfg(not(windows))]
        {
            let _ = (active, window, cx);
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
        let mut cadence = Cadence::new(began);
        assert!(!cadence.take(began + PERIOD / 2));
        assert!(cadence.take(began + PERIOD));
        assert!(!cadence.take(began + PERIOD));
        assert!(cadence.take(began + PERIOD * 10 + PERIOD / 2));
        assert_eq!(cadence.next, began + PERIOD * 11);
        assert!(!cadence.take(began + PERIOD * 10 + PERIOD / 2));
    }

    #[test]
    fn cadence_keeps_240hz_phase_instead_of_sleeping_after_work() {
        let began = Instant::now();
        let mut cadence = Cadence::new(began);
        for tick in 1..=240 {
            assert!(cadence.take(began + PERIOD * tick + Duration::from_micros(100)));
        }
        assert_eq!(cadence.next, began + PERIOD * 241);
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
                let (wake, _receiver) = native::Wake::start().unwrap();
                view.clock.running = Some((wake, Task::ready(())));
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
