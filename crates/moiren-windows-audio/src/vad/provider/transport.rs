//! Worker-side observation and revocation. Engine activation prepares transport;
//! only the lifecycle adapter can open a native gate.
use super::{lifecycle::EndpointControl, *};

pub(super) fn poll<C: EndpointControl>(
    control: &mut C,
    previous: super::super::EndpointSnapshot,
    state: &SessionState,
) -> Result<(super::super::EndpointSnapshot, bool), BackendError> {
    let queried = control.query(previous)?;
    let admitted_before = state.admitted();
    if queried.stream_generation() != previous.stream_generation() {
        state.revoke();
    }
    if queried.stream_generation() != previous.stream_generation()
        || queried.gate_epoch() != previous.gate_epoch()
        || queried.gate_open() != previous.gate_open()
        || queried.stream_state() != previous.stream_state()
        || queried.state() != previous.state()
    {
        log::debug!(
            target: "moiren::audio::vad",
            "event=transport_observed endpoint={:?} previous={previous:?} queried={queried:?} admitted_before={admitted_before} admitted_after={}",
            queried.key(),
            state.admitted(),
        );
    }
    let result = if queried.gate_open() && !state.admitted() {
        log::debug!(
            target: "moiren::audio::vad",
            "event=transport_gate_close endpoint={:?} reason=not_admitted previous={previous:?} queried={queried:?} admitted_before={admitted_before}",
            queried.key(),
        );
        let result = control.gate(queried, false);
        log::debug!(
            target: "moiren::audio::vad",
            "event=transport_gate_close_result endpoint={:?} result={result:?} admitted={}",
            queried.key(),
            state.admitted(),
        );
        result
    } else {
        Ok(queried)
    };
    // Include epochs advanced by our own close, even if no subsequent query
    // observes a difference. This only requests a reset: the PCM owner must be
    // retired and the producer/consumer handshake must finish before reuse.
    let observed = result.as_ref().copied().unwrap_or(queried);
    let changed = observed.stream_generation() != previous.stream_generation()
        || observed.gate_epoch() != previous.gate_epoch();
    if changed {
        state.reset_epoch.fetch_add(1, Ordering::AcqRel);
    }
    let current = result?;
    if current.state() != PdoState::Present {
        return Err(BackendError::ResourceUnavailable(
            "VAD transport endpoint is no longer Present".into(),
        ));
    }
    Ok((current, changed))
}

pub(super) fn after_io_error<C: EndpointControl>(
    control: &mut C,
    previous: super::super::EndpointSnapshot,
    state: &SessionState,
    error: BackendError,
) -> Result<super::super::EndpointSnapshot, BackendError> {
    let (current, changed) = poll(control, previous, state)?;
    log::debug!(
        target: "moiren::audio::vad",
        "event=transport_io_error endpoint={:?} previous={previous:?} current={current:?} epoch_changed={changed} admitted={} error={error:?}",
        current.key(),
        state.admitted(),
    );
    // Gate/stream transitions cancel pending PCM by design. Rebuild its slots
    // from the observed epoch; a same-epoch native failure remains a fault.
    if changed || !current.gate_open() || !state.admitted() {
        Ok(current)
    } else {
        Err(error)
    }
}
