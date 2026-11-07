use super::*;
use crate::{
    owner::LeaseClient,
    render_evidence::{RenderEvidenceConfig, RenderEvidenceFormat, render_evidence_stream},
    test_allocation::assert_allocates_nothing,
};
use std::cell::{Cell, RefCell};
use windows::core::HRESULT;

fn evidence() -> (
    crate::render_evidence::RenderEvidenceWriter,
    crate::render_evidence::RenderEvidenceReader,
) {
    render_evidence_stream(
        RenderEvidenceConfig {
            capacity_frames: 2,
            max_packet_frames: 2,
        },
        RenderEvidenceFormat {
            sample_rate: 48_000,
            sample_format: SampleFormat::Float32,
            channels: 2,
            container_bits: 32,
            valid_bits: 32,
        },
    )
    .unwrap()
}

#[test]
fn render_operation_evidence_preserves_native_results_and_optional_clocks() {
    use crate::render_evidence::RenderEventKind;
    for kind in [
        RenderEventKind::Start,
        RenderEventKind::Stop,
        RenderEventKind::Wait,
        RenderEventKind::Write,
    ] {
        for failed in [false, true] {
            let (mut writer, mut reader) = evidence();
            assert_allocates_nothing(|| {
                let mut event =
                    render_evidence::RenderOperationEvidence::begin(true, kind, || Some(200));
                event.sample.padding_frames = Some(17);
                event.sample.ready = (kind == RenderEventKind::Wait).then_some(false);
                event.finish(Some(&mut writer), failed, || Some(199));
            });
            let event = reader.pop_event().unwrap();
            assert_eq!(event.sample.kind, kind);
            assert_eq!(event.sample.failed, failed);
            assert_eq!(event.sample.qpc_before_100ns, Some(200));
            assert_eq!(event.sample.qpc_after_100ns, Some(199));
            assert_eq!(event.sample.padding_frames, Some(17));
            assert_eq!(event.packet_sequence, None);
            assert_eq!(writer.snapshot().timeline.regressed_clocks, 1);
        }
    }
    let (mut writer, mut reader) = evidence();
    let event =
        render_evidence::RenderOperationEvidence::begin(true, RenderEventKind::Write, || None);
    event.finish(Some(&mut writer), true, || None);
    assert_eq!(reader.pop_event().unwrap().sample.qpc_after_100ns, None);
    assert_eq!(writer.snapshot().timeline.unavailable_clocks, 1);
}

#[test]
fn disabled_render_operation_evidence_never_reads_clock_or_emits() {
    let (mut writer, mut reader) = evidence();
    assert_allocates_nothing(|| {
        let event = render_evidence::RenderOperationEvidence::begin(
            false,
            crate::render_evidence::RenderEventKind::Write,
            || panic!("disabled clock"),
        );
        event.finish(Some(&mut writer), false, || panic!("disabled clock"));
    });
    assert!(reader.pop_event().is_none());
    assert_eq!(writer.snapshot().timeline.events, 0);
}

#[test]
fn render_evidence_native_release_records_outcome_after_one_release() {
    for hr in [0, 1, -2_147_024_891] {
        let (mut writer, reader) = evidence();
        let reader = RefCell::new(reader);
        struct Client<'a> {
            reader: &'a RefCell<crate::render_evidence::RenderEvidenceReader>,
            calls: Cell<usize>,
            hr: HRESULT,
        }
        impl LeaseClient for Client<'_> {
            fn release_packet(&self, _: u32, _: u32) -> windows::core::Result<()> {
                unreachable!()
            }
            fn release_packet_hresult(&self, frames: u32, flags: u32) -> HRESULT {
                assert_eq!(frames, 2);
                assert_eq!(flags, 0);
                assert!(
                    self.reader.borrow_mut().pop().is_none(),
                    "no outcome before release"
                );
                self.calls.set(self.calls.get() + 1);
                self.hr
            }
        }
        let client = Client {
            reader: &reader,
            calls: Cell::new(0),
            hr: HRESULT(hr),
        };
        let bytes: [u8; 16] = std::array::from_fn(|i| (i + 1) as u8);
        let result = assert_allocates_nothing(|| {
            let staged = stage_render_evidence(&bytes, 2, Some(&mut writer));
            release_render_evidence(
                PacketLease::new(&client, 2, LeaseFallback::RenderSilent),
                staged,
                Some(&mut writer),
            )
        });
        assert_eq!(result.0.0, hr);
        assert_eq!(result.1, Some(1));
        assert_eq!(client.calls.get(), 1);
        for offset in 0..2 {
            let frame = reader.borrow_mut().pop().unwrap();
            assert_eq!(&frame.bytes[..8], &bytes[offset * 8..(offset + 1) * 8]);
            assert_eq!(frame.release_hresult, hr);
        }
        assert_eq!(
            writer.snapshot().accepted_frames,
            if hr >= 0 { 2 } else { 0 }
        );
        assert_eq!(
            writer.snapshot().rejected_frames,
            if hr < 0 { 2 } else { 0 }
        );
    }
}

