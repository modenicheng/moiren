//! Independent fake IASIO metadata driver; these tests never activate COM.

use super::*;
use crate::professional::asio::{abi, callback};
use std::{
    ffi::c_void,
    ptr,
    sync::{Arc, Mutex},
    thread::ThreadId,
};

type Events = Arc<Mutex<Vec<&'static str>>>;

#[derive(Clone)]
struct Metadata {
    inputs: i32,
    outputs: i32,
    sizes: [i32; 4],
    rate: f64,
    latency: [i32; 2],
    failure: Option<(&'static str, i32)>,
    fail_init: bool,
    unterminated_names: bool,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            inputs: 2,
            outputs: 1,
            sizes: [64, 4096, 96, 32],
            rate: 48000.0,
            latency: [32, 64],
            failure: None,
            fail_init: false,
            unterminated_names: false,
        }
    }
}

#[repr(C)]
struct MetadataDriver {
    interface: abi::Driver,
    metadata: Metadata,
    events: Events,
    owner: ThreadId,
}

unsafe fn fake<'a>(raw: *mut abi::Driver) -> &'a MetadataDriver {
    // SAFETY: each fake is boxed with the IASIO interface as its first field.
    let fake = unsafe { &*raw.cast::<MetadataDriver>() };
    assert_eq!(fake.owner, std::thread::current().id());
    fake
}

fn result(driver: &MetadataDriver, operation: &'static str) -> i32 {
    driver.events.lock().unwrap().push(operation);
    match driver.metadata.failure {
        Some((name, code)) if name == operation => code,
        _ => abi::OK,
    }
}

