use super::model::*;
use std::time::{Duration, Instant};
/// Desired intent and actual ownership have separate identities. Only reaping
/// releases the owner slot, including when a newer request supersedes it.
pub struct ServiceCore {
    generation: SessionGeneration,
    desired: Option<DesiredIntent>,
    owner: Option<SessionGeneration>,
    phase: SessionPhase,
    error: Option<String>,
    running_since: Option<Instant>,
}
enum DesiredIntent {
    Session(SessionSpec),
    // Graph resource bindings are typed in EngineProject, not physical IDs in
    // the legacy two-endpoint summary. Keep lifecycle intent independently.
    Graph { limit: RunLimit },
}
impl DesiredIntent {
    fn limit(&self) -> RunLimit {
        match self {
            Self::Session(spec) => spec.limit,
            Self::Graph { limit } => *limit,
        }
    }
}
impl Default for ServiceCore {
    fn default() -> Self {
        Self {
            generation: SessionGeneration(0),
            desired: None,
            owner: None,
            phase: SessionPhase::Idle,
            error: None,
            running_since: None,
        }
    }
}
impl ServiceCore {
    /// Managed graph updates keep the same service owner identity. Session
    /// controls may update run limits/defaults without pretending it restarted.
    pub(crate) fn managed_desired(&mut self, spec: SessionSpec) -> Result<(), DispatchError> {
        self.managed_graph_desired(Some(spec))
    }
    pub(crate) fn managed_graph_desired(
        &mut self,
        summary: Option<SessionSpec>,
    ) -> Result<(), DispatchError> {
        self.check_open()?;
        if self.desired.is_none() {
            self.restart_managed_graph(summary)?;
        } else {
            self.desired = Some(self.graph_intent(summary)?);
        }
        Ok(())
    }
    pub(crate) fn restart_managed_graph(
        &mut self,
        summary: Option<SessionSpec>,
    ) -> Result<(), DispatchError> {
        self.check_open()?;
        let intent = self.graph_intent(summary)?;
        self.advance()?;
        self.desired = Some(intent);
        self.error = None;
        self.phase = if self.owner.is_some() {
            SessionPhase::Stopping
        } else {
            SessionPhase::Starting
        };
        Ok(())
    }
    fn graph_intent(&self, summary: Option<SessionSpec>) -> Result<DesiredIntent, DispatchError> {
        match summary {
            Some(spec) => {
                spec.validate()?;
                Ok(DesiredIntent::Session(spec))
            }
            None => Ok(DesiredIntent::Graph {
                limit: self.run_limit().unwrap_or(RunLimit::UntilStopped),
            }),
        }
    }
    pub(crate) fn run_limit(&self) -> Option<RunLimit> {
        self.desired.as_ref().map(DesiredIntent::limit)
    }
    /// A managed host can wait for resources and later recover without a new
    /// user request. Keep its intent/identity while projecting actual execution.
    pub(crate) fn managed_execution(
        &mut self,
        generation: SessionGeneration,
        running: bool,
        error: Option<String>,
    ) {
        if self.generation != generation || self.desired.is_none() || self.check_open().is_err() {
            return;
        }
        self.owner = Some(generation);
        self.error = error;
        if running {
            if self.running_since.is_none() {
                self.running_since = Some(Instant::now());
            }
            self.phase = SessionPhase::Running;
        } else {
            self.running_since = None;
            self.phase = if self.error.is_some() {
                SessionPhase::Failed
            } else {
                SessionPhase::Starting
            };
        }
    }
    pub fn update_gain_pan(&mut self, gain: f64, pan: f64) {
        if valid_gain_pan(gain, pan)
            && let Some(DesiredIntent::Session(spec)) = self.desired.as_mut()
        {
            spec.gain = gain;
            spec.pan = pan;
        }
    }
    pub fn start(&mut self, spec: SessionSpec) -> Result<SessionGeneration, DispatchError> {
        self.check_open()?;
        spec.validate()?;
        self.advance()?;
        self.desired = Some(DesiredIntent::Session(spec));
        self.error = None;
        self.phase = if self.owner.is_some() {
            SessionPhase::Stopping
        } else {
            SessionPhase::Starting
        };
        Ok(self.generation)
    }
    pub fn stop(&mut self) -> Result<SessionGeneration, DispatchError> {
        self.check_open()?;
        self.advance()?;
        self.desired = None;
        self.error = None;
        self.phase = if self.owner.is_some() {
            SessionPhase::Stopping
        } else {
            SessionPhase::Idle
        };
        Ok(self.generation)
    }
    fn advance(&mut self) -> Result<(), DispatchError> {
        self.generation = SessionGeneration(
            self.generation
                .0
                .checked_add(1)
                .ok_or(DispatchError::GenerationExhausted)?,
        );
        Ok(())
    }
    fn check_open(&self) -> Result<(), DispatchError> {
        if matches!(self.phase, SessionPhase::Exiting | SessionPhase::Exited) {
            Err(DispatchError::Exiting)
        } else {
            Ok(())
        }
    }
    pub fn may_activate(&self, g: SessionGeneration) -> bool {
        self.phase == SessionPhase::Starting
            && self.generation == g
            && self.desired.is_some()
            && self.owner.is_none()
    }
    pub fn activated(&mut self, g: SessionGeneration) -> bool {
        if !self.may_activate(g) {
            return false;
        }
        self.owner = Some(g);
        true
    }
    /// Test-orchestration primitive for the session lifecycle: transitions an
    /// owned, activating generation into Running. The managed host drives the
    /// same transition through `managed_execution`, so production dispatch
    /// never calls this directly.
    pub fn owner_started(&mut self, g: SessionGeneration) -> bool {
        if self.phase != SessionPhase::Starting
            || self.generation != g
            || self.owner != Some(g)
            || self.desired.is_none()
        {
            return false;
        }
        self.phase = SessionPhase::Running;
        self.running_since = Some(Instant::now());
        true
    }
    pub fn owner_reaped(&mut self, g: SessionGeneration) -> bool {
        if self.owner != Some(g) {
            return false;
        }
        self.owner = None;
        self.running_since = None;
        if self.phase != SessionPhase::Exiting {
            self.phase = if self.desired.is_some() {
                SessionPhase::Starting
            } else if self.error.is_some() {
                SessionPhase::Failed
            } else {
                SessionPhase::Idle
            };
        }
        true
    }
    pub fn failed(&mut self, g: SessionGeneration, error: String) -> bool {
        if self.generation != g || self.check_open().is_err() {
            return false;
        }
        self.error = Some(error);
        self.desired = None;
        self.phase = if self.owner.is_some() {
            SessionPhase::Stopping
        } else {
            SessionPhase::Failed
        };
        true
    }
    pub fn begin_exit(&mut self) {
        if self.phase == SessionPhase::Exited {
            return;
        }
        self.desired = None;
        self.phase = SessionPhase::Exiting;
    }
    pub fn finish_exit(&mut self) -> bool {
        if self.phase != SessionPhase::Exiting || self.owner.is_some() {
            return false;
        }
        self.phase = SessionPhase::Exited;
        true
    }
    pub fn phase(&self) -> SessionPhase {
        self.phase
    }
    pub fn generation(&self) -> SessionGeneration {
        self.generation
    }
    pub fn owner_generation(&self) -> Option<SessionGeneration> {
        self.owner
    }
    /// Legacy single-input/output summary only. A managed graph may be running
    /// without one; its actual resources and revision live in the graph host.
    pub fn desired(&self) -> Option<&SessionSpec> {
        match &self.desired {
            Some(DesiredIntent::Session(spec)) => Some(spec),
            _ => None,
        }
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn elapsed(&self) -> Duration {
        self.running_since.map_or(Duration::ZERO, |t| t.elapsed())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graph_intent_retains_identity_until_restart_and_stop_revokes_it() {
        let mut c = ServiceCore::default();
        c.managed_graph_desired(None).unwrap();
        let first = c.generation();
        assert!(c.desired().is_none());
        assert!(c.may_activate(first));
        c.managed_execution(first, true, None);
        c.managed_graph_desired(None).unwrap();
        assert_eq!(c.generation(), first);
        assert_eq!(c.phase(), SessionPhase::Running);
        c.restart_managed_graph(None).unwrap();
        let next = c.generation();
        assert_ne!(next, first);
        assert_eq!(c.phase(), SessionPhase::Stopping);
        assert!(!c.may_activate(next));
        assert!(c.owner_reaped(first));
        assert!(c.may_activate(next));
        c.managed_execution(first, true, None);
        assert_eq!(c.owner_generation(), None);
        c.managed_execution(next, true, None);
        c.stop().unwrap();
        c.managed_execution(next, true, None);
        assert_eq!(c.phase(), SessionPhase::Stopping);
        c.owner_reaped(next);
        assert_eq!(c.phase(), SessionPhase::Idle);
    }
    #[test]
    fn graph_intent_preserves_explicit_run_limit_and_exit_cannot_restart() {
        let mut c = ServiceCore::default();
        c.start(SessionSpec {
            source: InputSelection::Tone {
                frequency_hz: 440.0,
            },
            output_endpoint_id: "physical-output".into(),
            limit: RunLimit::Seconds(12),
            gain: 1.0,
            pan: 0.0,
            max_block_frames: 256,
        })
        .unwrap();
        c.managed_graph_desired(None).unwrap();
        assert_eq!(c.run_limit(), Some(RunLimit::Seconds(12)));
        c.restart_managed_graph(None).unwrap();
        assert_eq!(c.run_limit(), Some(RunLimit::Seconds(12)));
        c.stop().unwrap();
        c.managed_graph_desired(None).unwrap();
        assert_eq!(c.run_limit(), Some(RunLimit::UntilStopped));
        c.begin_exit();
        assert_eq!(c.managed_graph_desired(None), Err(DispatchError::Exiting));
        assert_eq!(c.restart_managed_graph(None), Err(DispatchError::Exiting));
        assert!(c.finish_exit());
    }
    #[test]
    fn exit_revokes_permits_even_when_generation_is_exhausted() {
        let mut c = ServiceCore {
            generation: SessionGeneration(u64::MAX),
            ..ServiceCore::default()
        };
        assert_eq!(c.stop(), Err(DispatchError::GenerationExhausted));
        c.begin_exit();
        assert_eq!(c.phase(), SessionPhase::Exiting);
        assert!(!c.may_activate(SessionGeneration(u64::MAX)));
        assert_eq!(c.stop(), Err(DispatchError::Exiting));
    }
}
