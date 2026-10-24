use anyhow::ensure;
use moiren_windows_audio::vad::{EndpointKey, PdoState};
#[cfg(windows)]
use moiren_windows_audio::vad::{Intent, StableEndpointId};

// Shared identity.rs operates on this shape; matching never parses MMDevice IDs.
#[cfg(windows)]
#[derive(Clone, Debug)]
pub(super) struct Candidate {
    pub id: String,
    pub raw_instance: String,
    pub parent_instance: String,
    pub intent: Intent,
}

#[cfg(windows)]
pub(super) fn select<'a>(
    candidates: &'a [Candidate],
    stable_id: StableEndpointId,
    root: &str,
) -> anyhow::Result<Option<&'a Candidate>> {
    let stable_text = stable_id.to_string();
    let mut matching = candidates.iter().filter(|candidate| {
        candidate
            .raw_instance
            .rsplit_once('\\')
            .is_some_and(|(hardware, instance)| {
                hardware.eq_ignore_ascii_case(r"MOIREN\VAD_RAW")
                    && instance
                        .rsplit('&')
                        .next()
                        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(&stable_text))
            })
            && candidate.parent_instance.eq_ignore_ascii_case(root)
            && candidate.intent == Intent::Microphone
    });
    let first = matching.next();
    ensure!(
        matching.next().is_none(),
        "ambiguous owned microphone identity"
    );
    Ok(first)
}

pub(super) fn require_retiring(state: PdoState, gate_open: bool) -> anyhow::Result<()> {
    ensure!(
        matches!(state, PdoState::Removing | PdoState::Removed) && !gate_open,
        "removal must retain closed admission until final Removed: {state:?}, gate={gate_open}"
    );
    Ok(())
}

pub(super) fn require_fresh_runtime(
    previous: EndpointKey,
    next: EndpointKey,
) -> anyhow::Result<()> {
    ensure!(
        next.session_epoch() == previous.session_epoch()
            && next.pdo_token() != previous.pdo_token()
            && next.endpoint_generation() > previous.endpoint_generation(),
        "same-ID recreate must advance runtime identity without opening a new session"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_windows_audio::vad::RuntimePdoToken;

    fn key(token: u64, session: u64, generation: u64) -> EndpointKey {
        EndpointKey::new(RuntimePdoToken::new(token).unwrap(), session, generation).unwrap()
    }

    #[test]
    fn accepted_removal_does_not_mean_destroyed() {
        assert!(require_retiring(PdoState::Removing, false).is_ok());
        assert!(require_retiring(PdoState::Removed, false).is_ok());
        for state in [PdoState::Present, PdoState::Creating] {
            assert!(require_retiring(state, false).is_err());
        }
        assert!(require_retiring(PdoState::Removing, true).is_err());
    }

    #[test]
    fn recreate_requires_fresh_token_generation_and_same_owner_session() {
        let previous = key(1, 7, 1);
        assert!(require_fresh_runtime(previous, key(2, 7, 2)).is_ok());
        for next in [key(1, 7, 2), key(2, 7, 1), key(2, 8, 2)] {
            assert!(require_fresh_runtime(previous, next).is_err());
        }
    }

    #[cfg(windows)]
    #[test]
    fn only_exact_owned_microphone_identity_is_eligible() {
        let stable = StableEndpointId::new(42, 7);
        let root = r"ROOT\MOIRENVAD\0000";
        let candidate = || Candidate {
            id: "opaque endpoint".into(),
            raw_instance: format!(r"MOIREN\VAD_RAW\7&ABCD&0&{stable}"),
            parent_instance: root.into(),
            intent: Intent::Microphone,
        };
        assert!(select(&[candidate()], stable, root).unwrap().is_some());
        for row in [
            Candidate {
                parent_instance: "other root".into(),
                ..candidate()
            },
            Candidate {
                intent: Intent::Playback,
                ..candidate()
            },
            Candidate {
                raw_instance: format!(r"OTHER\VAD_RAW\{stable}"),
                ..candidate()
            },
        ] {
            assert!(select(&[row], stable, root).unwrap().is_none());
        }
        assert!(select(&[candidate(), candidate()], stable, root).is_err());
    }
}
