use super::backend::ToneSource;
use super::evidence::verify_spectrum;
use crate::service::{BuiltinState, GraphEdit, InputSelection, ServiceGraph};
use moiren_core::graph::NodeKind;
use moiren_engine::spectrum::{SpectrumSettings, SpectrumWindow};
use moiren_engine::{
    boundary::RtAudioSource,
    buffer::{BufferArena, BufferSlotLayout, PortAccess, ProcessIo},
    processor::ProcessContext,
};

fn sample() -> (SpectrumWindow, SpectrumSettings, Vec<f32>) {
    let settings = SpectrumSettings {
        fft_size: 1024,
        hop_frames: 512,
    };
    let window = SpectrumWindow {
        timeline_epoch: 1,
        frame_start: 4096,
        frame_end: 5120,
        sample_rate: 48_000.,
        fft_size: settings.fft_size,
        channels: 2,
    };
    // Validator fixtures only; native captures consume NodeSpectra observations.
    let mut magnitudes = vec![0.; 2 * settings.bins_per_channel()];
    magnitudes[20] = 0.1;
    magnitudes[settings.bins_per_channel() + 20] = 0.1;
    (window, settings, magnitudes)
}

#[::core::prelude::v1::test]
fn spectrum_capture_accepts_both_channel_major_peaks_at_bin_resolution() {
    let (window, settings, values) = sample();
    let evidence = verify_spectrum(window, &values, settings, 2, 937.5).unwrap();
    assert_eq!(evidence.peaks.len(), 2);
    assert_eq!(evidence.bin_hz, 46.875);
    for peak in evidence.peaks {
        assert_eq!(peak.bin, 20);
        assert_eq!(peak.frequency_hz, 937.5);
    }
    assert!(verify_spectrum(window, &values, settings, 2, 950.).is_ok());
}

#[::core::prelude::v1::test]
fn spectrum_capture_rejects_dc_and_wrong_second_channel() {
    let (window, settings, mut values) = sample();
    let second = settings.bins_per_channel();
    values[second + 20] = 0.;
    values[second] = 1.;
    assert!(verify_spectrum(window, &values, settings, 2, 937.5).is_err());
    values[second] = 0.;
    values[second + 24] = 0.1;
    assert!(verify_spectrum(window, &values, settings, 2, 937.5).is_err());
}

#[::core::prelude::v1::test]
fn spectrum_capture_rejects_silent_nonfinite_and_negative_magnitudes() {
    let (window, settings, values) = sample();
    assert!(verify_spectrum(window, &vec![0.; values.len()], settings, 2, 937.5).is_err());
    for invalid in [f32::NAN, f32::INFINITY, -0.1] {
        let mut bad = values.clone();
        bad[1] = invalid;
        assert!(verify_spectrum(window, &bad, settings, 2, 937.5).is_err());
    }
}

#[::core::prelude::v1::test]
fn spectrum_capture_rejects_wrong_shape_rate_and_incomplete_window() {
    let (window, settings, values) = sample();
    assert!(verify_spectrum(window, &values[1..], settings, 2, 937.5).is_err());
    assert!(verify_spectrum(window, &values, settings, 1, 937.5).is_err());
    for bad in [
        SpectrumWindow {
            sample_rate: 0.,
            ..window
        },
        SpectrumWindow {
            sample_rate: f64::NAN,
            ..window
        },
        SpectrumWindow {
            frame_end: window.frame_end - 1,
            ..window
        },
        SpectrumWindow {
            frame_start: window.frame_end + 1,
            ..window
        },
        SpectrumWindow {
            fft_size: 2048,
            ..window
        },
    ] {
        assert!(verify_spectrum(bad, &values, settings, 2, 937.5).is_err());
    }
}

#[::core::prelude::v1::test]
fn spectrum_capture_uses_active_settings_and_rejects_invalid_hop() {
    let (window, settings, values) = sample();
    assert!(
        verify_spectrum(
            window,
            &values,
            SpectrumSettings {
                fft_size: 2048,
                ..settings
            },
            2,
            937.5
        )
        .is_err()
    );
    assert!(
        verify_spectrum(
            window,
            &values,
            SpectrumSettings {
                hop_frames: 0,
                ..settings
            },
            2,
            937.5
        )
        .is_err()
    );
    // The header has no hop: the caller must separately fence binding identity.
    assert!(
        verify_spectrum(
            window,
            &values,
            SpectrumSettings {
                hop_frames: 256,
                ..settings
            },
            2,
            937.5
        )
        .is_ok()
    );
}

#[::core::prelude::v1::test]
fn spectrum_capture_tone_rejects_hardware_and_invalid_domains() {
    for name in [
        "capture-input",
        "moiren:tone:NaN",
        "moiren:tone:0",
        "moiren:tone:24000",
    ] {
        assert!(ToneSource::new(name, 2, 48_000.).is_err());
    }
    assert!(ToneSource::new("moiren:tone:937.5", 0, 48_000.).is_err());
    assert!(ToneSource::new("moiren:tone:937.5", 2, f64::NAN).is_err());
}

