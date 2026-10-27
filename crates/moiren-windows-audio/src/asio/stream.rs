use std::{ptr, sync::atomic::Ordering, time::Duration};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{CreateEventW, ResetEvent},
    UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE,
        PeekMessageW, QS_ALLINPUT, TranslateMessage,
    },
};

use super::{
    AsioOptions, BackendError, abi,
    buffer::{BufferSizes, validate_options},
    callback::{self, Channel, Context, Data, Exclusive},
    driver::{Apartment, Driver},
    registry,
    sample::{Encoding, Frame},
};
use crate::native::{
    NativeLatency, NativeRecoveryRequest, NativeStream, NativeStreamInfo, NativeStreamStats,
};

struct Event(HANDLE);

impl Event {
    fn new() -> Result<Self, BackendError> {
        unsafe { CreateEventW(None, false, false, None) }
            .map(Self)
            .map_err(|error| BackendError::Failed(format!("ASIO wake event: {error}")))
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn dispatch_owner_messages(remaining: &mut usize) {
    let mut message = MSG::default();
    while *remaining != 0 {
        if !unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            break;
        }
        *remaining -= 1;
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

pub struct AsioStream {
    driver: Option<Driver>,
    // Kept until final driver Release; neither the driver nor this stream is Send.
    _apartment: Option<Apartment>,
    lease: Exclusive,
    event: Event,
    context: Option<Box<Context>>,
    capture: Option<rtrb::Consumer<Frame>>,
    render: Option<rtrb::Producer<Frame>>,
    inputs: [Option<Encoding>; 2],
    outputs: [Option<Encoding>; 2],
    // Drivers may retain these descriptors until disposeBuffers or Release.
    buffers: Vec<abi::BufferInfo>,
    buffers_attempted: bool,
    started: bool,
    terminal: bool,
    info: NativeStreamInfo,
    latencies: Option<(u32, u32)>,
}

impl AsioStream {
    pub fn open(options: AsioOptions) -> Result<Self, BackendError> {
        let driver_id = options.driver_id.clone();
        let sample_rate = options.sample_rate;
        let buffer_frames = options.buffer_frames;
        let input = options.input;
        let output = options.output;
        let result = Self::open_inner(options);
        if let Err(error) = &result {
            log::error!(
                target: "moiren::audio::asio",
                "ASIO stream open failed: driver={driver_id:?} input={input} output={output} requested_sample_rate={sample_rate} requested_buffer_frames={buffer_frames} error={error}"
            );
        }
        result
    }

    fn open_inner(options: AsioOptions) -> Result<Self, BackendError> {
        validate_options(
            options.sample_rate,
            options.buffer_frames,
            options.input,
            options.output,
        )?;
        let lease = Exclusive::acquire()?;
        let clsid = registry::parse_id(&options.driver_id)?;
        let id = registry::canonical_id(&clsid);
        if !registry::list()?.iter().any(|driver| driver.id == id) {
            return Err(BackendError::ResourceUnavailable(format!(
                "ASIO driver {id} is not registered in the 64-bit SOFTWARE\\ASIO registry; \
                 install the device manufacturer's matching driver"
            )));
        }
        let apartment = Apartment::new()?;
        log::debug!(target: "moiren::audio::asio", "stream stage=CoCreateInstance enter");
        let driver = Driver::open(&clsid)?;
        log::debug!(target: "moiren::audio::asio", "stream stage=CoCreateInstance exit; init enter");
        driver.initialize()?;
        log::debug!(target: "moiren::audio::asio", "stream stage=init exit; configure enter");
        Self::configure(options, lease, Some(apartment), driver)
    }

    fn configure(
        options: AsioOptions,
        lease: Exclusive,
        apartment: Option<Apartment>,
        driver: Driver,
    ) -> Result<Self, BackendError> {
        let event = Event::new()?;
        let mut stream = Self {
            driver: Some(driver),
            _apartment: apartment,
            lease,
            event,
            context: None,
            capture: None,
            render: None,
            inputs: [None; 2],
            outputs: [None; 2],
            buffers: Vec::with_capacity(4),
            buffers_attempted: false,
            started: false,
            terminal: false,
            info: NativeStreamInfo {
                sample_rate: options.sample_rate,
                format: moiren_engine::audio_config::SampleFormat::Native,
                buffer_frames: options.buffer_frames,
                channels: 0,
            },
            latencies: None,
        };
        stream.prepare(options)?;
        Ok(stream)
    }

    fn driver(&self) -> &Driver {
        self.driver.as_ref().expect("live ASIO driver")
    }

    fn prepare(&mut self, options: AsioOptions) -> Result<(), BackendError> {
        let driver = self.driver();
        let raw = driver.raw();
        let vt = driver.vtable();
        let rate = f64::from(options.sample_rate);
        // SAFETY: all IASIO calls use the sole owner-thread reference and exact
        // SDK ABI. Every output pointer targets initialized owner-thread memory.
        unsafe {
            driver.check("canSampleRate", (vt.can_sample_rate)(raw, rate))?;
            log::debug!(target: "moiren::audio::asio", "stream stage=setSampleRate enter rate={rate}");
            driver.check("setSampleRate", (vt.set_sample_rate)(raw, rate))?;
            log::debug!(target: "moiren::audio::asio", "stream stage=setSampleRate exit");
        }
        self.verify_rate()?;
        let mut sizes = BufferSizes {
            min: 0,
            max: 0,
            preferred: 0,
            granularity: 0,
        };
        let mut inputs = 0;
        let mut outputs = 0;
        unsafe {
            driver.check(
                "getBufferSize",
                (vt.get_buffer_size)(
                    raw,
                    &mut sizes.min,
                    &mut sizes.max,
                    &mut sizes.preferred,
                    &mut sizes.granularity,
                ),
            )?;
            driver.check(
                "getChannels",
                (vt.get_channels)(raw, &mut inputs, &mut outputs),
            )?;
        }
        self.info.buffer_frames = sizes.resolve(options.buffer_frames)?;
        if (options.input && inputs <= 0) || (options.output && outputs <= 0) {
            return Err(BackendError::ResourceUnavailable(format!(
                "ASIO requested direction is unavailable (driver inputs={inputs}, outputs={outputs})"
            )));
        }
        let input_count = if options.input { inputs.min(2) } else { 0 };
        let output_count = if options.output { outputs.min(2) } else { 0 };
        self.info.channels = input_count.max(output_count) as usize;
        for (is_input, count) in [(true, input_count), (false, output_count)] {
            for channel in 0..count {
                self.buffers.push(abi::BufferInfo {
                    is_input: i32::from(is_input),
                    channel_num: channel,
                    buffers: [ptr::null_mut(); 2],
                });
            }
        }
        self.query_encodings()?;
        let driver = self.driver();
        // Record the attempt before calling: partially prepared drivers must
        // also receive disposeBuffers on a failed createBuffers path.
        let raw = driver.raw();
        let create = driver.vtable().create_buffers;
        self.buffers_attempted = true;
        log::debug!(target: "moiren::audio::asio", "stream stage=createBuffers enter frames={}", self.info.buffer_frames);
        let code = unsafe {
            create(
                raw,
                self.buffers.as_mut_ptr(),
                self.buffers.len() as i32,
                self.info.buffer_frames as i32,
                // Table storage is static; the C API's pointer is non-const.
                ptr::from_ref(&callback::CALLBACKS).cast_mut(),
            )
        };
        self.driver().check("createBuffers", code)?;
        log::debug!(target: "moiren::audio::asio", "stream stage=createBuffers exit");
        // Re-query after preparation: use actual types, not the initial hints.
        self.query_encodings()?;
        self.verify_rate()?;
        let mut input_buffers = [None; 2];
        let mut output_buffers = [None; 2];
        for descriptor in &self.buffers {
            let input = descriptor.is_input != 0;
            let channel = descriptor.channel_num as usize;
            let encoding = if input {
                self.inputs[channel]
            } else {
                self.outputs[channel]
            }
            .expect("queried channel encoding");
            let buffers = descriptor.buffers.map(|buffer| buffer.cast::<u8>());
            if buffers.iter().any(|buffer| buffer.is_null()) {
                return Err(BackendError::Failed(
                    "ASIO createBuffers returned a null channel buffer".into(),
                ));
            }
            let buffer = Some(Channel {
                buffers,
                bytes: encoding.bytes,
            });
            if input {
                input_buffers[channel] = buffer;
            } else {
                output_buffers[channel] = buffer;
            }
        }
        // Four periods bound capture backlog and output look-ahead. Ring
        // endpoints are freed by the owner only, after retiring the callback.
        let capacity = self.info.buffer_frames as usize * 4;
        let capture = if options.input {
            let (producer, consumer) = rtrb::RingBuffer::new(capacity);
            self.capture = Some(consumer);
            Some(producer)
        } else {
            None
        };
        let render = if options.output {
            let (producer, consumer) = rtrb::RingBuffer::new(capacity);
            self.render = Some(producer);
            Some(consumer)
        } else {
            None
        };
        self.context = Some(Box::new(Context::new(
            Data {
                input: capture,
                output: render,
                inputs: input_buffers,
                outputs: output_buffers,
                frames: self.info.buffer_frames as usize,
                discard_first_input: true,
            },
            options.sample_rate,
            self.event.0,
        )));
        debug_assert_ne!(
            self.info.format,
            moiren_engine::audio_config::SampleFormat::Native
        );
        self.zero_outputs();
        self.refresh_latencies();
        Ok(())
    }

    fn verify_rate(&self) -> Result<(), BackendError> {
        let driver = self.driver();
        let mut actual = 0.0;
        let code = unsafe { (driver.vtable().get_sample_rate)(driver.raw(), &mut actual) };
        driver.check("getSampleRate", code)?;
        if actual != f64::from(self.info.sample_rate) {
            return Err(BackendError::Unsupported(format!(
                "ASIO driver negotiated {actual} Hz instead of requested {} Hz",
                self.info.sample_rate
            )));
        }
        Ok(())
    }

    fn query_encodings(&mut self) -> Result<(), BackendError> {
        let mut format = None;
        let mut inputs = [None; 2];
        let mut outputs = [None; 2];
        for buffer in &self.buffers {
            let mut info = abi::ChannelInfo {
                channel: buffer.channel_num,
                is_input: buffer.is_input,
                ..Default::default()
            };
            let driver = self.driver();
            let code = unsafe { (driver.vtable().get_channel_info)(driver.raw(), &mut info) };
            driver.check("getChannelInfo", code)?;
            let encoding = Encoding::from_asio(info.sample_type)?;
            let actual = encoding.format();
            if format.is_some_and(|previous| previous != actual) {
                return Err(BackendError::Unsupported(
                    "ASIO enabled channels have mixed effective sample formats; \
                     select matching input/output types in the driver control panel"
                        .into(),
                ));
            }
            format = Some(actual);
            if buffer.is_input != 0 {
                inputs[buffer.channel_num as usize] = Some(encoding);
            } else {
                outputs[buffer.channel_num as usize] = Some(encoding);
            }
        }
        self.inputs = inputs;
        self.outputs = outputs;
        self.info.format = format.expect("at least one enabled channel");
        Ok(())
    }

    fn zero_outputs(&self) {
        for buffer in &self.buffers {
            if buffer.is_input == 0 {
                let encoding = self.outputs[buffer.channel_num as usize].expect("output encoding");
                for pointer in buffer.buffers {
                    // SAFETY: only called detached, after successful buffer
                    // validation, while the driver still owns every allocation.
                    unsafe {
                        ptr::write_bytes(
                            pointer.cast::<u8>(),
                            0,
                            self.info.buffer_frames as usize * encoding.bytes,
                        );
                    }
                }
            }
        }
    }

    fn refresh_latencies(&mut self) {
        let driver = self.driver();
        let mut input = 0;
        let mut output = 0;
        let code =
            unsafe { (driver.vtable().get_latencies)(driver.raw(), &mut input, &mut output) };
        self.latencies = if code == abi::OK && input >= 0 && output >= 0 {
            Some((input as u32, output as u32))
        } else {
            None
        };
    }

    /// Driver-reported input/output latencies in hardware frames. This excludes
    /// the application's bounded ring backlog and engine processing latency.
    pub fn latencies(&self) -> Option<(u32, u32)> {
        self.latencies
    }

    fn check(&self) -> Result<(), BackendError> {
        if let Some(context) = &self.context {
            context.check()?;
        }
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), BackendError> {
        self.lease.detach();
        let Some(driver) = self.driver.take() else {
            return Ok(());
        };
        let mut result = Ok(());
        if self.started {
            let code = unsafe { (driver.vtable().stop)(driver.raw()) };
            result = driver.check("stop", code);
            self.started = false;
        }
        if self.buffers_attempted {
            let code = unsafe { (driver.vtable().dispose_buffers)(driver.raw()) };
            if let Err(error) = driver.check("disposeBuffers", code) {
                result = result.and(Err(error));
            }
            self.buffers_attempted = false;
        }
        // Even on driver failure, final Release precedes freeing descriptors,
        // rings, event or context. Late detached callbacks touch no freed state.
        drop(driver);
        self.context = None;
        result
    }

    /// Explicit final teardown when the caller needs disposeBuffers errors.
    /// Dropping the stream performs the same ordered cleanup.
    pub fn close(mut self) -> Result<(), BackendError> {
        self.shutdown()
    }
}

impl NativeStream for AsioStream {
    fn info(&self) -> NativeStreamInfo {
        self.info
    }

    fn stats(&self) -> NativeStreamStats {
        self.context
            .as_ref()
            .map_or_else(NativeStreamStats::default, |context| context.stats())
    }

    fn driver_latency(&self) -> Option<NativeLatency> {
        self.latencies
            .map(|(input_frames, output_frames)| NativeLatency {
                input_frames,
                output_frames,
            })
    }

    fn recovery_request(&self) -> Option<NativeRecoveryRequest> {
        if self.terminal {
            None
        } else {
            self.context
                .as_ref()
                .and_then(|context| context.recovery_request())
        }
    }

    fn release(&mut self) -> Result<(), BackendError> {
        self.shutdown()
    }

    fn start(&mut self) -> Result<(), BackendError> {
        if self.started {
            return self.check();
        }
        self.verify_rate()?;
        self.zero_outputs();
        unsafe { ResetEvent(self.event.0) }
            .map_err(|error| BackendError::Failed(format!("ASIO reset wake event: {error}")))?;
        let context = self.context.as_mut().expect("prepared ASIO context");
        context.prepare();
        // SAFETY: Box pins the context's address through detach and reader
        // retirement. start may synchronously invoke callbacks before returning.
        unsafe { self.lease.publish(context) };
        self.started = true; // A failed start must still get stop during cleanup.
        log::debug!(target: "moiren::audio::asio", "stream stage=start enter");
        let driver = self.driver();
        let code = unsafe { (driver.vtable().start)(driver.raw()) };
        log::debug!(target: "moiren::audio::asio", "stream stage=start exit code={code}");
        if let Err(error) = driver.check("start", code) {
            self.terminal = true;
            self.lease.detach();
            let _ = self.stop();
            return Err(error);
        }
        if let Err(error) = self.check() {
            let _ = self.stop();
            return Err(error);
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        self.lease.detach();
        if !self.started {
            return Ok(());
        }
        let driver = self.driver();
        log::debug!(target: "moiren::audio::asio", "stream stage=stop enter");
        let code = unsafe { (driver.vtable().stop)(driver.raw()) };
        log::debug!(target: "moiren::audio::asio", "stream stage=stop exit code={code}");
        if let Err(error) = driver.check("stop", code) {
            self.terminal = true;
            return Err(error);
        }
        self.started = false;
        if let Some(capture) = self.capture.as_mut() {
            while capture.pop().is_ok() {}
        }
        if let Some(context) = self.context.as_mut() {
            context.discard_output();
        }
        Ok(())
    }

    fn wait(&mut self, timeout: Duration) -> Result<bool, BackendError> {
        self.check()?;
        if !self.started {
            return Err(BackendError::Failed(
                "ASIO stream has not been started".into(),
            ));
        }
        // IASIO is apartment-bound. Service STA work even when capture or the
        // audio event is always ready. Share one budget across both dispatch
        // points; third-party window procedures can still take unbounded time.
        // This is owner-side work, never callback or Engine::render work.
        let mut message_budget = 32;
        dispatch_owner_messages(&mut message_budget);
        self.check()?;
        if self
            .context
            .as_ref()
            .is_some_and(|context| context.latency_changed.swap(false, Ordering::AcqRel))
        {
            self.refresh_latencies();
        }
        if self
            .capture
            .as_ref()
            .is_some_and(|queue| queue.slots() != 0)
        {
            return Ok(true);
        }
        // Round up sub-millisecond waits; never accidentally map a finite
        // duration to INFINITE (u32::MAX).
        let milliseconds = if message_budget < 32 {
            0 // Return control promptly after servicing a message batch.
        } else {
            timeout
                .as_nanos()
                .div_ceil(1_000_000)
                .min(u128::from(u32::MAX - 1)) as u32
        };
        let result = unsafe {
            MsgWaitForMultipleObjectsEx(
                Some(&[self.event.0]),
                milliseconds,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            )
        };
        // Preserve a failed wait's last-error before dispatch calls can change
        // it. New messages also get service when the audio handle wins the wait.
        let ready = match result {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            result if result.0 == WAIT_OBJECT_0.0 + 1 => Ok(true),
            _ => Err(BackendError::Failed(format!(
                "ASIO event wait failed: {}",
                windows::core::Error::from_thread()
            ))),
        };
        dispatch_owner_messages(&mut message_budget);
        self.check()?;
        ready.map(|ready| ready || message_budget < 32)
    }

    fn read(&mut self, output: &mut [[f64; 2]]) -> Result<usize, BackendError> {
        self.check()?;
        let Some(capture) = self.capture.as_mut() else {
            return Ok(0);
        };
        let mut count = 0;
        for destination in output.iter_mut() {
            let Ok(frame) = capture.pop() else {
                break;
            };
            let left = self.inputs[0]
                .expect("capture encoding")
                .decode(&frame.samples[0]);
            let right = self.inputs[1].map_or(left, |encoding| encoding.decode(&frame.samples[1]));
            *destination = [left, right]; // Mono input is duplicated to stereo.
            count += 1;
        }
        Ok(count)
    }

    fn write(&mut self, input: &[[f64; 2]]) -> Result<usize, BackendError> {
        self.check()?;
        let Some(render) = self.render.as_mut() else {
            return Ok(0);
        };
        let slots = render.slots().min(input.len());
        let mut count = 0;
        for sample in &input[..slots] {
            let mut frame = Frame::default();
            let left = self.outputs[0].expect("render encoding");
            if let Some(right) = self.outputs[1] {
                frame.samples = [left.encode(sample[0]), right.encode(sample[1])];
            } else {
                let finite = sample.map(|value| if value.is_finite() { value } else { 0.0 });
                frame.samples[0] = left.encode(finite[0] * 0.5 + finite[1] * 0.5);
            }
            if render.push(frame).is_err() {
                break;
            }
            count += 1;
        }
        Ok(count)
    }
}

impl Drop for AsioStream {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
