use moiren_engine::host::BackendError;

pub(super) fn validate_frequency(frequency: f64) -> Result<(), BackendError> {
    if frequency.is_finite() && (0.0..24_000.0).contains(&frequency) && frequency > 0.0 {
        Ok(())
    } else {
        Err(BackendError::Unsupported(
            "tone frequency must be finite and between 0 and 24kHz".into(),
        ))
    }
}

pub(super) fn tone(name: &str) -> Result<f64, BackendError> {
    let frequency = name
        .strip_prefix("moiren:tone:")
        .and_then(|value| value.parse::<f64>().ok())
        .ok_or_else(|| {
            BackendError::Unsupported("use tone_input for a native software source".into())
        })?;
    validate_frequency(frequency)?;
    Ok(frequency)
}

pub(super) fn process(selector: &str) -> Result<(u32, u64), BackendError> {
    let (pid, creation) = selector
        .strip_prefix("moiren:process:")
        .and_then(|value| value.split_once(':'))
        .ok_or_else(|| {
            BackendError::Unsupported("use process_input with a selected process identity".into())
        })?;
    let pid = pid.parse::<u32>().ok().filter(|pid| *pid != 0);
    let creation = creation
        .parse::<u64>()
        .ok()
        .filter(|creation| *creation != 0);
    pid.zip(creation)
        .ok_or_else(|| BackendError::Unsupported("invalid selected process identity".into()))
}