unsafe extern "system" fn query(_: *mut abi::Driver, _: *const c_void, _: *mut *mut c_void) -> i32 {
    -1
}
unsafe extern "system" fn add_ref(_: *mut abi::Driver) -> u32 {
    1
}
unsafe extern "system" fn release(raw: *mut abi::Driver) -> u32 {
    let driver = unsafe { fake(raw) };
    assert!(
        callback::Exclusive::acquire().is_err(),
        "lease must outlive Release"
    );
    assert!(
        callback::is_detached(),
        "query must never publish a callback"
    );
    driver.events.lock().unwrap().push("release");
    unsafe { drop(Box::from_raw(raw.cast::<MetadataDriver>())) };
    0
}
unsafe extern "system" fn init(raw: *mut abi::Driver, _: *mut c_void) -> i32 {
    let driver = unsafe { fake(raw) };
    result(driver, "init");
    i32::from(!driver.metadata.fail_init)
}
unsafe extern "system" fn name(raw: *mut abi::Driver, out: *mut u8) {
    let driver = unsafe { fake(raw) };
    result(driver, "name");
    if driver.metadata.unterminated_names {
        unsafe { ptr::write_bytes(out, b'N', 32) };
    } else {
        unsafe { ptr::copy_nonoverlapping(c"Metadata IASIO".as_ptr().cast(), out, 15) };
    }
}
unsafe extern "system" fn version(raw: *mut abi::Driver) -> i32 {
    result(unsafe { fake(raw) }, "version");
    42
}
unsafe extern "system" fn error(_: *mut abi::Driver, out: *mut u8) {
    unsafe { ptr::copy_nonoverlapping(c"metadata query rejected".as_ptr().cast(), out, 24) };
}
unsafe extern "system" fn forbidden(raw: *mut abi::Driver) -> i32 {
    result(unsafe { fake(raw) }, "FORBIDDEN");
    -999
}
unsafe extern "system" fn channels(
    raw: *mut abi::Driver,
    input: *mut i32,
    output: *mut i32,
) -> i32 {
    let driver = unsafe { fake(raw) };
    unsafe {
        input.write(driver.metadata.inputs);
        output.write(driver.metadata.outputs);
    }
    result(driver, "getChannels")
}
unsafe extern "system" fn latencies(
    raw: *mut abi::Driver,
    input: *mut i32,
    output: *mut i32,
) -> i32 {
    let driver = unsafe { fake(raw) };
    unsafe {
        input.write(driver.metadata.latency[0]);
        output.write(driver.metadata.latency[1]);
    }
    result(driver, "getLatencies")
}
unsafe extern "system" fn sizes(
    raw: *mut abi::Driver,
    min: *mut i32,
    max: *mut i32,
    preferred: *mut i32,
    granularity: *mut i32,
) -> i32 {
    let driver = unsafe { fake(raw) };
    for (out, value) in [min, max, preferred, granularity]
        .into_iter()
        .zip(driver.metadata.sizes)
    {
        unsafe { out.write(value) };
    }
    result(driver, "getBufferSize")
}
unsafe extern "system" fn can_rate(raw: *mut abi::Driver, rate: f64) -> i32 {
    let code = result(unsafe { fake(raw) }, "canSampleRate");
    if code != abi::OK {
        code
    } else if rate == 44100.0 || rate == 48000.0 {
        abi::OK
    } else {
        -995
    }
}
unsafe extern "system" fn get_rate(raw: *mut abi::Driver, rate: *mut f64) -> i32 {
    let driver = unsafe { fake(raw) };
    unsafe { rate.write(driver.metadata.rate) };
    result(driver, "getSampleRate")
}
unsafe extern "system" fn set_rate(raw: *mut abi::Driver, _: f64) -> i32 {
    unsafe { forbidden(raw) }
}
unsafe extern "system" fn clocks(
    raw: *mut abi::Driver,
    _: *mut abi::ClockSource,
    _: *mut i32,
) -> i32 {
    unsafe { forbidden(raw) }
}
unsafe extern "system" fn clock(raw: *mut abi::Driver, _: i32) -> i32 {
    unsafe { forbidden(raw) }
}
unsafe extern "system" fn position(
    raw: *mut abi::Driver,
    _: *mut abi::Samples,
    _: *mut abi::Samples,
) -> i32 {
    unsafe { forbidden(raw) }
}
unsafe extern "system" fn channel(raw: *mut abi::Driver, out: *mut abi::ChannelInfo) -> i32 {
    let driver = unsafe { fake(raw) };
    let info = unsafe { &mut *out };
    let input = info.is_input != 0;
    info.sample_type = if input {
        if info.channel == 0 { 19 } else { 32 }
    } else {
        9876
    };
    info.is_active = i32::from(!input);
    info.channel_group = 7;
    if driver.metadata.unterminated_names {
        info.name = [b'C'; 32];
    } else {
        let name = if input { b"Input\0" } else { b"Main\0\0" };
        info.name[..name.len()].copy_from_slice(name);
    }
    result(driver, "getChannelInfo")
}
unsafe extern "system" fn create(
    raw: *mut abi::Driver,
    _: *mut abi::BufferInfo,
    _: i32,
    _: i32,
    _: *mut abi::Callbacks,
) -> i32 {
    unsafe { forbidden(raw) }
}
unsafe extern "system" fn future(raw: *mut abi::Driver, _: i32, _: *mut c_void) -> i32 {
    unsafe { forbidden(raw) }
}

static VTABLE: abi::VTable = abi::VTable {
    query_interface: query,
    add_ref,
    release,
    init,
    get_driver_name: name,
    get_driver_version: version,
    get_error_message: error,
    start: forbidden,
    stop: forbidden,
    get_channels: channels,
    get_latencies: latencies,
    get_buffer_size: sizes,
    can_sample_rate: can_rate,
    get_sample_rate: get_rate,
    set_sample_rate: set_rate,
    get_clock_sources: clocks,
    set_clock_source: clock,
    get_sample_position: position,
    get_channel_info: channel,
    create_buffers: create,
    dispose_buffers: forbidden,
    control_panel: forbidden,
    future,
    output_ready: forbidden,
};

