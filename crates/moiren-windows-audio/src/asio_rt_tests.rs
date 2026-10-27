//! Compile the real callback into the allocator-instrumented test executable.
use moiren_engine::host::BackendError;

#[path = "asio/abi.rs"]
#[allow(dead_code)]
mod abi;
#[path = "asio/callback.rs"]
#[allow(dead_code)]
mod callback;
#[path = "asio/sample.rs"]
mod sample;

#[test]
fn real_asio_callbacks_and_diagnostic_snapshots_do_not_allocate_or_free() {
    use callback::{Channel, Context, Data, Exclusive};
    use windows::Win32::{Foundation::CloseHandle, System::Threading::CreateEventW};
    let _serial = callback::TEST_SERIAL.lock().unwrap();
    let lease = Exclusive::acquire().unwrap();
    let event = unsafe { CreateEventW(None, false, false, None) }.unwrap();
    let (tx, mut rx) = rtrb::RingBuffer::new(4);
    let input = [[1u8; 4]; 2];
    let mut output = [[0xffu8; 4]; 2];
    let (_render, render) = rtrb::RingBuffer::new(2);
    let context = Context::new(Data {
        input: Some(tx), output: Some(render),
        inputs: [Some(Channel { buffers: [input[0].as_ptr().cast_mut(), input[1].as_ptr().cast_mut()], bytes: 2 }), None],
        outputs: [Some(Channel { buffers: [output[0].as_mut_ptr(), output[1].as_mut_ptr()], bytes: 2 }), None],
        frames: 2, discard_first_input: true,
    }, 48000, event);
    // SAFETY: owner stack storage is live and stationary until detach retires
    // all readers. Only the callback accesses the ring endpoints in Context.
    unsafe { lease.publish(&context); }
    crate::measured(|| {
        for index in 0..8 {
            unsafe { (callback::CALLBACKS.buffer_switch)(index % 2, 1); }
        }
        context.stats();
        unsafe { (callback::CALLBACKS.message)(6, 0, std::ptr::null_mut(), std::ptr::null_mut()); }
    });
    lease.detach();
    assert_eq!(context.stats().capture_dropped_frames, 10);
    assert_eq!(context.stats().render_silent_frames, 16);
    assert_eq!(rx.slots(), 4);
    while rx.pop().is_ok() {}
    drop(context);
    unsafe { CloseHandle(event).unwrap(); }
}
