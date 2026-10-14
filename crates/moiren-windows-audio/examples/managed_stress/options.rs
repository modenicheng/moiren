use anyhow::{Context, bail, ensure};

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub seconds: u32,
    pub nodes: usize,
    pub edits_per_second: u32,
    pub parameters_per_second: u32,
    pub cycles: u32,
    pub confirmed_swaps: bool,
}

impl Options {
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> anyhow::Result<Self> {
        let mut options = Self {
            seconds: 60,
            nodes: 32,
            edits_per_second: 20,
            parameters_per_second: 200,
            cycles: 10,
            confirmed_swaps: false,
        };
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            let value: u32 = arguments
                .next()
                .context("each option requires a value")?
                .parse()?;
            match argument.as_str() {
                "--seconds" => options.seconds = value,
                "--nodes" => options.nodes = value as usize,
                "--edits-per-second" => options.edits_per_second = value,
                "--parameters-per-second" => options.parameters_per_second = value,
                "--cycles" => options.cycles = value,
                "--confirmed-swaps" => {
                    ensure!(value <= 1, "confirmed-swaps must be 0 or 1");
                    options.confirmed_swaps = value == 1;
                }
                _ => bail!("unknown option: {argument}"),
            }
        }
        ensure!(
            (1..=1800).contains(&options.seconds),
            "seconds must be 1..1800"
        );
        ensure!((2..=120).contains(&options.nodes), "nodes must be 2..120");
        ensure!(
            options.edits_per_second <= 2000,
            "edit rate must be <= 2000"
        );
        ensure!(
            options.parameters_per_second <= 10_000,
            "parameter rate must be <= 10000"
        );
        ensure!(options.cycles <= 100, "cycles must be <= 100");
        Ok(options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_bounded_rates_and_rejects_unsafe_or_missing_values() {
        assert_eq!(Options::parse(Vec::new()).unwrap().nodes, 32);
        for arguments in [
            vec!["--seconds", "0"],
            vec!["--nodes", "121"],
            vec!["--cycles", "101"],
            vec!["--parameters-per-second", "10001"],
            vec!["--seconds"],
            vec!["--unknown", "3"],
        ] {
            assert!(Options::parse(arguments.into_iter().map(str::to_owned)).is_err());
        }
    }

    #[test]
    fn confirmed_swaps_is_explicit_and_rejects_non_boolean_values() {
        let options = Options::parse(["--confirmed-swaps".into(), "1".into()]).unwrap();
        assert!(options.confirmed_swaps);
        assert!(!Options::parse(Vec::new()).unwrap().confirmed_swaps);
        assert!(Options::parse(["--confirmed-swaps".into(), "2".into()]).is_err());
    }
}
