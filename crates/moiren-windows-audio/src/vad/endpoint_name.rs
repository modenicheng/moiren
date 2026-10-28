//! Explicit control-worker Windows endpoint name evidence.

use super::{MAX_ENDPOINTS, StableEndpointId, VadExportDiagnosticProvenance, VadExportObservation};
use moiren_engine::node::VirtualEndpointId;
use std::{collections::BTreeMap, time::Duration};

const NAME_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const NEGATIVE_RETRY_INTERVAL: Duration = Duration::from_secs(1);
/// Cap on attributed owned-enumeration errors retained in one observation.
pub(super) const MAX_REPORTED_OWNED_ERRORS: usize = 8;

/// Detached evidence from an exact Windows endpoint property read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VadEndpointNameObservation {
    pub endpoint_id: VirtualEndpointId,
    /// Runtime PDO token, session epoch, and endpoint generation.
    pub native_identity: [u64; 3],
    pub observed_qpc_100ns: Option<u64>,
    pub mmdevice_id: Option<String>,
    /// Raw PKEY_Device_FriendlyName, never the requested/provider name.
    pub name: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone)]
struct CacheEntry {
    identity: [u64; 3],
    observation: VadEndpointNameObservation,
    observed_at: Duration,
    retry_at: Duration,
}

#[derive(Default)]
pub(super) struct NameObserver {
    cache: BTreeMap<VirtualEndpointId, CacheEntry>,
}

impl NameObserver {
    pub(super) fn observe<R: EndpointNameReader, C: ObserverClock>(
        &mut self,
        exports: &[VadExportObservation],
        reader: &mut R,
        clock: &mut C,
    ) -> Vec<VadEndpointNameObservation> {
        let now = clock.now();
        let mut output = BTreeMap::new();
        let mut eligible = BTreeMap::new();
        let mut pending = Vec::new();

        for export in exports.iter().take(MAX_ENDPOINTS as usize) {
            let Some(snapshot) = export.snapshot else {
                continue;
            };
            if export.removing
                || export.error.is_some()
                || export.diagnostic_provenance
                    == VadExportDiagnosticProvenance::CachedAfterQueryFailure
                || snapshot.state() != super::PdoState::Present
            {
                continue;
            }

            let identity = [
                snapshot.pdo_token().get(),
                snapshot.session_epoch(),
                snapshot.endpoint_generation(),
            ];
            eligible.insert(export.endpoint_id, identity);

            if let Some(cached) = self.cache.get(&export.endpoint_id)
                && cached.identity == identity
                && if cached.observation.name.is_some() {
                    now.saturating_sub(cached.observed_at) < NAME_REFRESH_INTERVAL
                } else {
                    now < cached.retry_at
                }
            {
                output.insert(export.endpoint_id, cached.observation.clone());
                continue;
            }

            pending.push((
                export.endpoint_id,
                identity,
                super::StableEndpointId::from_virtual_endpoint_id(export.endpoint_id),
                export.intent,
            ));
        }

        if !pending.is_empty() {
            let requests: Vec<_> = pending
                .iter()
                .map(|(id, _, stable_id, intent)| (*id, *stable_id, *intent))
                .collect();
            let results = reader.read(&requests);
            let observed_at = clock.now();
            for (index, (endpoint_id, identity, _, _)) in pending.into_iter().enumerate() {
                let result = results
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| Err("endpoint reader returned too few results".into()));
                let observation = match result {
                    Ok((mmdevice_id, name)) => VadEndpointNameObservation {
                        endpoint_id,
                        native_identity: identity,
                        observed_qpc_100ns: clock.qpc_100ns(),
                        mmdevice_id: Some(mmdevice_id),
                        name: Some(name),
                        error: None,
                    },
                    Err(error) => VadEndpointNameObservation {
                        endpoint_id,
                        native_identity: identity,
                        observed_qpc_100ns: clock.qpc_100ns(),
                        mmdevice_id: None,
                        name: None,
                        error: Some(error),
                    },
                };
                self.cache.insert(
                    endpoint_id,
                    CacheEntry {
                        identity,
                        observation: observation.clone(),
                        observed_at,
                        retry_at: observed_at + NEGATIVE_RETRY_INTERVAL,
                    },
                );
                output.insert(endpoint_id, observation);
            }
        }