#[test]
fn render_evidence_stage_refusal_does_not_gate_native_submission() {
    struct Client(Cell<usize>);
    impl LeaseClient for Client {
        fn release_packet(&self, _: u32, _: u32) -> windows::core::Result<()> {
            unreachable!()
        }
        fn release_packet_hresult(&self, _: u32, _: u32) -> HRESULT {
            self.0.set(self.0.get() + 1);
            HRESULT(0)
        }
    }
    let (mut writer, mut reader) = evidence();
    let client = Client(Cell::new(0));
    writer.stage_packet(&[1; 8], 1).unwrap();
    writer.finish_packet(0).unwrap();
    reader.pop().unwrap();
    let hr = assert_allocates_nothing(|| {
        let staged = stage_render_evidence(&[9; 24], 3, Some(&mut writer));
        release_render_evidence(
            PacketLease::new(&client, 3, LeaseFallback::RenderSilent),
            staged,
            Some(&mut writer),
        )
    });
    assert_eq!(hr.0.0, 0);
    assert_eq!(hr.1, None);
    assert_eq!(client.0.get(), 1);
    assert!(reader.pop().is_none());
    let summary = writer.snapshot();
    assert_eq!(summary.attempts, 1);
    assert_eq!(summary.invalid_attempts, 1);
    assert_eq!(summary.unobserved_packets, 1);
    assert_eq!(summary.unobserved_frames, 3);
}

#[test]
fn render_release_timeline_retains_packet_hresult_and_frontier_without_native_bytes() {
    struct Client(HRESULT);
    impl LeaseClient for Client {
        fn release_packet(&self, _: u32, _: u32) -> windows::core::Result<()> {
            unreachable!()
        }
        fn release_packet_hresult(&self, _: u32, _: u32) -> HRESULT {
            self.0
        }
    }
    for hr in [0, -42] {
        let (mut writer, mut reader) = evidence();
        let client = Client(HRESULT(hr));
        assert_allocates_nothing(|| {
            let mut event = RenderOperationEvidence::begin(
                true,
                crate::render_evidence::RenderEventKind::Write,
                || Some(100),
            );
            event.sample.requested_frames = 2;
            event.sample.padding_frames = Some(0);
            let staged = stage_render_evidence(&[7; 16], 2, Some(&mut writer));
            let (actual_hr, sequence) = release_render_evidence(
                PacketLease::new(&client, 2, LeaseFallback::RenderSilent),
                staged,
                Some(&mut writer),
            );
            event.packet_sequence = sequence;
            event.sample.submitted_frames = 2;
            event.sample.release_hresult = Some(actual_hr.0);
            event.finish(Some(&mut writer), actual_hr.is_err(), || Some(101));
        });
        let event = reader.pop_event().unwrap();
        assert_eq!(event.packet_sequence, Some(1));
        assert_eq!(event.sample.release_hresult, Some(hr));
        assert_eq!(event.accepted_frames, if hr >= 0 { 2 } else { 0 });
        assert_eq!(event.sample.failed, hr < 0);
    }
}

