//! A fake IASIO implementation exercises the actual stream without opening COM
//! drivers. Registry-only discovery and opt-in hardware tests are separate.

use super::*;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};

type Events = Arc<Mutex<Vec<&'static str>>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Failure {
    None,
    Rate,
    WrongRate,
    Buffer,
    NativePreferred,
    Format,
    MixedFormat,
    Create,
    NullBuffer,
    Start,
    Stop,
    Dispose,
}

#[repr(C)]
struct MockDriver {
    interface: abi::Driver,
    events: Events,
    failure: Failure,
    rate: f64,
    allocation: Vec<Box<[u8]>>,
    descriptors: *mut abi::BufferInfo,
    descriptor_count: usize,
    callback: *const abi::Callbacks,
}

unsafe fn mock<'a>(driver: *mut abi::Driver) -> &'a mut MockDriver {
    unsafe { &mut *driver.cast::<MockDriver>() }
}

fn record(driver: &MockDriver, event: &'static str) {
    driver.events.lock().unwrap().push(event);
}

unsafe extern "system" fn query(_: *mut abi::Driver, _: *const c_void, _: *mut *mut c_void) -> i32 {
    -1
}
unsafe extern "system" fn add_ref(_: *mut abi::Driver) -> u32 {
    1
}
unsafe extern "system" fn release(driver: *mut abi::Driver) -> u32 {
    assert!(
        callback::is_detached(),
        "Release must follow callback retirement"
    );
    let driver = unsafe { Box::from_raw(driver.cast::<MockDriver>()) };
    record(&driver, "release");
    if !driver.descriptors.is_null() {
        // Descriptor storage must still exist through final driver Release.
        let descriptor = unsafe { driver.descriptors.read() };
        assert_eq!(descriptor.channel_num, 0);
    }
    0
}
unsafe extern "system" fn init(driver: *mut abi::Driver, _: *mut c_void) -> i32 {
    record(unsafe { mock(driver) }, "init");
    1
}
unsafe extern "system" fn name(_: *mut abi::Driver, out: *mut u8) {
    unsafe { ptr::copy_nonoverlapping(c"Mock IASIO".as_ptr().cast(), out, 11) };
}
unsafe extern "system" fn version(_: *mut abi::Driver) -> i32 {
    2
}
unsafe extern "system" fn error(_: *mut abi::Driver, out: *mut u8) {
    unsafe { ptr::copy_nonoverlapping(c"mock rejected request".as_ptr().cast(), out, 22) };
}
unsafe extern "system" fn start(driver: *mut abi::Driver) -> i32 {
    let driver = unsafe { mock(driver) };
    record(driver, "start");
    if driver.failure == Failure::Start {
        return -999;
    }
    // Synchronous startup callbacks are legal, including before start returns.
    let switch = unsafe { (*driver.callback).buffer_switch };
    unsafe {
        switch(0, 1);
        switch(1, 1);
    }
    abi::OK
}
unsafe extern "system" fn stop(driver: *mut abi::Driver) -> i32 {
    assert!(
        callback::is_detached(),
        "stop must follow callback retirement"
    );
    let driver = unsafe { mock(driver) };
    record(driver, "stop");
    if driver.failure == Failure::Stop {
        -999
    } else {
        abi::OK
    }
}
unsafe extern "system" fn channels(_: *mut abi::Driver, input: *mut i32, output: *mut i32) -> i32 {
    unsafe {
        input.write(2);
        output.write(2);
    }
    abi::OK
}
unsafe extern "system" fn latencies(_: *mut abi::Driver, input: *mut i32, output: *mut i32) -> i32 {
    unsafe {
        input.write(32);
        output.write(64);
    }
    abi::OK
}
unsafe extern "system" fn sizes(
    driver: *mut abi::Driver,
    min: *mut i32,
    max: *mut i32,
    preferred: *mut i32,
    granularity: *mut i32,
) -> i32 {
    unsafe {
        min.write(if mock(driver).failure == Failure::Buffer {
            128
        } else {
            64
        });
        max.write(4096);
        preferred.write(if mock(driver).failure == Failure::NativePreferred {
            96
        } else {
            256
        });
        granularity.write(if mock(driver).failure == Failure::NativePreferred {
            32
        } else {
            -1
        });
    }
    abi::OK
}
unsafe extern "system" fn can_rate(driver: *mut abi::Driver, _: f64) -> i32 {
    if unsafe { mock(driver).failure } == Failure::Rate {
        -995
    } else {
        abi::OK
    }
}
unsafe extern "system" fn get_rate(driver: *mut abi::Driver, rate: *mut f64) -> i32 {
    let driver = unsafe { mock(driver) };
    unsafe {
        rate.write(if driver.failure == Failure::WrongRate {
            44100.0
        } else {
            driver.rate
        })
    };
    abi::OK
}
unsafe extern "system" fn set_rate(driver: *mut abi::Driver, rate: f64) -> i32 {
    unsafe { mock(driver).rate = rate };
    abi::OK
}
unsafe extern "system" fn clocks(
    _: *mut abi::Driver,
    _: *mut abi::ClockSource,
    _: *mut i32,
) -> i32 {
    -1000
}
unsafe extern "system" fn clock(_: *mut abi::Driver, _: i32) -> i32 {
    -1000
}
unsafe extern "system" fn position(
    _: *mut abi::Driver,
    _: *mut abi::Samples,
    _: *mut abi::Samples,
) -> i32 {
    -1000
}
unsafe extern "system" fn channel_info(
    driver: *mut abi::Driver,
    info: *mut abi::ChannelInfo,
) -> i32 {
    let driver = unsafe { mock(driver) };
    let info = unsafe { &mut *info };
    info.sample_type = match driver.failure {
        Failure::Format => 32,
        Failure::MixedFormat if info.is_input == 0 => 18,
        _ => 19,
    };
    abi::OK
}
unsafe extern "system" fn create(
    driver: *mut abi::Driver,
    descriptors: *mut abi::BufferInfo,
    channels: i32,
    frames: i32,
    callbacks: *mut abi::Callbacks,
) -> i32 {
    assert!(
        callback::is_detached(),
        "configuration must not publish audio yet"
    );
    let driver = unsafe { mock(driver) };
    record(driver, "create");
    driver.descriptors = descriptors;
    driver.descriptor_count = channels as usize;
    driver.callback = callbacks;
    for index in 0..channels as usize {
        let mut info = unsafe { descriptors.add(index).read() };
        for half in 0..2 {
            let mut bytes = vec![0; frames as usize * 4].into_boxed_slice();
            if info.is_input != 0 {
                for sample in bytes.as_chunks_mut::<4>().0 {
                    sample.copy_from_slice(&0.25f32.to_le_bytes());
                }
            }
            info.buffers[half] = bytes.as_mut_ptr().cast();
            driver.allocation.push(bytes);
        }
        if driver.failure == Failure::NullBuffer {
            info.buffers[1] = ptr::null_mut();
        }
        unsafe { descriptors.add(index).write(info) };
        if driver.failure == Failure::Create {
            return -999;
        }
    }
    abi::OK
}
unsafe extern "system" fn dispose(driver: *mut abi::Driver) -> i32 {
    assert!(
        callback::is_detached(),
        "dispose must follow callback retirement"
    );
    let driver = unsafe { mock(driver) };
    record(driver, "dispose");
    if driver.failure == Failure::Dispose {
        -999
    } else {
        abi::OK
    }
}
unsafe extern "system" fn panel(_: *mut abi::Driver) -> i32 {
    -1000
}
unsafe extern "system" fn future(_: *mut abi::Driver, _: i32, _: *mut c_void) -> i32 {
    -1000
}
unsafe extern "system" fn ready(_: *mut abi::Driver) -> i32 {
    -1000
}