fn snapshot(metadata: Metadata, events: &Events) -> Result<AsioCapabilities, BackendError> {
    query_with_driver(|| {
        let raw = Box::into_raw(Box::new(MetadataDriver {
            interface: abi::Driver { vtable: &VTABLE },
            metadata,
            events: Arc::clone(events),
            owner: std::thread::current().id(),
        }));
        Ok(QueryDriver {
            driver: unsafe { Driver::from_test_raw(raw.cast()) },
            _apartment: None,
            identity: AsioDriver {
                id: "{01234567-89AB-CDEF-0123-456789ABCDEF}".into(),
                name: "Registry description".into(),
            },
        })
    })
}

fn assert_released(events: &Events) {
    let events = events.lock().unwrap();
    assert_eq!(events.first(), Some(&"init"));
    assert_eq!(events.last(), Some(&"release"));
    assert_eq!(
        events.iter().filter(|&&event| event == "release").count(),
        1
    );
    assert!(
        !events.contains(&"FORBIDDEN"),
        "query called a mutating driver API"
    );
    drop(events);
    assert!(
        callback::Exclusive::acquire().is_ok(),
        "query leaked the lease"
    );
}

#[test]
fn snapshot_reports_native_metadata_without_starting_or_changing_driver() {
    let _serial = callback::TEST_SERIAL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let events = Events::default();
    let capabilities = snapshot(Metadata::default(), &events).unwrap();
    assert_eq!(capabilities.driver.name, "Registry description");
    assert_eq!(capabilities.driver_name, "Metadata IASIO");
    assert_eq!(capabilities.driver_version, 42);
    assert_eq!(capabilities.inputs.len(), 2);
    assert_eq!(capabilities.outputs.len(), 1);
    assert_eq!(capabilities.inputs[1].index, 1);
    assert_eq!(capabilities.inputs[0].name, "Input");
    assert_eq!(capabilities.outputs[0].name, "Main");
    assert!(!capabilities.inputs[0].is_active);
    assert!(capabilities.outputs[0].is_active);
    assert_eq!(capabilities.inputs[0].group, 7);
    assert_eq!(
        capabilities.inputs[0].sample_type,
        AsioSampleType::Float32Lsb
    );
    assert_eq!(
        capabilities.inputs[1].sample_type,
        AsioSampleType::DsdInt8Lsb1
    );
    assert_eq!(
        capabilities.outputs[0].sample_type,
        AsioSampleType::Unknown(9876)
    );
    assert_eq!(
        capabilities.buffer,
        AsioBufferCapabilities {
            min_frames: 64,
            max_frames: 4096,
            preferred_frames: 96,
            granularity: 32
        }
    );
    assert_eq!(
        capabilities
            .sample_rates
            .map(|rate| (rate.sample_rate, rate.supported)),
        [(44100, true), (48000, true), (88200, false), (96000, false)]
    );
    assert_eq!(capabilities.current_sample_rate, Some(48000.0));
    assert_eq!(
        capabilities.latencies,
        Some(AsioLatencies {
            input_frames: 32,
            output_frames: 64
        })
    );
    assert!(serde_json::to_value(&capabilities).unwrap()["current_sample_rate"].is_number());
    assert_released(&events);
}

#[test]
fn every_rejected_required_query_releases_driver_and_lease() {
    let _serial = callback::TEST_SERIAL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for operation in [
        "getChannels",
        "getBufferSize",
        "getChannelInfo",
        "canSampleRate",
        "getSampleRate",
        "getLatencies",
    ] {
        let events = Events::default();
        let error = snapshot(
            Metadata {
                failure: Some((operation, -999)),
                ..Metadata::default()
            },
            &events,
        )
        .unwrap_err();
        assert!(
            matches!(error, BackendError::Failed(_)),
            "{operation}: {error}"
        );
        assert!(error.to_string().contains(operation));
        assert_released(&events);
    }
    let events = Events::default();
    let error = snapshot(
        Metadata {
            fail_init: true,
            ..Metadata::default()
        },
        &events,
    )
    .unwrap_err();
    assert!(matches!(error, BackendError::ResourceUnavailable(_)));
    assert_released(&events);
}