        self.cache
            .retain(|endpoint_id, entry| eligible.get(endpoint_id) == Some(&entry.identity));
        exports
            .iter()
            .take(MAX_ENDPOINTS as usize)
            .filter_map(|export| output.remove(&export.endpoint_id))
            .collect()
    }
}

pub(super) trait EndpointNameReader {
    fn read(
        &mut self,
        requests: &[(VirtualEndpointId, super::StableEndpointId, super::Intent)],
    ) -> Vec<Result<(String, String), String>>;
}

pub(super) trait ObserverClock {
    fn now(&mut self) -> Duration;
    fn qpc_100ns(&mut self) -> Option<u64>;
}

#[cfg(windows)]
mod windows_reader;

#[cfg(windows)]
pub(crate) use windows_reader::{WindowsEndpointNameReader, WindowsObserverClock};

#[derive(Debug)]
struct Candidate {
    mmdevice_id: String,
    raw_instance: String,
    root_instance: String,
    intent: super::Intent,
}

/// Outcome of deciding whether a failed enumeration candidate belongs to the
/// owned VAD device. `Undecidable` means the optional identity lookup itself
/// failed; such an endpoint stays unattributed and never fails the name read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CandidateOwnership {
    Owned,
    NotOwned,
    Undecidable,
}

/// Join the errors of enumeration candidates that are confirmed owned. The
/// exact-identity `select` below remains the only source of a successful name;
/// an undecidable hint must not turn into a false observation error about the
/// attribution machinery itself.
pub(super) fn owned_candidate_errors(
    candidates: &[(u32, String, CandidateOwnership)],
    reported_limit: usize,
) -> Vec<String> {
    let mut reports = Vec::new();
    let mut omitted = 0;
    for (index, error, ownership) in candidates.iter() {
        if *ownership != CandidateOwnership::Owned {
            continue;
        }
        if reports.len() < reported_limit {
            reports.push(format!("MMDevice index {index}: {error}"));
        } else {
            omitted += 1;
        }
    }
    if omitted != 0 {
        reports.push(format!(
            "{omitted} additional owned endpoint topology errors omitted"
        ));
    }
    reports
}

