use anyhow::{Context, bail};
use moiren_app::render_cli::{RENDER_HELP, RenderCommand, parse_render_args};
use moiren_app::{AppConfig, OfflineApp};

fn render(command: RenderCommand) -> anyhow::Result<()> {
    if matches!(command, RenderCommand::Help) {
        println!("{RENDER_HELP}");
        return Ok(());
    }
    #[cfg(windows)]
    {
        use moiren_app::tone::{ToneSession, prepare_tone};
        use moiren_windows_audio::render::{
            DemandRenderer, RenderOptions, RenderStatus, list_render_endpoints, start_render,
        };
        match command {
            RenderCommand::List => {
                let endpoints = list_render_endpoints().context("listing render endpoints")?;
                serde_json::to_writer_pretty(std::io::stdout().lock(), &endpoints)?;
                println!();
            }
            RenderCommand::Tone {
                endpoint_id,
                seconds,
                tone,
            } => {
                let ToneSession {
                    compiled, output, ..
                } = prepare_tone(tone).context("preparing the tone graph")?;
                let renderer = DemandRenderer::new(compiled.engine, output)?;
                eprintln!(
                    "Playing {:.1} Hz, gain {}, pan {}, for {} seconds on {}",
                    tone.frequency_hz, tone.gain, tone.pan, seconds, endpoint_id
                );
                let session = start_render(
                    RenderOptions {
                        endpoint_id,
                        duration: std::time::Duration::from_secs(u64::from(seconds)),
                    },
                    renderer,
                )?;
                let report = session.join().context("joining the render owner")?;
                serde_json::to_writer_pretty(std::io::stdout().lock(), &report)?;
                println!();
                if report.status == RenderStatus::Failed {
                    bail!(
                        "render failed: {}",
                        report.failure.as_deref().unwrap_or("unknown failure")
                    );
                }
                drop((compiled.control, compiled.bindings));
            }
            RenderCommand::Help => unreachable!("handled above"),
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = command;
        bail!("WASAPI render requires Windows");
    }
}

fn main() -> anyhow::Result<()> {
    let mut config = AppConfig::default();
    let mut args = std::env::args().skip(1).peekable();
    if args.peek().is_some_and(|arg| arg == "render") {
        args.next();
        return render(parse_render_args(args)?);
    }
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "Moiren\nUsage: moiren-app [--gain 0..16]\nRuns the offline software IO demo.\nUse moiren-app render --help for an explicitly selected Windows output."
                );
                return Ok(());
            }
            "--gain" => {
                config.initial_gain = args
                    .next()
                    .context("--gain requires a value")?
                    .parse()
                    .context("invalid --gain value")?;
            }
            _ => bail!("unknown argument: {arg}; use --help"),
        }
    }
    let mut app = OfflineApp::new(config).context("preparing the offline application")?;
    let input = [0.25, -0.5, 0.5, -0.25, 1.0, -1.0, 0.0, 0.0];
    let mut output = [0.0; 8];
    let report = app
        .process_interleaved(&input, &mut output)
        .context("processing the IO chain")?;
    println!("InputNode -> Gain({}) -> OutputNode", config.initial_gain);
    println!("input:  {input:?}\noutput: {output:?}");
    println!("processed: {report:?}");
    println!("input status:  {:?}", app.input_status());
    println!("output status: {:?}", app.output_status());
    app.stop();
    Ok(())
}
