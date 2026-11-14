//! Opt-in host-only archive for an independent reader, never native evidence.
use super::*;

#[test]
#[ignore = "writes a host-only archive to MOIREN_SRC_READBACK_FIXTURE; no native devices"]
fn src_stream_independent_readback_fixture() {
    let directory = PathBuf::from(
        std::env::var_os("MOIREN_SRC_READBACK_FIXTURE").expect("explicit fixture output directory"),
    );
    // Refuse reuse: even an earlier failed fixture remains evidence.
    std::fs::create_dir(&directory).unwrap();
    let metadata = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("fixture-origin.json"))
        .unwrap();
    serde_json::to_writer(
        metadata,
        &serde_json::json!({
            "schema_version":1, "native_execution":false, "qualified":false,
            "scope":"real backend/EngineHost/IO-Bus callbacks; test provider and finite synthetic clock; no native devices, waveform or source-origin evidence",
            "expected_checkpoints":132000,
        }),
    )
    .unwrap();
    let mut fixture = fixture::Fixture::new();
    let owner = fixture.observations.retain_session().unwrap();
    let mut stream =
        NativeSrcStream::open(&owner, &directory.join("src-trace.chunks.jsonl")).unwrap();
    for _ in 0..660 {
        fixture.callbacks(100);
        stream.poll();
    }
    fixture.close();
    let mut report = None;
    stream
        .finish(&directory.join("src-trace.json"), &mut report)
        .unwrap();
    let report = report.unwrap();
    assert_eq!(report["history_complete"], true);
    assert_eq!(report["qualified"], false);
    assert_eq!(report["drained_checkpoints"], 132000);
}