#[test]
fn invalid_channel_counts_are_rejected_before_any_enumeration() {
    let _serial = callback::TEST_SERIAL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for (inputs, outputs) in [(-1, 1), (1, -1), (257, 0), (0, i32::MAX)] {
        let events = Events::default();
        assert!(
            snapshot(
                Metadata {
                    inputs,
                    outputs,
                    ..Metadata::default()
                },
                &events
            )
            .is_err()
        );
        assert!(!events.lock().unwrap().contains(&"getChannelInfo"));
        assert_released(&events);
    }
    let events = Events::default();
    let empty = snapshot(
        Metadata {
            inputs: 0,
            outputs: 0,
            ..Metadata::default()
        },
        &events,
    )
    .unwrap();
    assert!(empty.inputs.is_empty() && empty.outputs.is_empty());
    assert_released(&events);
    let events = Events::default();
    let bounded = snapshot(
        Metadata {
            inputs: 256,
            outputs: 256,
            ..Metadata::default()
        },
        &events,
    )
    .unwrap();
    assert_eq!((bounded.inputs.len(), bounded.outputs.len()), (256, 256));
    assert_eq!(
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|&&op| op == "getChannelInfo")
            .count(),
        512
    );
    assert_released(&events);
}

#[test]
fn invalid_buffer_metadata_never_yields_an_actionable_snapshot() {
    let _serial = callback::TEST_SERIAL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for sizes in [
        [0, 4096, 96, 32],
        [128, 64, 128, -1],
        [64, 4096, -1, 32],
        [64, 4096, 80, 32],
        [64, 4096, 96, -2],
    ] {
        let events = Events::default();
        assert!(
            snapshot(
                Metadata {
                    sizes,
                    ..Metadata::default()
                },
                &events
            )
            .is_err(),
            "{sizes:?}"
        );
        assert_released(&events);
    }
    // Discovery is not restricted to the application's streaming menu.
    let events = Events::default();
    let small = snapshot(
        Metadata {
            sizes: [16, 16, 16, 0],
            ..Metadata::default()
        },
        &events,
    )
    .unwrap();
    assert_eq!(small.buffer.preferred_frames, 16);
    assert_released(&events);
}

#[test]
fn unavailable_optional_metadata_is_not_fabricated() {
    let _serial = callback::TEST_SERIAL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for code in [-997, -995] {
        let events = Events::default();
        let capabilities = snapshot(
            Metadata {
                failure: Some(("getSampleRate", code)),
                ..Metadata::default()
            },
            &events,
        )
        .unwrap();
        assert_eq!(capabilities.current_sample_rate, None);
        assert_released(&events);
    }
    let events = Events::default();
    let capabilities = snapshot(
        Metadata {
            failure: Some(("getLatencies", -997)),
            ..Metadata::default()
        },
        &events,
    )
    .unwrap();
    assert_eq!(capabilities.latencies, None);
    assert_released(&events);
    for rate in [f64::NAN, f64::INFINITY, 0.0, -48000.0] {
        let events = Events::default();
        assert!(
            snapshot(
                Metadata {
                    rate,
                    ..Metadata::default()
                },
                &events
            )
            .is_err()
        );
        assert_released(&events);
    }
    let events = Events::default();
    assert!(
        snapshot(
            Metadata {
                latency: [-1, 0],
                ..Metadata::default()
            },
            &events
        )
        .is_err()
    );
    assert_released(&events);
}

