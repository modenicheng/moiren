use anyhow::{Context, bail};
use moiren_app::{AppConfig, OfflineApp};

fn main() -> anyhow::Result<()> {
    let mut config = AppConfig::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "Moiren offline IO demo\nUsage: moiren-app [--gain 0..16]\nRuns software input -> InputNode -> Gain -> OutputNode -> software output."
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
