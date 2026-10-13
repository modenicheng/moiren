use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum FadeError {
    #[error("fade requires 1..=48000 output frames")]
    InvalidFrames,
    #[error("fade was already requested")]
    AlreadyRequested,
}
#[derive(Default)]
struct SharedFade {
    fade_in: AtomicU32,
    fade_out: AtomicU32,
    submitted: AtomicBool,
}
#[derive(Clone)]
pub struct RenderFadeControl(Arc<SharedFade>);
impl RenderFadeControl {
    pub fn request_fade_in(&self, frames: u32) -> Result<(), FadeError> {
        Self::request(&self.0.fade_in, frames)
    }
    pub fn request_fade_out(&self, frames: u32) -> Result<(), FadeError> {
        Self::request(&self.0.fade_out, frames)
    }
    fn request(target: &AtomicU32, frames: u32) -> Result<(), FadeError> {
        if !(1..=48_000).contains(&frames) {
            return Err(FadeError::InvalidFrames);
        }
        target
            .compare_exchange(0, frames, Ordering::Release, Ordering::Relaxed)
            .map(|_| ())
            .map_err(|_| FadeError::AlreadyRequested)
    }
    pub fn fade_out_submitted(&self) -> bool {
        self.0.submitted.load(Ordering::Acquire)
    }
}
pub(super) struct FadeEnvelope {
    shared: RenderFadeControl,
    fade_in_total: u32,
    fade_in_remaining: u32,
    fade_out_total: u32,
    fade_out_remaining: u32,
    gain: f32,
    fade_out_start: f32,
}
impl FadeEnvelope {
    pub fn new() -> Self {
        Self {
            shared: RenderFadeControl(Arc::new(SharedFade::default())),
            fade_in_total: 0,
            fade_in_remaining: 0,
            fade_out_total: 0,
            fade_out_remaining: 0,
            gain: 1.0,
            fade_out_start: 1.0,
        }
    }
    pub fn control(&self) -> RenderFadeControl {
        self.shared.clone()
    }
    pub fn apply(&mut self, samples: &mut [f32]) {
        if samples.is_empty() {
            return;
        }
        if self.fade_in_total == 0 {
            self.fade_in_total = self.shared.0.fade_in.load(Ordering::Acquire);
            self.fade_in_remaining = self.fade_in_total;
        }
        if self.fade_out_total == 0 {
            self.fade_out_total = self.shared.0.fade_out.load(Ordering::Acquire);
            self.fade_out_remaining = self.fade_out_total;
            self.fade_out_start = self.gain;
        }
        if self.fade_in_remaining == 0 && self.fade_out_total == 0 {
            return;
        }
        for frame in samples.as_chunks_mut::<{ super::CHANNELS }>().0 {
            if self.fade_out_total != 0 {
                self.fade_out_remaining = self.fade_out_remaining.saturating_sub(1);
                self.gain = self.fade_out_start * self.fade_out_remaining as f32
                    / self.fade_out_total as f32;
            } else if self.fade_in_remaining != 0 {
                self.fade_in_remaining -= 1;
                self.gain = (self.fade_in_total - self.fade_in_remaining) as f32
                    / self.fade_in_total as f32;
            }
            for sample in frame {
                *sample *= self.gain;
            }
        }
    }
    pub fn acknowledge_submission(&self) {
        if self.fade_out_total != 0 && self.fade_out_remaining == 0 {
            self.shared.0.submitted.store(true, Ordering::Release);
        }
    }
}