static VTABLE: abi::VTable = abi::VTable {
    query_interface: query,
    add_ref,
    release,
    init,
    get_driver_name: name,
    get_driver_version: version,
    get_error_message: error,
    start,
    stop,
    get_channels: channels,
    get_latencies: latencies,
    get_buffer_size: sizes,
    can_sample_rate: can_rate,
    get_sample_rate: get_rate,
    set_sample_rate: set_rate,
    get_clock_sources: clocks,
    set_clock_source: clock,
    get_sample_position: position,
    get_channel_info: channel_info,
    create_buffers: create,
    dispose_buffers: dispose,
    control_panel: panel,
    future,
    output_ready: ready,
};

fn configured(failure: Failure, events: Events) -> Result<AsioStream, BackendError> {
    configured_frames(failure, events, 64)
}

fn configured_frames(
    failure: Failure,
    events: Events,
    frames: u32,
) -> Result<AsioStream, BackendError> {
    let lease = Exclusive::acquire()?;
    let driver = Box::new(MockDriver {
        interface: abi::Driver { vtable: &VTABLE },
        events,
        failure,
        rate: 0.0,
        allocation: Vec::new(),
        descriptors: ptr::null_mut(),
        descriptor_count: 0,
        callback: ptr::null(),
    });
    let driver = unsafe { Driver::from_test_raw(Box::into_raw(driver).cast()) };
    driver.initialize()?;
    AsioStream::configure(
        AsioOptions {
            driver_id: "mock".into(),
            sample_rate: 48000,
            buffer_frames: frames,
            input: true,
            output: true,
        },
        lease,
        None,
        driver,
    )
}

