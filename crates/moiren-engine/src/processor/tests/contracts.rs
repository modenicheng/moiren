use super::*;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Recording {
    timeline_epoch: u64,
    end_frame: u64,
    peak: f64,
    squares: f64,
    samples: u64,
}
impl Recording {
    fn rms(&self) -> f64 {
        (self.squares / self.samples as f64).sqrt()
    }
}

/// Minimal single-input observer recording what it saw, so the wrapper
/// contract test does not depend on any specific measurement backend.
struct RecordingObserver {
    recording: Arc<Mutex<Recording>>,
}
impl RecordingObserver {
    fn new() -> (Self, Arc<Mutex<Recording>>) {
        let recording = Arc::new(Mutex::new(Recording::default()));
        (
            Self {
                recording: Arc::clone(&recording),
            },
            recording,
        )
    }
}
impl RtObserver<f64> for RecordingObserver {
    fn validate_inputs(&self, io: &crate::buffer::PreparedIo) -> Result<(), ProcessorError> {
        if io.input_count() == 1 && io.input_channels(0).is_some() {
            Ok(())
        } else {
            Err(ProcessorError::InvalidIo)
        }
    }
    fn observe(
        &mut self,
        ctx: &ProcessContext,
        inputs: crate::buffer::ReadPorts<'_, f64>,
        _signals: crate::signal::ProcessSignals<'_>,
    ) {
        let input = inputs.get(0).expect("validated observer input");
        let mut recording = self.recording.lock().unwrap();
        for frame in 0..ctx.frames {
            for channel in 0..input.channel_count() {
                let sample = input.channel(channel)[frame];
                recording.peak = recording.peak.max(sample.abs());
                recording.squares += sample * sample;
                recording.samples += 1;
            }
        }
        recording.timeline_epoch = ctx.timeline_epoch;
        recording.end_frame = ctx.timeline_start + ctx.frames as u64;
    }
}

#[test]
fn gain_and_sum_reject_invalid_port_contracts() {
    let arena = arena::<f64>(&[1, 1, 2]);
    let a = arena.slot(0).unwrap();
    let b = arena.slot(1).unwrap();
    let c = arena.slot(2).unwrap();
    let input = PortAccess::Read { port: 0, slot: a };
    let output = PortAccess::Write { port: 0, slot: b };
    for ports in [
        vec![],
        vec![input],
        vec![input, PortAccess::Write { port: 1, slot: b }],
        vec![input, output, PortAccess::Write { port: 1, slot: c }],
        vec![input, PortAccess::Write { port: 0, slot: c }],
    ] {
        let io = arena.prepare_io(&ports).unwrap();
        assert_eq!(
            RtProcessor::<f64>::validate_io(&Gain, &io),
            Err(ProcessorError::InvalidIo)
        );
        assert_eq!(
            RtProcessor::<f64>::validate_io(&Bus, &io),
            Err(ProcessorError::InvalidIo)
        );
    }
    for ports in [
        vec![output],
        vec![PortAccess::Read { port: 1, slot: a }, output],
        vec![input, PortAccess::Read { port: 1, slot: a }, output],
        vec![PortAccess::InPlace {
            input: 1,
            output: 0,
            slot: a,
        }],
    ] {
        let io = arena.prepare_io(&ports).unwrap();
        assert_eq!(
            RtProcessor::<f64>::validate_io(&Gain, &io),
            Err(ProcessorError::InvalidIo)
        );
    }
    let io = arena
        .prepare_io(&[PortAccess::InPlace {
            input: 0,
            output: 0,
            slot: a,
        }])
        .unwrap();
    assert_eq!(
        RtProcessor::<f64>::validate_io(&Bus, &io),
        Err(ProcessorError::InvalidIo)
    );
}

#[test]
fn observer_requires_read_only_io_and_delegates_input_validation_and_audio() {
    let mut arena = arena::<f64>(&[1, 1]);
    seed(&mut arena, 0, &[&[1.0, -2.0, 3.0, -4.0]]);
    let a = arena.slot(0).unwrap();
    let b = arena.slot(1).unwrap();
    let input = PortAccess::Read { port: 0, slot: a };
    let output = PortAccess::Write { port: 0, slot: b };
    let (observer_probe, recording) = RecordingObserver::new();
    let mut observer = Observer(observer_probe);
    assert_eq!(RtProcessor::<f64>::role(&observer), ProcessorRole::Observer);
    for ports in [
        vec![],
        vec![PortAccess::Read { port: 1, slot: a }],
        vec![input, PortAccess::Read { port: 1, slot: a }],
        vec![output],
        vec![input, output],
        vec![PortAccess::InPlace {
            input: 0,
            output: 0,
            slot: a,
        }],
    ] {
        let io = arena.prepare_io(&ports).unwrap();
        assert_eq!(
            RtProcessor::<f64>::validate_io(&observer, &io),
            Err(ProcessorError::InvalidIo)
        );
    }
    let io = arena.prepare_io(&[input]).unwrap();
    RtProcessor::<f64>::validate_io(&observer, &io).unwrap();
    let (_, parameters) = parameter_channel(vec![], 1, 1, 1, 16).unwrap();
    let bindings = parameters.bindings(METER);
    arena
        .with_io(&io, 0..4, |io| {
            observer.process(
                &CONTEXT,
                io,
                parameters.view(&bindings),
                crate::signal::ProcessSignals::empty(&mut ()),
            )
        })
        .unwrap();
    let recording = recording.lock().unwrap();
    assert_eq!(recording.timeline_epoch, CONTEXT.timeline_epoch);
    assert_eq!(
        recording.end_frame,
        CONTEXT.timeline_start + CONTEXT.frames as u64
    );
    assert_eq!(recording.peak, 4.0);
    assert_eq!(recording.samples, 4);
    assert_eq!(recording.rms(), 7.5_f64.sqrt());
    drop(recording);
    assert_samples_exact(&mut arena, 0, &[&[1.0, -2.0, 3.0, -4.0]]);
}
