use super::*;

#[test]
fn delay_preserves_state_across_blocks() {
    let mut arena = arena::<f64>(&[1, 1]);
    seed(&mut arena, 0, &[&[1.0, 2.0, 3.0, 4.0]]);
    let io = arena
        .prepare_io(&[
            PortAccess::Read {
                port: 0,
                slot: arena.slot(0).unwrap(),
            },
            PortAccess::Write {
                port: 0,
                slot: arena.slot(1).unwrap(),
            },
        ])
        .unwrap();
    let settings = DelaySettings {
        delay_ms: 1.0,
        max_delay_ms: 10.0,
        mix: 1.0,
        ..DelaySettings::default()
    };
    let (_, parameters) =
        parameter_channel(Delay::parameters(PROCESSOR, settings).to_vec(), 1, 1, 1, 16).unwrap();
    let bindings = parameters.bindings(PROCESSOR);
    let mut delay = Delay::new(1, settings, 1_000.0).unwrap();
    for (start, frames) in [(0, 2), (2, 2)] {
        arena
            .with_io(&io, start..start + frames, |io| {
                delay.process(
                    &ProcessContext {
                        timeline_start: start as u64,
                        frames,
                        processing_sr: 1_000.0,
                        ..CONTEXT
                    },
                    io,
                    parameters.view(&bindings),
                    crate::signal::ProcessSignals::empty(&mut ()),
                )
            })
            .unwrap();
    }
    assert_samples(&mut arena, 1, &[&[0.0, 1.0, 2.0, 3.0]]);
}