#[::core::prelude::v1::test]
fn spectrum_capture_tone_is_real_continuous_stereo_audio_across_callbacks() {
    let mut source = ToneSource::new("moiren:tone:937.5", 2, 48_000.).unwrap();
    let mut arena = BufferArena::<f32>::new(
        &[BufferSlotLayout {
            channels: 2,
            capacity_frames: 64,
        }],
        512,
    )
    .unwrap();
    let io = arena
        .prepare_io(&[PortAccess::Write {
            port: 0,
            slot: arena.slot(0).unwrap(),
        }])
        .unwrap();
    for block in 0..2 {
        let context = ProcessContext {
            timeline_epoch: 1,
            timeline_start: block * 64,
            frames: 64,
            processing_sr: 48_000.,
        };
        arena
            .with_io(&io, 0..64, |io| {
                let ProcessIo::Separate { mut outputs, .. } = io else {
                    panic!("source writes require separate IO")
                };
                assert_eq!(
                    source
                        .read(&context, outputs.get_mut(0).unwrap())
                        .transferred_frames,
                    64
                );
                let output = outputs.get_mut(0).unwrap();
                assert_eq!(output.channel(0), output.channel(1));
                for (frame, &value) in output.channel(0).iter().enumerate() {
                    let expected = (((block * 64 + frame as u64) as f64 * 937.5 / 48_000.
                        * std::f64::consts::TAU)
                        .sin()
                        * 0.1) as f32;
                    assert!((value - expected).abs() < 1e-6);
                }
            })
            .unwrap();
    }
}

#[::core::prelude::v1::test]
fn spectrum_capture_fixture_has_only_real_tone_analyzer_and_sink() {
    let graph = super::fixture().unwrap();
    assert_eq!(
        graph
            .graph
            .nodes()
            .iter()
            .map(|node| node.kind())
            .collect::<Vec<_>>(),
        [NodeKind::Source, NodeKind::SpectrumAnalyzer, NodeKind::Sink]
    );
    assert_eq!(graph.graph.edges().len(), 2);
    assert_eq!(
        graph.node_of_kind(NodeKind::Source).unwrap().1.source,
        Some(InputSelection::Tone {
            frequency_hz: super::TONE_HZ
        })
    );
    assert!(graph.graph.nodes().iter().all(|node| node.channels() == 2));
}

#[::core::prelude::v1::test]
fn spectrum_capture_graph_guard_allows_only_intended_analyzer_settings() {
    let before = super::fixture().unwrap();
    let node = before
        .node_of_kind(NodeKind::SpectrumAnalyzer)
        .unwrap()
        .0
        .id();
    let next = SpectrumSettings {
        fft_size: 2048,
        hop_frames: 512,
    };
    let mut graph = ServiceGraph::default();
    graph
        .replace(before.graph.clone(), before.nodes.clone())
        .unwrap();
    graph
        .edit(GraphEdit::SetBuiltin {
            node,
            state: BuiltinState::SpectrumAnalyzer(next),
        })
        .unwrap();
    assert!(
        super::native::verify_graph_content(&before, &graph.snapshot(), node, Some(next)).is_ok()
    );
    assert!(super::native::verify_graph_content(&before, &graph.snapshot(), node, None).is_err());
    graph
        .edit(GraphEdit::SetLabel {
            node,
            label: "unintended rename".into(),
        })
        .unwrap();
    assert!(
        super::native::verify_graph_content(&before, &graph.snapshot(), node, Some(next)).is_err()
    );
}

#[::core::prelude::v1::test]
fn spectrum_capture_channel_readout_requires_truthful_selected_and_total_counts() {
    assert_eq!(
        super::native::parse_channel_readout("Ch 1 / 2").unwrap(),
        (0, 2)
    );
    assert_eq!(
        super::native::parse_channel_readout("Ch 2 / 2").unwrap(),
        (1, 2)
    );
    for text in [
        "",
        "Ch 0 / 2",
        "Ch 3 / 2",
        "Ch 1 / 0",
        "Ch 1 / 257",
        "Channel 2",
        "Ch NaN / 2",
    ] {
        assert!(
            super::native::parse_channel_readout(text).is_err(),
            "{text}"
        );
    }
}

#[::core::prelude::v1::test]
fn spectrum_capture_inspector_geometry_includes_both_exact_inputs() {
    let graph = super::fixture().unwrap();
    let node = graph
        .node_of_kind(NodeKind::SpectrumAnalyzer)
        .unwrap()
        .0
        .id();
    let settings = SpectrumSettings::default();
    let card = super::native::control_ids(node, "card", settings);
    let inspector = super::native::control_ids(node, "inspector", settings);
    assert_eq!(card.len(), 14);
    assert_eq!(inspector.len(), 16);
    for field in ["Fft", "Hop"] {
        let id = format!("spectrum-exact-{node:?}-{field}");
        assert!(!card.contains(&id));
        assert_eq!(inspector.iter().filter(|target| **target == id).count(), 1);
    }
    assert_eq!(
        inspector.last(),
        Some(&format!("spectrum-exact-{node:?}-Hop")),
        "Inspector reveal must reach its final precision field"
    );
}
