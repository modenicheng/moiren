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
    /// Content token for the snapshot publisher: bumped by every mutation of
    /// the published fields (generation, phase, owner, desired, error). All
    /// writes live in this file, so each method keeps its bump in step with
    /// the writes it performs.
    state_epoch: u64,
}
#[derive(PartialEq)]
enum DesiredIntent {
    Session(SessionSpec),
    // Graph resource bindings are typed in EngineProject, not physical IDs in
    // the legacy two-endpoint summary. Keep lifecycle intent independently.
    Graph,
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
            state_epoch: 0,
        }
    }
}
impl ServiceCore {
    /// Content token advanced by every published-field mutation below.
    pub fn state_epoch(&self) -> u64 {
        self.state_epoch
    }
    fn bump(&mut self) {
        self.state_epoch = self.state_epoch.wrapping_add(1);
    }
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
            let intent = self.graph_intent(summary)?;
            if self.desired.as_ref() != Some(&intent) {
                self.desired = Some(intent);
                self.bump();
            }
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
        // The generation advanced: a real content change.
        self.bump();
        Ok(())
    }
    fn graph_intent(&self, summary: Option<SessionSpec>) -> Result<DesiredIntent, DispatchError> {
        match summary {
            Some(spec) => {
                spec.validate()?;
                Ok(DesiredIntent::Session(spec))
            }
            None => Ok(DesiredIntent::Graph),
        }
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
        // Projected every service round while an owner exists; the settled
        // steady state re-reports the same facts and must not advance the
        // content token.
        let mut changed = self.owner != Some(generation) || self.error != error;
        self.owner = Some(generation);
        self.error = error;
        if running {
            if self.running_since.is_none() {
                self.running_since = Some(Instant::now());
                changed = true;
            }
            changed |= self.phase != SessionPhase::Running;
            self.phase = SessionPhase::Running;
        } else {
            let phase = if self.error.is_some() {
                SessionPhase::Failed
            } else {
                SessionPhase::Starting
            };
            changed |= self.running_since.is_some() || self.phase != phase;
            self.running_since = None;
            self.phase = phase;
        }
        if changed {
            self.bump();
        }
    }
    pub fn update_gain_pan(&mut self, gain: f64, pan: f64) {
        if valid_gain_pan(gain, pan)
            && let Some(DesiredIntent::Session(spec)) = self.desired.as_mut()
            && (spec.gain != gain || spec.pan != pan)
        {
            spec.gain = gain;
            spec.pan = pan;
            self.bump();
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
        // The generation advanced: a real content change.
        self.bump();
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
        self.bump();
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
        self.bump();
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
        // The owned generation always clears here: a real content change.
        self.bump();
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
        // Session failure is an event, not a per-round projection; the
        // publish-side dedup absorbs any content-equivalent repeat.
        self.bump();
        true
    }
    pub fn begin_exit(&mut self) {
        if self.phase == SessionPhase::Exited {
            return;
        }
        let changed = self.desired.is_some() || self.phase != SessionPhase::Exiting;
        self.desired = None;
        self.phase = SessionPhase::Exiting;
        if changed {
            self.bump();
        }
    }
    pub fn finish_exit(&mut self) -> bool {
        if self.phase != SessionPhase::Exiting || self.owner.is_some() {
            return false;
        }
        self.phase = SessionPhase::Exited;
        self.bump();
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
    fn graph_intent_retains_identity_until_restart_and_exit_revokes_it() {
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
        // Only exit revokes the intent: the session has no stop path.
        c.begin_exit();
        c.managed_execution(next, true, None);
        assert_eq!(c.phase(), SessionPhase::Exiting);
        c.owner_reaped(next);
        assert_eq!(c.phase(), SessionPhase::Exiting);
    }
    #[test]
    fn session_intent_survives_restart_and_exit_cannot_restart() {
        let mut c = ServiceCore::default();
        c.start(SessionSpec {
            source: InputSelection::Tone {
                frequency_hz: 440.0,
            },
            output_endpoint_id: "physical-output".into(),
            gain: 1.0,
            pan: 0.0,
            max_block_frames: 256,
        })
        .unwrap();
        assert!(c.desired().is_some());
        c.managed_graph_desired(None).unwrap();
        // A graph intent replaces the legacy summary; only the lifecycle
        // identity matters from here on.
        assert!(c.desired().is_none());
        c.restart_managed_graph(None).unwrap();
        assert!(c.desired().is_none());
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
        assert_eq!(
            c.start(SessionSpec {
                source: InputSelection::Tone {
                    frequency_hz: 440.0,
                },
                output_endpoint_id: "physical-output".into(),
                gain: 1.0,
                pan: 0.0,
                max_block_frames: 256,
            }),
            Err(DispatchError::GenerationExhausted)
        );
        c.begin_exit();
        assert_eq!(c.phase(), SessionPhase::Exiting);
        assert!(!c.may_activate(SessionGeneration(u64::MAX)));
    }
}
