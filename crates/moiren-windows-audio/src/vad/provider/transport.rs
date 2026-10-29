//! Worker-side observation and revocation. Engine activation prepares transport;
//! only the lifecycle adapter can open a native gate.
use super::{lifecycle::EndpointControl, *};

#[cfg(test)]
pub(super) fn poll<C: EndpointControl>(
    control: &mut C,
    previous: super::super::EndpointSnapshot,
    state: &SessionState,
) -> Result<(super::super::EndpointSnapshot, bool), BackendError> {
    poll_with_started_transport(control, previous, state, true)
}

pub(super) fn poll_with_started_transport<C: EndpointControl>(
    control: &mut C,
    previous: super::super::EndpointSnapshot,
    state: &SessionState,
    transport_started: bool,
) -> Result<(super::super::EndpointSnapshot, bool), BackendError> {
    let queried = control.query(previous)?;
    if queried.dropped_packets() != previous.dropped_packets()
        || queried.silence_packets() != previous.silence_packets()
    {
        log::debug!(
            target: "moiren::audio::vad",
            "event=transport_pcm_counters previous={previous:?} queried={queried:?}",
        );
    }
    let admitted_before = state.admitted();
    // Before the first PCM owner exists, the native client is still completing
    // its initial stream transition. There is no retired owner to fence yet,
    // so any stamp change in this one startup window is not recovery. Once an
    // owner has been created, every subsequent change remains a real reset.
    let initial_stream_start = !transport_started;
    if queried.stream_generation() != previous.stream_generation() && !initial_stream_start {
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
    let stream_changed = observed.stream_generation() != previous.stream_generation();
    // Opening a freshly admitted gate advances GateEpoch, but there is no old
    // PCM owner to fence yet. Count later gate changes as recovery; otherwise
    // every normal Engine activation starts with a false epoch reset.
    let opening_gate = !previous.gate_open() && observed.gate_open();
    let changed = (stream_changed
        || (observed.gate_epoch() != previous.gate_epoch() && !opening_gate))
        && !initial_stream_start;
    if changed {
        state.telemetry.recovery();
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
    // Only a collected terminal PCM cancellation can be explained by a gate
    // transition or explicit stop. A device/protocol/timeout fault stays a
    // fault even if shutdown races it; never turn all retirement errors green.
    if error != BackendError::Cancelled {
        log::debug!(
            target: "moiren::audio::vad",
            "event=transport_io_fault endpoint={:?} previous={previous:?} cancel={} admitted={} error={error:?}",
            previous.key(),
            state.cancel.load(Ordering::Acquire),
            state.admitted(),
        );
        return Err(error);
    }
    // A normal stop can race completion collection after the last loop check.
    // Do not require an error-recovery QUERY once shutdown has been requested:
    // run_* still closes the gate, retires all PCM owners and finishes drains.
    if state.cancel.load(Ordering::Acquire) {
        log::debug!(
            target: "moiren::audio::vad",
            "event=transport_pcm_cancel endpoint={:?} previous={previous:?} phase=stopping drained={}",
            previous.key(),
            state.drained.load(Ordering::Acquire),
        );
        return Ok(previous);
    }
    let (current, changed) = poll_with_started_transport(control, previous, state, true)?;
    log::debug!(
        target: "moiren::audio::vad",
        "event=transport_io_error endpoint={:?} previous={previous:?} current={current:?} epoch_changed={changed} cancel={} admitted={} error={error:?}",
        current.key(),
        state.cancel.load(Ordering::Acquire),
        state.admitted(),
    );
    // Native Pause/Closed alone does not prove caller stop intent. Without
    // cancellation, a revoked gate/admission or an observed epoch change,
    // retain the fault instead of restarting same-generation PCM silently.
    if changed || !current.gate_open() || !state.admitted() {
        Ok(current)
    } else {
        Err(error)
    }
}
