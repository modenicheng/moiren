//! No driver calls, allocation, locks, conversion, or reclamation on callbacks.
//! The global reader barrier includes callbacks which have not loaded the
//! context yet, closing the load-pointer / retire-pointer use-after-free race.

use std::{
    cell::UnsafeCell,
    ffi::c_void,
    ptr,
    sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering},
};
use windows::Win32::Foundation::HANDLE;

use super::super::native::{NativeRecoveryRequest, NativeStreamStats};
use super::{BackendError, abi, sample::Frame};

static EXCLUSIVE: AtomicBool = AtomicBool::new(false);
static CONTEXT: AtomicPtr<Context> = AtomicPtr::new(ptr::null_mut());
static READERS: AtomicUsize = AtomicUsize::new(0);

// Avoid the windows Result wrapper on the callback: constructing its Error on
// a failed SetEvent can capture COM error information and allocate.
#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetEvent(event: HANDLE) -> i32;
}

#[cfg(test)]
pub(super) static TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(super) fn is_detached() -> bool {
    CONTEXT.load(Ordering::SeqCst).is_null() && READERS.load(Ordering::SeqCst) == 0
}

pub(super) const RATE_CHANGED: u32 = 1;
const RESET_REQUEST: u32 = 2;
const RESYNC_REQUEST: u32 = 3;
const BUFFER_CHANGED: u32 = 4;
const REENTRANT: u32 = 5;
const BAD_INDEX: u32 = 6;
const EVENT_FAILED: u32 = 7;

const SELECTOR_SUPPORTED: i32 = 1;
const ENGINE_VERSION: i32 = 2;
const RESET_SELECTOR: i32 = 3;
const BUFFER_SIZE_SELECTOR: i32 = 4;
const RESYNC_SELECTOR: i32 = 5;
const LATENCIES_CHANGED_SELECTOR: i32 = 6;
const SUPPORTS_TIME_INFO: i32 = 7;
const SUPPORTS_TIME_CODE: i32 = 8;

pub(super) struct Exclusive(());

impl Exclusive {
    pub fn acquire() -> Result<Self, BackendError> {
        EXCLUSIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                BackendError::ResourceUnavailable(
                    "another ASIO stream owns this process's exclusive callback context".into(),
                )
            })?;
        Ok(Self(()))
    }

    /// # Safety
    /// `context` must stay at this address until `detach` has returned. The
    /// caller must stop/dispose/release the old driver before publishing a new
    /// one; old drivers may not deliver callbacks into a subsequent session.
    pub unsafe fn publish(&self, context: &Context) {
        debug_assert!(CONTEXT.load(Ordering::SeqCst).is_null());
        CONTEXT.store(ptr::from_ref(context).cast_mut(), Ordering::SeqCst);
    }

    pub fn detach(&self) {
        CONTEXT.store(ptr::null_mut(), Ordering::SeqCst);
        // Only the non-RT owner waits. New callbacks see null and leave without
        // touching any session memory. The seq-cst count precedes pointer load.
        while READERS.load(Ordering::SeqCst) != 0 {
            std::thread::yield_now();
        }
    }
}

impl Drop for Exclusive {
    fn drop(&mut self) {
        self.detach();
        EXCLUSIVE.store(false, Ordering::Release);
    }
}

struct Reader;

impl Reader {
    fn enter() -> Self {
        READERS.fetch_add(1, Ordering::SeqCst);
        Self
    }
    fn context(&self) -> Option<&Context> {
        // SAFETY: the owner detaches and waits for this reader before freeing.
        unsafe { CONTEXT.load(Ordering::SeqCst).as_ref() }
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        READERS.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy)]
pub(super) struct Channel {
    pub buffers: [*mut u8; 2],
    pub bytes: usize,
}

pub(super) struct Data {
    pub input: Option<rtrb::Producer<Frame>>,
    pub output: Option<rtrb::Consumer<Frame>>,
    pub inputs: [Option<Channel>; 2],
    pub outputs: [Option<Channel>; 2],
    pub frames: usize,
    pub discard_first_input: bool,
}

pub(super) struct Context {
    data: UnsafeCell<Data>,
    copying: AtomicBool,
    fault: AtomicU32,
    callbacks: AtomicU64,
    capture_dropped: AtomicU64,
    render_silent: AtomicU64,
    pub latency_changed: AtomicBool,
    sample_rate: u32,
    event: HANDLE,
}

impl Context {
    pub fn new(data: Data, sample_rate: u32, event: HANDLE) -> Self {
        Self {
            data: UnsafeCell::new(data),
            copying: AtomicBool::new(false),
            fault: AtomicU32::new(0),
            callbacks: AtomicU64::new(0),
            capture_dropped: AtomicU64::new(0),
            render_silent: AtomicU64::new(0),
            latency_changed: AtomicBool::new(false),
            sample_rate,
            event,
        }
    }

