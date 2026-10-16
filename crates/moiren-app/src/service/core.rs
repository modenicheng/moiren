use super::model::*;
use std::time::{Duration, Instant};
/// Desired intent and actual ownership have separate identities. Only reaping
/// releases the owner slot, including when a newer request supersedes it.
pub struct ServiceCore {
    generation: SessionGeneration,
    desired: Option<SessionSpec>,
    owner: Option<SessionGeneration>,
    phase: SessionPhase,
    error: Option<String>,
    running_since: Option<Instant>,
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
        self.check_open()?;
        spec.validate()?;
        if self.desired.is_none() {
            self.start(spec)?;
        } else {
            self.desired = Some(spec);
        }
        Ok(())
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
            && let Some(spec) = self.desired.as_mut()
        {
            spec.gain = gain;
            spec.pan = pan;
        }
    }
    pub fn start_failed(
        &mut self,
        generation: SessionGeneration,
        error: super::BackendError,
    ) -> bool {
        self.failed(generation, error.to_string())
    }
    pub fn start(&mut self, spec: SessionSpec) -> Result<SessionGeneration, DispatchError> {
        self.check_open()?;
        spec.validate()?;
        self.advance()?;
        self.desired = Some(spec);
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
    /// Natural completion consumes only this owner's unchanged intent. A newer
    /// requested session remains queued and no generation counter is advanced.
    pub fn owner_completed(&mut self, g: SessionGeneration) {
        if self.generation == g && self.owner == Some(g) && self.check_open().is_ok() {
            self.desired = None;
        }
    }
    pub fn owner_stopping(&mut self, g: SessionGeneration) {
        if self.owner == Some(g) && self.check_open().is_ok() {
            self.phase = SessionPhase::Stopping;
        }
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
    pub fn desired(&self) -> Option<&SessionSpec> {
        self.desired.as_ref()
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