#[test]
fn render_evidence_native_buffer_can_be_invalidated_during_release() {
    struct Client {
        native: RefCell<Option<Box<[u8; 16]>>>,
        calls: Cell<usize>,
    }
    impl LeaseClient for Client {
        fn release_packet(&self, _: u32, _: u32) -> windows::core::Result<()> {
            unreachable!()
        }
        fn release_packet_hresult(&self, _: u32, _: u32) -> HRESULT {
            let mut memory = self.native.borrow_mut().take().unwrap();
            memory.fill(0xee);
            drop(memory);
            self.calls.set(self.calls.get() + 1);
            HRESULT(0)
        }
    }
    let (mut writer, mut reader) = evidence();
    let expected: [u8; 16] = std::array::from_fn(|i| (i + 1) as u8);
    let client = Client {
        native: RefCell::new(Some(Box::new(expected))),
        calls: Cell::new(0),
    };
    let staged = {
        let native = client.native.borrow();
        stage_render_evidence(&**native.as_ref().unwrap(), 2, Some(&mut writer))
    };
    let result = release_render_evidence(
        PacketLease::new(&client, 2, LeaseFallback::RenderSilent),
        staged,
        Some(&mut writer),
    );
    assert_eq!(result.0.0, 0);
    assert_eq!(result.1, Some(1));
    assert_eq!(client.calls.get(), 1);
    assert!(client.native.borrow().is_none());
    for offset in 0..2 {
        assert_eq!(
            &reader.pop().unwrap().bytes[..8],
            &expected[offset * 8..(offset + 1) * 8]
        );
    }
}

#[test]
fn render_evidence_attachment_requires_exact_layout_prestart_and_whole_native_packet() {
    let (mut writer, _reader) = evidence();
    let codec = SampleCodec::with_layout(SampleFormat::Float32, 2, 32, 32).unwrap();
    let validate = |writer: &crate::render_evidence::RenderEvidenceWriter,
                    started,
                    render,
                    rate,
                    cap,
                    attached| {
        validate_render_evidence_attachment(writer, codec, rate, cap, started, render, attached)
    };
    assert!(validate(&writer, false, true, 48_000, 2, false).is_ok());
    for (started, render, rate, cap, attached) in [
        (true, true, 48_000, 2, false),
        (false, false, 48_000, 2, false),
        (false, true, 44_100, 2, false),
        (false, true, 48_000, 3, false),
        (false, true, 48_000, 2, true),
    ] {
        assert!(validate(&writer, started, render, rate, cap, attached).is_err());
    }
    let different = SampleCodec::with_layout(SampleFormat::Pcm24, 2, 32, 24).unwrap();
    assert!(
        validate_render_evidence_attachment(&writer, different, 48_000, 2, false, true, false)
            .is_err()
    );
    writer.stage_packet(&[7; 8], 1).unwrap();
    assert!(validate(&writer, false, true, 48_000, 2, false).is_err());
    writer.close();
    assert!(validate(&writer, false, true, 48_000, 2, false).is_err());
}

#[test]
fn render_evidence_native_layout_does_not_collapse_container_or_valid_bits() {
    for (sample_format, container_bits, valid_bits) in [
        (SampleFormat::Pcm16, 16, 16),
        (SampleFormat::Pcm24, 24, 24),
        (SampleFormat::Pcm24, 32, 24),
        (SampleFormat::Pcm32, 32, 20),
        (SampleFormat::Float32, 32, 32),
        (SampleFormat::Float64, 64, 64),
    ] {
        for channels in [1, 2] {
            let codec =
                SampleCodec::with_layout(sample_format, channels, container_bits, valid_bits)
                    .unwrap();
            let format = render_evidence::negotiated_format(codec, 44_100);
            assert_eq!(
                format,
                RenderEvidenceFormat {
                    sample_rate: 44_100,
                    sample_format,
                    channels,
                    container_bits,
                    valid_bits,
                }
            );
            let (writer, _reader) = render_evidence_stream(
                RenderEvidenceConfig {
                    capacity_frames: 2,
                    max_packet_frames: 2,
                },
                format,
            )
            .unwrap();
            validate_render_evidence_attachment(&writer, codec, 44_100, 2, false, true, false)
                .unwrap();
        }
    }
}