    /// Called only while detached and with every previous reader retired.
    pub fn prepare(&mut self) {
        self.fault.store(0, Ordering::Relaxed);
        self.callbacks.store(0, Ordering::Relaxed);
        self.capture_dropped.store(0, Ordering::Relaxed);
        self.render_silent.store(0, Ordering::Relaxed);
        self.latency_changed.store(false, Ordering::Relaxed);
        self.data.get_mut().discard_first_input = true;
    }

    /// Owner-only, detached cleanup; no samples from a previous run may play
    /// after a successful stop/start pair.
    pub fn discard_output(&mut self) {
        if let Some(output) = self.data.get_mut().output.as_mut() {
            while output.pop().is_ok() {}
        }
    }

    pub fn check(&self) -> Result<(), BackendError> {
        let reason = match self.fault.load(Ordering::Acquire) {
            0 => return Ok(()),
            RATE_CHANGED => "sample rate changed or external clock was lost",
            RESET_REQUEST => "driver requested reset",
            RESYNC_REQUEST => "driver requested resynchronization",
            BUFFER_CHANGED => "driver changed buffer size",
            REENTRANT => "driver delivered overlapping audio callbacks",
            BAD_INDEX => "driver supplied an invalid double-buffer index",
            _ => "callback wake event failed",
        };
        Err(BackendError::ResourceUnavailable(format!(
            "ASIO {reason}; stop and reopen the stream"
        )))
    }

    pub fn recovery_request(&self) -> Option<NativeRecoveryRequest> {
        match self.fault.load(Ordering::Acquire) {
            RATE_CHANGED => Some(NativeRecoveryRequest::SampleRateChanged),
            RESET_REQUEST => Some(NativeRecoveryRequest::Reset),
            RESYNC_REQUEST => Some(NativeRecoveryRequest::Resync),
            BUFFER_CHANGED => Some(NativeRecoveryRequest::BufferSizeChanged),
            _ => None,
        }
    }

    pub fn stats(&self) -> NativeStreamStats {
        NativeStreamStats {
            callbacks: self.callbacks.load(Ordering::Relaxed),
            capture_dropped_frames: self.capture_dropped.load(Ordering::Relaxed),
            render_silent_frames: self.render_silent.load(Ordering::Relaxed),
            ..Default::default()
        }
    }

    fn fail(&self, fault: u32) {
        let _ = self
            .fault
            .compare_exchange(0, fault, Ordering::AcqRel, Ordering::Relaxed);
        self.signal();
    }

    fn signal(&self) {
        // SAFETY: the owner closes the event only after detaching all readers.
        if unsafe { SetEvent(self.event) } == 0 {
            let _ =
                self.fault
                    .compare_exchange(0, EVENT_FAILED, Ordering::AcqRel, Ordering::Relaxed);
        }
    }

    fn copy(&self, index: i32) {
        if !(0..=1).contains(&index) {
            self.fail(BAD_INDEX);
            return;
        }
        // ASIO requires sequential buffer switches. A broken driver must not
        // create aliased mutable ring endpoints; reject overlap without waiting.
        if self
            .copying
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            self.fail(REENTRANT);
            return;
        }
        // SAFETY: copying is the unique audio callback gate. The owner never
        // accesses Data while published; message callbacks touch atomics only.
        let data = unsafe { &mut *self.data.get() };
        self.callbacks.fetch_add(1, Ordering::Relaxed);
        let index = index as usize;
        if !data.discard_first_input
            && let Some(input) = data.input.as_mut()
        {
            if input.slots() < data.frames {
                self.capture_dropped
                    .fetch_add(data.frames as u64, Ordering::Relaxed);
            } else {
                for frame_index in 0..data.frames {
                    let mut frame = Frame::default();
                    for (channel_index, channel) in data.inputs.iter().enumerate() {
                        if let Some(channel) = channel {
                            // SAFETY: the driver allocated two complete buffers,
                            // validated before publication and disposed after retire.
                            unsafe {
                                ptr::copy_nonoverlapping(
                                    channel.buffers[index].add(frame_index * channel.bytes),
                                    frame.samples[channel_index].as_mut_ptr(),
                                    channel.bytes,
                                );
                            }
                        }
                    }
                    // The consumer can only free slots since the capacity check.
                    let _ = input.push(frame);
                }
            }
        }
        data.discard_first_input = false;
        if let Some(output) = data.output.as_mut() {
            let mut silent = 0;
            // Underrun writes native zeroes, never replays stale device memory.
            for frame_index in 0..data.frames {
                let frame = output.pop().unwrap_or_else(|_| {
                    silent += 1;
                    Frame::default()
                });
                for (channel_index, channel) in data.outputs.iter().enumerate() {
                    if let Some(channel) = channel {
                        // SAFETY: same validated, live buffer bounds as above.
                        unsafe {
                            ptr::copy_nonoverlapping(
                                frame.samples[channel_index].as_ptr(),
                                channel.buffers[index].add(frame_index * channel.bytes),
                                channel.bytes,
                            );
                        }
                    }
                }
            }
            self.render_silent.fetch_add(silent, Ordering::Relaxed);
        }
        self.copying.store(false, Ordering::Release);
        self.signal();
    }
}