#[test]
fn actual_stream_lifecycle_duplex_conversion_and_failure_cleanup_use_mock_iasio_only() {
    let _serial = callback::TEST_SERIAL.lock().unwrap();
    let default_stream = configured_frames(Failure::None, Events::default(), 0).unwrap();
    assert_eq!(default_stream.info().buffer_frames, 256);
    assert_eq!(
        unsafe { mock(default_stream.driver().raw()) }.allocation[0].len(),
        256 * 4
    );
    default_stream.close().unwrap();
    let native_default = configured_frames(Failure::NativePreferred, Events::default(), 0).unwrap();
    assert_eq!(native_default.info().buffer_frames, 96);
    assert_eq!(
        unsafe { mock(native_default.driver().raw()) }.allocation[0].len(),
        96 * 4
    );
    native_default.close().unwrap();
    for failure in [
        Failure::Rate,
        Failure::WrongRate,
        Failure::Buffer,
        Failure::Format,
        Failure::MixedFormat,
    ] {
        let events = Events::default();
        assert!(configured(failure, Arc::clone(&events)).is_err());
        assert_eq!(*events.lock().unwrap(), ["init", "release"]);
    }
    for failure in [Failure::Create, Failure::NullBuffer] {
        let events = Events::default();
        assert!(configured(failure, Arc::clone(&events)).is_err());
        assert_eq!(
            *events.lock().unwrap(),
            ["init", "create", "dispose", "release"]
        );
    }
    for failure in [
        Failure::Start,
        Failure::Stop,
        Failure::Dispose,
        Failure::None,
    ] {
        let events = Events::default();
        let mut stream = configured(failure, Arc::clone(&events)).unwrap();
        assert_eq!(*events.lock().unwrap(), ["init", "create"]);
        assert_eq!(
            stream.info().format,
            moiren_engine::audio_config::SampleFormat::Float32
        );
        assert_ne!(
            stream.info().format,
            moiren_engine::audio_config::SampleFormat::Native
        );
        assert_eq!(stream.info().buffer_frames, 64);
        assert_eq!(stream.latencies(), Some((32, 64)));
        assert!(stream.wait(Duration::ZERO).is_err());
        assert_eq!(stream.write(&[[0.5, -0.5]; 64]).unwrap(), 64);
        if failure == Failure::Start {
            assert!(stream.start().is_err());
        } else {
            stream.start().unwrap();
            assert!(stream.wait(Duration::ZERO).unwrap());
            let mut capture = [[0.0; 2]; 80];
            assert_eq!(stream.read(&mut capture).unwrap(), 64);
            assert!(capture[..64].iter().all(|frame| *frame == [0.25; 2]));
            // Native output writes were performed by synchronous callbacks.
            let driver = unsafe { mock(stream.driver().raw()) };
            let left = &driver.allocation[4];
            let right = &driver.allocation[6];
            assert_eq!(&left[..4], &0.5f32.to_le_bytes());
            assert_eq!(&right[..4], &(-0.5f32).to_le_bytes());
            assert_eq!(stream.write(&[[0.75, -0.75]; 64]).unwrap(), 64);
            assert_eq!(stream.stop().is_err(), failure == Failure::Stop);
            if failure == Failure::None {
                assert_eq!(stream.read(&mut capture).unwrap(), 0);
                stream.start().unwrap();
                let driver = unsafe { mock(stream.driver().raw()) };
                assert!(driver.allocation[4].iter().all(|&byte| byte == 0));
                assert!(driver.allocation[6].iter().all(|&byte| byte == 0));
                assert_eq!(stream.read(&mut capture).unwrap(), 64);
                stream.stop().unwrap();
            }
        }
        assert_eq!(
            stream.close().is_err(),
            matches!(failure, Failure::Stop | Failure::Dispose)
        );
        let events = events.lock().unwrap();
        let dispose = events.iter().position(|&event| event == "dispose").unwrap();
        let release = events.iter().position(|&event| event == "release").unwrap();
        assert!(dispose < release);
        assert_eq!(events.last(), Some(&"release"));
        assert!(callback::is_detached());
        assert!(Exclusive::acquire().is_ok());
    }
}

