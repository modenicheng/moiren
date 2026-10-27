//! Native lifecycle acceptance after measurement, never during timed samples.
use super::*;

#[cfg(windows)]
fn running(shell: &Entity<StudioShell>, cx: &AsyncApp) -> bool {
    shell.read_with(cx, |shell, cx| {
        shell.mixer_frame.read(cx).clock.is_running()
    })
}

#[cfg(windows)]
async fn settle(cx: &mut AsyncApp) {
    cx.background_executor()
        .timer(Duration::from_millis(120))
        .await;
}

pub(super) async fn check(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    directory: &std::path::Path,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        anyhow::ensure!(running(shell, cx), "native clock was not running on Mixer");
        for state in ["graph", "settings", "minimized", "hidden", "stopped"] {
            shell.update(cx, |shell, cx| {
                match state {
                    "graph" => shell.set_page(Page::Graph, cx),
                    "settings" => shell.settings_open = true,
                    "minimized" => shell.presentation.minimized = true,
                    "hidden" => shell.presentation.visible = false,
                    _ => shell.state.update(cx, |state, _| {
                        std::sync::Arc::make_mut(&mut state.snapshot).phase = SessionPhase::Idle;
                    }),
                }
                cx.notify();
            });
            // Cancellation is notification-driven, not conditional on the
            // inactive page receiving another paint callback.
            anyhow::ensure!(!running(shell, cx), "clock remained active in {state}");
            settle(cx).await;
            shell.update(cx, |shell, cx| {
                shell.presentation.show();
                shell.settings_open = false;
                shell.state.update(cx, |state, _| {
                    std::sync::Arc::make_mut(&mut state.snapshot).phase = SessionPhase::Running;
                });
                shell.set_page(Page::Mixer, cx);
            });
            settle(cx).await;
            anyhow::ensure!(running(shell, cx), "clock failed to resume after {state}");
        }
        let before = shell.read_with(cx, |shell, _| shell.mixer.trace.borrow().meter_paints);
        settle(cx).await;
        let after = shell.read_with(cx, |shell, _| shell.mixer.trace.borrow().meter_paints);
        anyhow::ensure!(after > before, "resumed clock did not paint meters");
        // The native minimized event must also reach the cancellation observer.
        window.update(cx, |_, window, _| window.minimize_window())?;
        settle(cx).await;
        anyhow::ensure!(
            !running(shell, cx),
            "native minimize did not stop the clock"
        );
        restore(window, cx)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        while !running(shell, cx) && Instant::now() < deadline {
            settle(cx).await;
        }
        anyhow::ensure!(
            running(shell, cx),
            "native restore did not resume the clock"
        );
        // Close the worker channel while the view remains active, exercising
        // the same receive-error path as a native timer failure.
        shell.read_with(cx, |shell, cx| {
            shell.mixer_frame.read(cx).clock.interrupt_worker();
        });
        settle(cx).await;
        anyhow::ensure!(!running(shell, cx), "interrupted worker remained active");
        let fallback_before =
            shell.read_with(cx, |shell, _| shell.mixer.trace.borrow().meter_paints);
        settle(cx).await;
        let fallback_after =
            shell.read_with(cx, |shell, _| shell.mixer.trace.borrow().meter_paints);
        anyhow::ensure!(
            fallback_after > fallback_before,
            "worker failure froze RAF fallback"
        );
        shell.update(cx, |shell, cx| shell.set_page(Page::Graph, cx));
        shell.update(cx, |shell, cx| shell.set_page(Page::Mixer, cx));
        settle(cx).await;
        anyhow::ensure!(running(shell, cx), "clock failed to restart after fallback");
        std::fs::write(
            directory.join("clock-lifecycle.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"cancel_resume": ["graph", "settings", "minimized", "hidden", "stopped"],
                "native_minimize_restore": true, "resumed_meter_paints": after - before,
                "worker_failure_fallback_paints": fallback_after - fallback_before}),
            )?,
        )?;
    }
    #[cfg(not(windows))]
    let _ = (window, shell, directory, cx);
    Ok(())
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn restore(window: AnyWindowHandle, cx: &mut AsyncApp) -> anyhow::Result<()> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{SW_RESTORE, ShowWindowAsync},
    };
    let hwnd = window.update(cx, |_, window, _| -> anyhow::Result<HWND> {
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|error| anyhow::anyhow!("capture HWND: {error}"))?;
        let RawWindowHandle::Win32(raw) = handle.as_raw() else {
            anyhow::bail!("capture window has no HWND");
        };
        Ok(HWND(raw.hwnd.get() as *mut _))
    })??;
    // SAFETY: this HWND belongs only to the disposable capture window. Restore
    // asynchronously outside the GPUI borrow, without synthesizing user input.
    unsafe { ShowWindowAsync(hwnd, SW_RESTORE).ok()? };
    Ok(())
}