unsafe extern "C" fn buffer_switch(index: i32, _direct: i32) {
    let reader = Reader::enter();
    if let Some(context) = reader.context() {
        context.copy(index);
    }
}

unsafe extern "C" fn buffer_switch_time_info(
    params: *mut abi::Time,
    index: i32,
    direct: i32,
) -> *mut abi::Time {
    // No pointer is retained after return; timestamps are not needed by this
    // bounded native transport. In particular, do not call back into IASIO.
    unsafe { buffer_switch(index, direct) };
    params
}

unsafe extern "C" fn sample_rate_changed(rate: f64) {
    let reader = Reader::enter();
    if let Some(context) = reader.context()
        && rate != f64::from(context.sample_rate)
    {
        context.fail(RATE_CHANGED);
    }
}

unsafe extern "C" fn message(
    selector: i32,
    value: i32,
    _message: *mut c_void,
    _optional: *mut f64,
) -> i32 {
    // These capability queries also occur during createBuffers, before any
    // context is published. Time-code and buffer-size renegotiation are absent.
    match selector {
        SELECTOR_SUPPORTED => {
            return i32::from(matches!(
                value,
                ENGINE_VERSION
                    | RESET_SELECTOR
                    | RESYNC_SELECTOR
                    | SUPPORTS_TIME_INFO
                    | LATENCIES_CHANGED_SELECTOR
            ));
        }
        ENGINE_VERSION => return 2,
        // kAsioSupportsTimeInfo is a capability query. It must answer even
        // while the session pointer is detached between stop and restart.
        SUPPORTS_TIME_INFO => return 1,
        SUPPORTS_TIME_CODE => return 0,
        _ => {}
    }
    let reader = Reader::enter();
    let Some(context) = reader.context() else {
        return 0;
    };
    match selector {
        RESET_SELECTOR => context.fail(RESET_REQUEST),
        BUFFER_SIZE_SELECTOR => {
            // Unsupported by the ASIO specification, and never resize live
            // buffers. Wake the owner if a driver sends it anyway.
            context.fail(BUFFER_CHANGED);
            return 0;
        }
        RESYNC_SELECTOR => context.fail(RESYNC_REQUEST),
        LATENCIES_CHANGED_SELECTOR => {
            context.latency_changed.store(true, Ordering::Release);
            context.signal();
        }
        _ => return 0,
    }
    1
}