#[test]
fn registered_driver_discovery_is_registry_only_and_returns_stable_unique_clsids() {
    let drivers = super::super::list_drivers().unwrap();
    let mut ids = std::collections::BTreeSet::new();
    for driver in drivers {
        assert!(!driver.name.is_empty());
        assert_eq!(
            registry::canonical_id(&registry::parse_id(&driver.id).unwrap()),
            driver.id
        );
        assert!(ids.insert(driver.id));
    }
}

#[test]
fn notifications_preserve_request_until_teardown_and_stop_errors_are_terminal() {
    use crate::native::{NativeLatency, NativeRecoveryRequest};
    let _serial = callback::TEST_SERIAL.lock().unwrap();
    for (selector, expected) in [
        (3, NativeRecoveryRequest::Reset),
        (5, NativeRecoveryRequest::Resync),
        (4, NativeRecoveryRequest::BufferSizeChanged),
        (0, NativeRecoveryRequest::SampleRateChanged),
    ] {
        let events = Events::default();
        let mut stream = configured(Failure::None, events.clone()).unwrap();
        stream.start().unwrap();
        assert_eq!(
            stream.driver_latency(),
            Some(NativeLatency {
                input_frames: 32,
                output_frames: 64
            })
        );
        let callbacks = unsafe { &*mock(stream.driver().raw()).callback };
        unsafe {
            if selector == 0 {
                (callbacks.sample_rate_changed)(44100.0);
            } else {
                (callbacks.message)(selector, 0, ptr::null_mut(), ptr::null_mut());
            }
        }
        assert!(stream.wait(Duration::ZERO).is_err());
        assert_eq!(stream.recovery_request(), Some(expected));
        stream.stop().unwrap();
        stream.release().unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            ["init", "create", "start", "stop", "dispose", "release"]
        );
    }
    let mut stream = configured(Failure::Stop, Events::default()).unwrap();
    stream.start().unwrap();
    let callbacks = unsafe { &*mock(stream.driver().raw()).callback };
    unsafe {
        (callbacks.message)(3, 0, ptr::null_mut(), ptr::null_mut());
    }
    assert!(stream.stop().is_err());
    assert_eq!(stream.recovery_request(), None);
}

