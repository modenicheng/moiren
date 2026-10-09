//! Independent owned tone process; the parent never changes existing sessions.
use anyhow::{Result, bail};
use moiren_app::host::{AudioHost, HostConfig, SourceSettings};
use moiren_engine::{
    boundary::{BoundaryReport, RtAudioSource},
    buffer::AudioBlockMut,
    processor::ProcessContext,
};
use moiren_windows_audio::render::{DemandRenderer, RenderOptions, RenderStatus, start_render};
use std::{
    os::windows::process::CommandExt,
    process::{Child, Command, Stdio},
};

struct Tone(f64);
impl RtAudioSource<f32> for Tone {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(&mut self, ctx: &ProcessContext, mut output: AudioBlockMut<'_, f32>) -> BoundaryReport {
        for frame in 0..ctx.frames {
            let value = (self.0.sin() * 0.02) as f32;
            output.channel_mut(0)[frame] = value;
            output.channel_mut(1)[frame] = value;
            self.0 = (self.0 + std::f64::consts::TAU * 440.0 / ctx.processing_sr)
                % std::f64::consts::TAU;
        }
        BoundaryReport {
            transferred_frames: ctx.frames,
            ..BoundaryReport::default()
        }
    }
}

pub(super) fn tone(endpoint: &str) -> Result<()> {
    let (mut host, renderer) = AudioHost::prepare(HostConfig::default())?;
    host.add_source_with_settings(
        Tone(0.0),
        SourceSettings {
            gain: 1.0,
            pan: 0.0,
            available: true,
        },
    )?;
    host.publish()?;
    let (engine, output) = renderer.into_parts();
    let native = DemandRenderer::new(engine, output)?;
    let (report, native) =
        start_render(RenderOptions::continuous(endpoint), native)?.join_with_renderer()?;
    let (engine, output) = native.into_parts();
    host.finish_parts(engine, output);
    if report.status == RenderStatus::Failed {
        bail!("owned generator render failed");
    }
    Ok(())
}

pub(super) struct OwnedChild(Child);
impl OwnedChild {
    pub(super) fn spawn(endpoint: &str) -> Result<Self> {
        Ok(Self(
            Command::new(std::env::current_exe()?)
                .args(["--owned-tone", endpoint])
                .creation_flags(0x08000000)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?,
        ))
    }
    pub(super) fn pid(&self) -> u32 {
        self.0.id()
    }
    pub(super) fn stop(&mut self) -> Result<()> {
        if self.0.try_wait()?.is_none() {
            self.0.kill()?;
        }
        self.0.wait()?;
        Ok(())
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