fn select<'a>(
    candidates: &'a [Candidate],
    stable_id: super::StableEndpointId,
    intent: super::Intent,
    root: &str,
) -> Result<&'a Candidate, String> {
    let stable_matches: Vec<_> = candidates
        .iter()
        .filter(|candidate| raw_instance_has_identity(&candidate.raw_instance, stable_id))
        .collect();
    let mut matches = stable_matches.iter().copied().filter(|candidate| {
        candidate.root_instance.eq_ignore_ascii_case(root) && candidate.intent == intent
    });
    let Some(first) = matches.next() else {
        let candidates = stable_matches
            .iter()
            .take(8)
            .map(|candidate| {
                format!(
                    "flow={:?}, root={:?}, pnp={:?}",
                    candidate.intent, candidate.root_instance, candidate.raw_instance
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        let omitted = stable_matches.len().saturating_sub(8);
        return Err(format!(
            "no exact Windows endpoint matches the owned VAD identity (requested flow={intent:?}, root={root:?}, stable_id={stable_id}; stable-ID candidates=[{candidates}], omitted={omitted})"
        ));
    };
    if matches.next().is_some() {
        return Err("multiple MMDevices map to the owned raw endpoint".into());
    }
    Ok(first)
}

fn raw_instance_has_identity(raw_instance: &str, stable_id: StableEndpointId) -> bool {
    let Some((hardware_id, instance)) = raw_instance.rsplit_once('\\') else {
        return false;
    };
    hardware_id.eq_ignore_ascii_case(r"MOIREN\VAD_RAW")
        && instance
            .rsplit('&')
            .next()
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case(&stable_id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vad::{Channels, EndpointSnapshot, Intent, PdoState, StableEndpointId, StreamState};

    const ROOT: &str = r"ROOT\MOIRENVAD\0001";

    fn id(byte: u8) -> VirtualEndpointId {
        VirtualEndpointId::from_bytes([byte; 16])
    }

    fn candidate(
        stable: StableEndpointId,
        root: &str,
        intent: Intent,
        mmdevice_id: &str,
    ) -> Candidate {
        Candidate {
            mmdevice_id: mmdevice_id.into(),
            raw_instance: format!(r"MOIREN\VAD_RAW\6&opaque&{stable}"),
            root_instance: root.into(),
            intent,
        }
    }

    fn snapshot(endpoint_id: VirtualEndpointId, state: PdoState) -> EndpointSnapshot {
        snapshot_with_identity(
            endpoint_id,
            state,
            100 + u64::from(endpoint_id.to_bytes()[0]),
            7,
        )
    }

    fn snapshot_with_identity(
        endpoint_id: VirtualEndpointId,
        state: PdoState,
        pdo_token: u64,
        session_epoch: u64,
    ) -> EndpointSnapshot {
        fn put32(bytes: &mut [u8], offset: usize, value: u32) {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        fn put64(bytes: &mut [u8], offset: usize, value: u64) {
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }

        let mut capabilities = [0_u8; 64];
        for (offset, value) in [
            (0, 64),
            (4, 2),
            (8, 31),
            (12, 64),
            (16, 1),
            (20, 48_000),
            (24, 3),
            (28, 6),
            (32, 6),
            (36, 4),
            (48, 4_800),
        ] {
            put32(&mut capabilities, offset, value);
        }
        put64(&mut capabilities, 40, session_epoch);
        let capabilities = crate::vad::Capabilities::decode(&capabilities).unwrap();
        let generation = pdo_token;
        let mut bytes = [0_u8; 128];
        for (offset, value) in [
            (0, 128),
            (4, 2),
            (16, state as u32),
            (20, Intent::Playback as u32),
            (24, Channels::Stereo as u32),
            (32, 1),
            (36, u32::from(state == PdoState::Present)),
            (88, 0),
            (92, 0),
            (100, StreamState::Closed as u32),
        ] {
            put32(&mut bytes, offset, value);
        }
        for (offset, value) in [(8, pdo_token), (40, session_epoch), (48, generation)] {
            put64(&mut bytes, offset, value);
        }
        bytes[72..88].copy_from_slice(
            &StableEndpointId::from_virtual_endpoint_id(endpoint_id).to_le_bytes(),
        );
        EndpointSnapshot::decode(&bytes, capabilities).unwrap()
    }

    fn export(
        endpoint_id: VirtualEndpointId,
        state: PdoState,
        removing: bool,
        error: Option<&str>,
        provenance: VadExportDiagnosticProvenance,
    ) -> VadExportObservation {
        VadExportObservation {
            endpoint_id,
            spec_name: "requested fallback must not be used".into(),
            intent: Intent::Playback,
            channels: Channels::Stereo,
            allow_default: false,
            snapshot: Some(snapshot(endpoint_id, state)),
            removing,
            transport_prepared: false,
            telemetry: None,
            error: error.map(str::to_owned),
            diagnostic_provenance: provenance,
        }
    }

    #[test]
    fn exact_root_stable_id_and_flow_select_the_owned_endpoint() {
        let stable_id = StableEndpointId::from_virtual_endpoint_id(id(0x42));
        let candidates = [candidate(
            stable_id,
            ROOT,
            Intent::Playback,
            "opaque MMDevice ID",
        )];
        assert_eq!(
            select(&candidates, stable_id, Intent::Playback, ROOT)
                .unwrap()
                .mmdevice_id,
            "opaque MMDevice ID"
        );
    }

    #[test]
    fn undecidable_ownership_hints_are_not_reported_as_observation_errors() {
        // Regression: a failing optional identity lookup (for example
        // GetDevice(error attribution) 0xE0000225 on a removing endpoint) used
        // to abort the whole name read; it must stay unattributed instead.
        let hints = vec![
            (
                0,
                "topology walk failed".into(),
                super::CandidateOwnership::Undecidable,
            ),
            (
                1,
                "adapter GetDevice failed".into(),
                super::CandidateOwnership::NotOwned,
            ),
        ];
        assert_eq!(
            super::owned_candidate_errors(&hints, super::MAX_REPORTED_OWNED_ERRORS),
            Vec::<String>::new()
        );
    }

    #[test]
    fn confirmed_owned_candidate_errors_are_attributed_with_index_and_limit() {
        let hints = vec![
            (
                0,
                "connector count is 3, expected 1".into(),
                super::CandidateOwnership::Owned,
            ),
            (1, "unrelated".into(), super::CandidateOwnership::NotOwned),
            (
                2,
                "adapter property missing".into(),
                super::CandidateOwnership::Owned,
            ),
        ];
        assert_eq!(
            super::owned_candidate_errors(&hints, 1),
            vec![
                "MMDevice index 0: connector count is 3, expected 1".to_owned(),
                "1 additional owned endpoint topology errors omitted".to_owned(),
            ]
        );
    }

    #[test]
    fn wrong_root_stable_id_and_flow_are_not_selected() {
        let stable_id = StableEndpointId::from_virtual_endpoint_id(id(0x42));
        for (candidate, root, intent) in [
            (
                candidate(
                    stable_id,
                    r"ROOT\OTHER\0001",
                    Intent::Playback,
                    "wrong root",
                ),
                ROOT,
                Intent::Playback,
            ),
            (
                candidate(
                    StableEndpointId::from_virtual_endpoint_id(id(0x43)),
                    ROOT,
                    Intent::Playback,
                    "wrong stable id",
                ),
                ROOT,
                Intent::Playback,
            ),
            (
                candidate(stable_id, ROOT, Intent::Microphone, "wrong flow"),
                ROOT,
                Intent::Playback,
            ),
        ] {
            let error = select(&[candidate], stable_id, intent, root).unwrap_err();
            assert!(error.contains("no exact Windows endpoint"));
            assert!(error.contains("stable-ID candidates"));
        }
    }

    #[test]
    fn duplicate_exact_endpoint_matches_are_rejected_as_ambiguous() {
        let stable_id = StableEndpointId::from_virtual_endpoint_id(id(0x42));
        let candidates = [
            candidate(stable_id, ROOT, Intent::Playback, "opaque one"),
            candidate(stable_id, ROOT, Intent::Playback, "opaque two"),
        ];
        assert!(
            select(&candidates, stable_id, Intent::Playback, ROOT)
                .unwrap_err()
                .contains("multiple MMDevices")
        );
    }

    struct FakeReader {
        calls: usize,
        result: Result<(String, String), String>,
    }

    impl Default for FakeReader {
        fn default() -> Self {
            Self {
                calls: 0,
                result: Err("reader result not configured".into()),
            }
        }
    }

    impl EndpointNameReader for FakeReader {
        fn read(
            &mut self,
            requests: &[(VirtualEndpointId, StableEndpointId, Intent)],
        ) -> Vec<Result<(String, String), String>> {
            self.calls += 1;
            requests.iter().map(|_| self.result.clone()).collect()
        }
    }

    struct FakeClock {
        now: Duration,
        qpc: u64,
    }

    impl ObserverClock for FakeClock {
        fn now(&mut self) -> Duration {
            self.now
        }

        fn qpc_100ns(&mut self) -> Option<u64> {
            self.qpc += 100;
            Some(self.qpc)
        }
    }

    fn fresh_export(endpoint_id: VirtualEndpointId) -> VadExportObservation {
        export(
            endpoint_id,
            PdoState::Present,
            false,
            None,
            VadExportDiagnosticProvenance::Fresh,
        )
    }

    #[test]
    fn unchanged_identity_cache_keeps_original_qpc_timestamp_until_one_second() {
        let endpoint_id = id(0x42);
        let mut observer = NameObserver::default();
        let mut reader = FakeReader {
            calls: 0,
            result: Ok(("opaque device ID".into(), "Exact raw name".into())),
        };
        let mut clock = FakeClock {
            now: Duration::ZERO,
            qpc: 500,
        };

        let first = observer.observe(&[fresh_export(endpoint_id)], &mut reader, &mut clock);
        clock.now += Duration::from_millis(900);
        let cached = observer.observe(&[fresh_export(endpoint_id)], &mut reader, &mut clock);
        assert_eq!(reader.calls, 1);
        assert_eq!(cached, first);
        assert_eq!(cached[0].observed_qpc_100ns, Some(600));
    }

    #[test]
    fn new_native_identity_resamples_immediately() {
        let endpoint_id = id(0x42);
        let mut observer = NameObserver::default();
        let mut reader = FakeReader {
            calls: 0,
            result: Ok(("opaque device ID".into(), "Exact raw name".into())),
        };
        let mut clock = FakeClock {
            now: Duration::ZERO,
            qpc: 500,
        };
        observer.observe(&[fresh_export(endpoint_id)], &mut reader, &mut clock);
        let mut replacement = fresh_export(endpoint_id);
        // Changing the runtime token models a new PDO/session identity while
        // preserving the stable endpoint ID and the exact same configured spec.
        replacement.snapshot = Some(snapshot_with_identity(
            endpoint_id,
            PdoState::Present,
            201,
            7,
        ));
        clock.now += Duration::from_millis(10);
        let result = observer.observe(&[replacement], &mut reader, &mut clock);
        assert_eq!(reader.calls, 2);
        assert_eq!(result[0].native_identity[0], 201);
    }

    #[test]
    fn negative_observations_retry_after_finite_deadline() {
        let endpoint_id = id(0x42);
        let mut observer = NameObserver::default();
        let mut reader = FakeReader {
            calls: 0,
            result: Err("MMDevice endpoint is not present yet".into()),
        };
        let mut clock = FakeClock {
            now: Duration::ZERO,
            qpc: 700,
        };
        let first = observer.observe(&[fresh_export(endpoint_id)], &mut reader, &mut clock);
        assert!(first[0].name.is_none());
        clock.now += Duration::from_millis(900);
        assert_eq!(
            observer.observe(&[fresh_export(endpoint_id)], &mut reader, &mut clock),
            first
        );
        assert_eq!(reader.calls, 1);
        clock.now += Duration::from_millis(101);
        let retried = observer.observe(&[fresh_export(endpoint_id)], &mut reader, &mut clock);
        assert_eq!(reader.calls, 2);
        assert_ne!(retried[0].observed_qpc_100ns, first[0].observed_qpc_100ns);
    }

    #[test]
    fn removing_removed_and_failed_queries_invalidate_cached_success() {
        let endpoint_id = id(0x42);
        let mut observer = NameObserver::default();
        let mut reader = FakeReader {
            calls: 0,
            result: Ok(("opaque device ID".into(), "Exact raw name".into())),
        };
        let mut clock = FakeClock {
            now: Duration::ZERO,
            qpc: 800,
        };
        observer.observe(&[fresh_export(endpoint_id)], &mut reader, &mut clock);
        assert!(
            observer
                .observe(
                    &[export(
                        endpoint_id,
                        PdoState::Present,
                        true,
                        None,
                        VadExportDiagnosticProvenance::Fresh
                    )],
                    &mut reader,
                    &mut clock,
                )
                .is_empty()
        );
        let recovered = observer.observe(&[fresh_export(endpoint_id)], &mut reader, &mut clock);
        assert_eq!(reader.calls, 2);
        assert_eq!(recovered[0].observed_qpc_100ns, Some(1000));

        observer.observe(
            &[export(
                endpoint_id,
                PdoState::Removed,
                true,
                None,
                VadExportDiagnosticProvenance::Fresh,
            )],
            &mut reader,
            &mut clock,
        );
        observer.observe(&[fresh_export(endpoint_id)], &mut reader, &mut clock);
        assert_eq!(reader.calls, 3);

        let failed = export(
            endpoint_id,
            PdoState::Present,
            false,
            Some("QUERY failed"),
            VadExportDiagnosticProvenance::CachedAfterQueryFailure,
        );
        assert!(
            observer
                .observe(&[failed], &mut reader, &mut clock)
                .is_empty()
        );
        observer.observe(&[fresh_export(endpoint_id)], &mut reader, &mut clock);
        assert_eq!(reader.calls, 4);
    }

    #[test]
    fn cache_retention_is_bounded_to_current_eligible_exports() {
        let first_id = id(0x42);
        let second_id = id(0x43);
        let mut observer = NameObserver::default();
        let mut reader = FakeReader {
            calls: 0,
            result: Ok(("opaque device ID".into(), "Exact raw name".into())),
        };
        let mut clock = FakeClock {
            now: Duration::ZERO,
            qpc: 900,
        };
        observer.observe(&[fresh_export(first_id)], &mut reader, &mut clock);
        observer.observe(&[fresh_export(second_id)], &mut reader, &mut clock);
        assert_eq!(observer.cache.len(), 1);
        assert!(observer.cache.contains_key(&second_id));
        assert!(!observer.cache.contains_key(&first_id));
    }
}