#[test]
fn fixed_names_are_bounded_even_without_a_nul_terminator() {
    let _serial = callback::TEST_SERIAL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let events = Events::default();
    let capabilities = snapshot(
        Metadata {
            unterminated_names: true,
            ..Metadata::default()
        },
        &events,
    )
    .unwrap();
    assert_eq!(capabilities.driver_name, "N".repeat(32));
    assert_eq!(capabilities.inputs[0].name, "C".repeat(32));
    assert_released(&events);
}

#[test]
fn query_shares_the_stream_lease_before_attempting_activation() {
    let _serial = callback::TEST_SERIAL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let lease = callback::Exclusive::acquire().unwrap();
    let result = query_with_driver(|| panic!("busy query must not activate a driver"));
    assert!(matches!(result, Err(BackendError::ResourceUnavailable(_))));
    assert!(matches!(
        query_capabilities("not-a-clsid"),
        Err(BackendError::ResourceUnavailable(_))
    ));
    drop(lease);
    assert!(matches!(
        query_capabilities("not-a-clsid"),
        Err(BackendError::Unsupported(_))
    ));
    assert!(callback::Exclusive::acquire().is_ok());
}

#[test]
fn activation_failure_returns_the_lease_without_native_cleanup_calls() {
    let _serial = callback::TEST_SERIAL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let result = query_with_driver(|| {
        Err(BackendError::ResourceUnavailable(
            "fake activation rejected".into(),
        ))
    });
    assert!(matches!(result, Err(BackendError::ResourceUnavailable(_))));
    assert!(callback::Exclusive::acquire().is_ok());
}

thread_local! {
    static STAGES: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

struct StageLogger;

impl log::Log for StageLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target() == "moiren::audio::asio::capabilities"
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            STAGES.with(|stages| {
                if let Some(stages) = stages.borrow_mut().as_mut() {
                    assert!(stages.len() < 2048, "query trace must remain bounded");
                    stages.push(record.args().to_string());
                }
            });
        }
    }

    fn flush(&self) {}
}

#[test]
fn trace_brackets_native_queries_and_cleanup_even_when_init_or_query_fails() {
    let _serial = callback::TEST_SERIAL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    static LOGGER: StageLogger = StageLogger;
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(log::LevelFilter::Debug);
    for metadata in [
        Metadata::default(),
        Metadata {
            fail_init: true,
            ..Metadata::default()
        },
        Metadata {
            failure: Some(("getChannels", -999)),
            ..Metadata::default()
        },
    ] {
        STAGES.with(|stages| *stages.borrow_mut() = Some(Vec::new()));
        let events = Events::default();
        let _ = snapshot(metadata, &events);
        let stages = STAGES.with(|stages| stages.borrow_mut().take().unwrap());
        for operation in [
            "lease.acquire",
            "driver.open",
            "driver.init",
            "driver.Release",
            "apartment.drop",
            "lease.drop",
        ] {
            let enter = stages
                .iter()
                .position(|line| line.starts_with(&format!("query stage={operation} enter")))
                .unwrap_or_else(|| panic!("missing {operation} enter: {stages:?}"));
            let exit = stages
                .iter()
                .position(|line| line.starts_with(&format!("query stage={operation} exit")))
                .unwrap_or_else(|| panic!("missing {operation} exit: {stages:?}"));
            assert!(enter < exit, "{operation}");
        }
        let events = events.lock().unwrap();
        if events.contains(&"getBufferSize") {
            for operation in [
                "getDriverName",
                "getDriverVersion",
                "getChannels",
                "getBufferSize",
                "getSampleRate",
                "getLatencies",
                "canSampleRate",
                "getChannelInfo",
            ] {
                assert!(
                    stages
                        .iter()
                        .any(|line| line.starts_with(&format!("query stage={operation} enter"))),
                    "missing {operation}: {stages:?}"
                );
                assert!(
                    stages
                        .iter()
                        .any(|line| line.starts_with(&format!("query stage={operation} exit"))),
                    "missing {operation}: {stages:?}"
                );
            }
        }
    }
}
