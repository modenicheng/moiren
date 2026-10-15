//! Developer-only capture of the production shell through GPUI's native renderer.
use super::*;

pub fn run_ui_capture() -> anyhow::Result<()> {
    let directory = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "docs/designs/screenshots/application".into());
    std::fs::create_dir_all(&directory)?;
    let service = start_service().map_err(|error| anyhow::anyhow!("service: {error:?}"))?;
    let handle = service.handle();
    let initial = crate::ui::state::initial_snapshot(&handle)?;
    let mut owned = OwnedService(Some(service));
    let failed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let capture_failure = failed.clone();
    gpui_kit::application()
        .with_assets(ActionAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            register_fonts(cx).expect("embedded font");
            let state = cx.new(|_| UiState::new(handle.clone(), initial));
            state.update(cx, |s, _| s.dispatch(AppRequest::RefreshCatalog));
            let (window, shell) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::centered(size(px(1440.), px(900.)), cx)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Moiren · UI capture".into()),
                        appears_transparent: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| StudioShell::new(state, window, cx, None)),
            )
            .expect("capture window");
            shell.update(cx, |s, cx| s.spawn_poll(cx));
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(1200))
                    .await;
                for (name, width, height) in
                    [("comfortable", 1440., 900.), ("compact", 1024., 720.)]
                {
                    let _ = window.update(cx, |_, window, _| {
                        window.resize(size(px(width), px(height)))
                    });
                    cx.background_executor()
                        .timer(Duration::from_millis(150))
                        .await;
                    for page in [Page::Graph, Page::Mixer, Page::Matrix, Page::Devices] {
                        shell.update(cx, |s, cx| {
                            s.density = if name == "compact" {
                                Density::Compact
                            } else {
                                Density::Comfortable
                            };
                            s.set_page(page, cx);
                        });
                        cx.background_executor()
                            .timer(Duration::from_millis(100))
                            .await;
                        let path =
                            directory.join(format!("{name}-{}.png", page.title().to_lowercase()));
                        let result = window.update(cx, |_, window, cx| -> anyhow::Result<()> {
                            window.draw(cx).clear(cx);
                            window.render_to_image()?.save(&path)?;
                            Ok(())
                        });
                        if !matches!(result, Ok(Ok(()))) {
                            capture_failure.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        eprintln!("Capture {}: {result:?}", path.display());
                    }
                    shell.update(cx, |s, cx| {
                        s.settings_open = true;
                        cx.notify();
                    });
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                    let path = directory.join(format!("{name}-settings.png"));
                    let result = window.update(cx, |_, window, cx| -> anyhow::Result<()> {
                        window.draw(cx).clear(cx);
                        window.render_to_image()?.save(&path)?;
                        Ok(())
                    });
                    if !matches!(result, Ok(Ok(()))) {
                        capture_failure.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    eprintln!("Capture {}: {result:?}", path.display());
                }
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    owned.finish()?;
    anyhow::ensure!(
        !failed.load(std::sync::atomic::Ordering::Relaxed),
        "native UI capture failed"
    );
    Ok(())
}