#[test]
fn owner_wait_dispatches_driver_sta_messages_without_audio_thread_work() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use windows::{
        Win32::{
            Foundation::{HWND, LPARAM, LRESULT, WPARAM},
            UI::WindowsAndMessaging::*,
        },
        core::w,
    };
    static DISPATCHED: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "system" fn procedure(
        window: HWND,
        message: u32,
        w: WPARAM,
        l: LPARAM,
    ) -> LRESULT {
        if message == WM_APP + 21 {
            DISPATCHED.fetch_add(1, Ordering::Relaxed);
            return LRESULT(0);
        }
        unsafe { DefWindowProcW(window, message, w, l) }
    }
    let _serial = callback::TEST_SERIAL.lock().unwrap();
    let class = w!("STATIC");
    let window = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            w!(""),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            None,
            None,
        )
    }
    .unwrap();
    unsafe {
        SetWindowLongPtrW(window, GWLP_WNDPROC, procedure as *const () as isize);
    }
    let mut stream = configured(Failure::None, Events::default()).unwrap();
    stream.start().unwrap();
    unsafe { PostMessageW(Some(window), WM_APP + 21, WPARAM(0), LPARAM(0)) }.unwrap();
    stream.wait(Duration::ZERO).unwrap(); // Both capture and audio wake are ready.
    let with_capture = DISPATCHED.load(Ordering::Relaxed);
    stream.read(&mut [[0.0; 2]; 64]).unwrap();
    stream.wait(Duration::ZERO).unwrap(); // Consume synchronous startup wake.
    unsafe { PostMessageW(Some(window), WM_APP + 21, WPARAM(0), LPARAM(0)) }.unwrap();
    stream.wait(Duration::from_millis(10)).unwrap();
    let dispatched = DISPATCHED.load(Ordering::Relaxed);
    for _ in 0..3 {
        unsafe {
            PostMessageW(Some(window), WM_APP + 21, WPARAM(0), LPARAM(0)).unwrap();
            windows::Win32::System::Threading::SetEvent(stream.event.0).unwrap();
        }
        stream.wait(Duration::ZERO).unwrap();
    }
    let with_audio_event = DISPATCHED.load(Ordering::Relaxed);
    for _ in 0..65 {
        unsafe { PostMessageW(Some(window), WM_APP + 21, WPARAM(0), LPARAM(0)) }.unwrap();
    }
    let mut flood_batches = Vec::new();
    for _ in 0..3 {
        unsafe { windows::Win32::System::Threading::SetEvent(stream.event.0) }.unwrap();
        stream.wait(Duration::ZERO).unwrap();
        flood_batches.push(DISPATCHED.load(Ordering::Relaxed));
    }
    stream.stop().unwrap();
    stream.close().unwrap();
    unsafe {
        DestroyWindow(window).unwrap();
    }
    assert_eq!(with_capture, 1, "capture backlog must not starve STA work");
    assert_eq!(dispatched, 2);
    assert_eq!(with_audio_event, 5, "audio wake must not starve STA work");
    assert_eq!(flood_batches, [37, 69, 70], "32-message budget per wait");
}

#[test]
#[ignore = "machine-specific registry inventory; does not open drivers or play audio"]
fn local_registry_contains_fl_studio_and_realtek_exact_clsids() {
    let drivers = super::super::list_drivers().unwrap();
    for (id, name) in [
        ("{188135E1-7171-3434-854F-01A3C71F3DF9}", "FL Studio ASIO"),
        ("{A80362FF-CE76-4DD9-874A-704C57BF0D6A}", "Realtek ASIO"),
    ] {
        assert!(
            drivers
                .iter()
                .any(|driver| driver.id == id && driver.name == name),
            "missing expected registry-only inventory: {name} {id}"
        );
    }
}

#[test]
#[ignore = "opens hardware: explicitly set MOIREN_ASIO_HARDWARE_TEST_DRIVER and select this test"]
fn explicitly_opted_in_hardware_duplex_silence() {
    let id = std::env::var("MOIREN_ASIO_HARDWARE_TEST_DRIVER")
        .expect("set MOIREN_ASIO_HARDWARE_TEST_DRIVER to explicitly authorize device activation");
    let mut stream = AsioStream::open(AsioOptions {
        driver_id: id,
        sample_rate: 48000,
        buffer_frames: 256,
        input: true,
        output: true,
    })
    .unwrap();
    stream.write(&[[0.0; 2]; 256]).unwrap();
    stream.start().unwrap();
    let mut captured = 0;
    for _ in 0..16 {
        if stream.wait(Duration::from_millis(100)).unwrap() {
            captured += stream.read(&mut [[0.0; 2]; 256]).unwrap();
            stream.write(&[[0.0; 2]; 256]).unwrap();
        }
        if captured >= 256 {
            break;
        }
    }
    stream.stop().unwrap();
    stream.close().unwrap();
    assert!(captured >= 256, "driver supplied no capture frames");
}