#[test]
#[ignore = "requires an explicitly pinned isolated remote Windows render endpoint"]
fn render_evidence_remote_native_smoke() {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "LAPTOP-NP49RG6H");
    let endpoint =
        std::env::var("MOIREN_RENDER_EVIDENCE_ENDPOINT").expect("explicit endpoint required");
    assert!(!endpoint.trim().is_empty());
    let mut stream = WasapiStream::open(WasapiOptions {
        endpoint_id: endpoint,
        sample_rate: 48_000,
        device: DeviceOptions {
            sample_format: SampleFormat::Float32,
            period_frames: Some(336),
            ..DeviceOptions::default()
        },
        input: false,
        exclusive_period_evidence: None,
    })
    .unwrap();
    let info = stream.info();
    let format = stream.render_evidence_format().unwrap();
    let (writer, mut reader) = render_evidence_stream(
        RenderEvidenceConfig {
            capacity_frames: 4096,
            max_packet_frames: info.buffer_frames as usize,
        },
        format,
    )
    .unwrap();
    stream.attach_render_evidence(writer).unwrap();
    let total = 96_000;
    let input: Vec<StereoFrame> = (0..total)
        .map(|i| {
            let left = ((i * 17 % 997) as f64 / 996.0 - 0.5) * 0.01;
            let right = ((i * 29 % 991) as f64 / 990.0 - 0.5) * 0.01;
            [left, right]
        })
        .collect();
    let codec = SampleCodec::with_layout(
        format.sample_format,
        format.channels,
        format.container_bits,
        format.valid_bits,
    )
    .unwrap();
    let frame_bytes = codec.frame_bytes();
    let mut expected = vec![0; total * frame_bytes];
    codec.encode(&input, &mut expected).unwrap();
    // Allocate before Start. Always stop/take/release before asserting the run
    // outcome so a failed native call still retires diagnostics off streaming.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut written = 0;
    let mut observed = 0;
    let mut observed_events = 0;
    let mut last_kind = None;
    let run = (|| -> Result<(), BackendError> {
        stream.start()?;
        while written < total {
            if std::time::Instant::now() > deadline {
                return Err(unsupported("native smoke deadline"));
            }
            written += stream.write(&input[written..])?;
            while let Some(frame) = reader.pop() {
                if frame.release_hresult < 0
                    || frame.bytes[..frame_bytes]
                        != expected[observed * frame_bytes..(observed + 1) * frame_bytes]
                {
                    return Err(unsupported("native encoded evidence mismatch"));
                }
                observed += 1;
            }
            while let Some(event) = reader.pop_event() {
                observed_events += 1;
                if event.sequence != observed_events {
                    return Err(unsupported("native render timeline sequence mismatch"));
                }
                last_kind = Some(event.sample.kind);
            }
            if written < total {
                stream.wait(Duration::from_millis(10))?;
            }
        }
        Ok(())
    })();
    let stopped = stream.stop();
    let writer = stream.take_render_evidence().unwrap();
    let released = stream.release();
    let stats = stream.stats();
    drop(stream);
    while let Some(event) = reader.pop_event() {
        observed_events += 1;
        assert_eq!(event.sequence, observed_events);
        last_kind = Some(event.sample.kind);
    }
    let summary = writer.snapshot();
    eprintln!(
        "render_evidence_native_receipt={}",
        serde_json::json!({
            "written_frames": written, "observed_frames": observed, "summary": summary,
            "native_stats": {"write_calls": stats.write_calls, "written_frames": stats.written_frames,
                "wait_timeouts": stats.wait_timeouts},
            "run_ok": run.is_ok(), "stop_ok": stopped.is_ok(), "release_ok": released.is_ok(),
            "physical_latency": null, "quality_qualified": false,
        })
    );
    run.unwrap();
    stopped.unwrap();
    released.unwrap();
    assert_eq!(written, total);
    assert_eq!(observed, total);
    assert_eq!(summary.accepted_frames, total as u64);
    assert_eq!(summary.stored_frames, total as u64);
    assert_eq!(summary.rejected_frames, 0);
    assert_eq!(summary.dropped_frames, 0);
    assert_eq!(summary.invalid_attempts, 0);
    assert_eq!(summary.unobserved_frames, 0);
    assert_eq!(summary.pending_abandoned_frames, 0);
    assert!(!summary.counters_saturated);
    assert!(reader.is_finished());
    assert!(writer.is_closed());
    assert!(reader.events_finished());
    assert_eq!(
        last_kind,
        Some(crate::render_evidence::RenderEventKind::Stop)
    );
    assert_eq!(observed_events, summary.timeline.events);
    assert_eq!(summary.timeline.dropped_events, 0);
    assert_eq!(summary.timeline.reader_abandoned_events, 0);
}