// A stable address even if a driver retains the callback table until Release.
pub(super) static CALLBACKS: abi::Callbacks = abi::Callbacks {
    buffer_switch,
    sample_rate_changed,
    message,
    buffer_switch_time_info,
};

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::{Foundation::CloseHandle, System::Threading::CreateEventW};

    #[test]
    fn callback_copies_native_bytes_with_silence_on_underrun_and_whole_block_overflow() {
        // Global callback tests share a test-only mutex with the fake driver.
        let _serial = TEST_SERIAL.lock().unwrap();
        let lease = Exclusive::acquire().unwrap();
        assert!(Exclusive::acquire().is_err());
        let event = unsafe { CreateEventW(None, false, false, None) }.unwrap();
        let (capture_tx, mut capture_rx) = rtrb::RingBuffer::new(4);
        let (mut render_tx, render_rx) = rtrb::RingBuffer::new(2);
        let input = [[1u8, 2, 3, 4], [5u8, 6, 7, 8]];
        let mut output = [[0xffu8; 4]; 2];
        let mut context = Box::new(Context::new(
            Data {
                input: Some(capture_tx),
                output: Some(render_rx),
                inputs: [
                    Some(Channel {
                        buffers: [input[0].as_ptr().cast_mut(), input[1].as_ptr().cast_mut()],
                        bytes: 2,
                    }),
                    None,
                ],
                outputs: [
                    Some(Channel {
                        buffers: [output[0].as_mut_ptr(), output[1].as_mut_ptr()],
                        bytes: 2,
                    }),
                    None,
                ],
                frames: 2,
                discard_first_input: true,
            },
            48000,
            event,
        ));
        let mut sample = Frame::default();
        sample.samples[0][..2].copy_from_slice(&[9, 10]);
        render_tx.push(sample).unwrap();
        unsafe {
            lease.publish(&context);
            buffer_switch(0, 1);
        }
        assert_eq!(capture_rx.slots(), 0); // First ASIO input buffer is invalid.
        assert_eq!(output[0], [9, 10, 0, 0]);
        unsafe { buffer_switch(1, 1) };
        assert_eq!(&capture_rx.pop().unwrap().samples[0][..2], &[5, 6]);
        // One capture block fits in the two remaining queue slots.
        unsafe { buffer_switch(0, 1) };
        assert_eq!(capture_rx.slots(), 3);
        assert_eq!(&capture_rx.pop().unwrap().samples[0][..2], &[7, 8]);
        // The next block fills the queue; the following whole block must drop
        // instead of being partially copied.
        unsafe { buffer_switch(1, 1) };
        assert_eq!(capture_rx.slots(), 4);
        unsafe { buffer_switch(0, 1) };
        assert_eq!(capture_rx.slots(), 4);
        let stats = context.stats();
        assert_eq!(stats.callbacks, 5);
        assert_eq!(stats.capture_dropped_frames, 2);
        assert_eq!(stats.render_silent_frames, 9);
        assert_eq!(&capture_rx.pop().unwrap().samples[0][..2], &[1, 2]);
        assert_eq!(&capture_rx.pop().unwrap().samples[0][..2], &[3, 4]);
        assert_eq!(&capture_rx.pop().unwrap().samples[0][..2], &[5, 6]);
        assert_eq!(&capture_rx.pop().unwrap().samples[0][..2], &[7, 8]);
        assert_eq!(output[0], [0; 4]);
        // Hold an entry across detach, including one which already obtained a
        // reference. Detach must wait for its final use, not merely clear ptr.
        let retired = AtomicBool::new(false);
        let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(0);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let reader = Reader::enter();
                let current = reader.context().unwrap();
                entered_tx.send(()).unwrap();
                while !CONTEXT.load(Ordering::SeqCst).is_null() {
                    std::thread::yield_now();
                }
                current.check().unwrap();
                retired.store(true, Ordering::Release);
            });
            entered_rx.recv().unwrap();
            lease.detach();
            assert!(retired.load(Ordering::Acquire));
        });
        context.check().unwrap();
        context.prepare();
        assert_eq!(context.stats().callbacks, 0);
        assert_eq!(context.stats().capture_dropped_frames, 0);
        assert_eq!(context.stats().render_silent_frames, 0);
        unsafe {
            lease.publish(&context);
            sample_rate_changed(44100.0);
        }
        lease.detach();
        assert!(context.check().is_err());
        drop(context);
        // All entry points safely ignore late calls while detached.
        unsafe {
            buffer_switch(0, 1);
            sample_rate_changed(0.0);
            assert_eq!(
                message(SUPPORTS_TIME_INFO, 0, ptr::null_mut(), ptr::null_mut()),
                1
            );
            assert_eq!(
                message(SUPPORTS_TIME_CODE, 0, ptr::null_mut(), ptr::null_mut()),
                0
            );
            CloseHandle(event).unwrap();
        }
        drop(lease);
        assert!(Exclusive::acquire().is_ok());
    }

    #[test]
    fn notification_recovery_is_typed_and_latency_support_is_advertised() {
        use super::super::super::native::NativeRecoveryRequest;
        let event = unsafe { CreateEventW(None, false, false, None) }.unwrap();
        let mut context = Context::new(
            Data {
                input: None,
                output: None,
                inputs: [None; 2],
                outputs: [None; 2],
                frames: 2,
                discard_first_input: true,
            },
            48000,
            event,
        );
        for (fault, request) in [
            (RATE_CHANGED, NativeRecoveryRequest::SampleRateChanged),
            (RESET_REQUEST, NativeRecoveryRequest::Reset),
            (RESYNC_REQUEST, NativeRecoveryRequest::Resync),
            (BUFFER_CHANGED, NativeRecoveryRequest::BufferSizeChanged),
        ] {
            context.prepare();
            context.fail(fault);
            assert_eq!(context.recovery_request(), Some(request));
        }
        for fault in [REENTRANT, BAD_INDEX, EVENT_FAILED] {
            context.prepare();
            context.fail(fault);
            assert_eq!(context.recovery_request(), None);
        }
        unsafe {
            assert_eq!(
                message(
                    SELECTOR_SUPPORTED,
                    LATENCIES_CHANGED_SELECTOR,
                    ptr::null_mut(),
                    ptr::null_mut()
                ),
                1
            );
            CloseHandle(event).unwrap();
        }
    }
}
