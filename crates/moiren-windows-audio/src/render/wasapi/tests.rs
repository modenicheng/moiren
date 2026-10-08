use super::*;
use super::{endpoint::project_render_endpoints, format::validate_format, packet::submit};
use crate::{catalog, owner::OwnedHandle};
use std::{
    cell::{RefCell, UnsafeCell},
    rc::Rc,
};
use windows::Win32::Media::Audio::*;
use windows::core::{Result as WinResult, implement};

#[test]
fn usable_render_choices_survive_missing_capture_and_local_errors() {
    let mut mix_format = catalog::FormatSnapshot::from_base(WAVEFORMATEX {
        wFormatTag: 3,
        nChannels: 2,
        nSamplesPerSec: 48000,
        nAvgBytesPerSec: 384000,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    });
    mix_format.channel_mask = Some(3);
    let endpoints = project_render_endpoints(catalog::CatalogSnapshot {
        defaults: vec![catalog::DefaultEndpoint {
            flow: "capture",
            role: "console",
            id: None,
        }],
        endpoints: vec![catalog::EndpointSnapshot {
            id: "speakers".into(),
            name: Some("Usable speakers".into()),
            flow: "render",
            mix_format: Some(mix_format),
            default_period_100ns: None,
            minimum_period_100ns: None,
            volume_scalar: None,
            muted: None,
            sessions: Vec::new(),
            errors: vec![catalog::ApiFailure {
                stage: "session enumeration".into(),
                hresult: "0x80070005".into(),
            }],
        }],
        errors: vec![catalog::ApiFailure {
            stage: "missing capture default".into(),
            hresult: "0x80070490".into(),
        }],
    });
    assert_eq!(endpoints[0].endpoint_id, "speakers");
    assert_eq!(endpoints[0].format_supported, Some(true));
    assert!(endpoints[0].errors[0].contains("0x80070005"));
    assert_eq!(endpoints[1].endpoint_id, "");
    assert!(endpoints[1].errors[0].contains("0x80070490"));
}

#[test]
fn stop_wakes_without_any_audio_event_and_takes_priority() {
    let stop = stop_event().unwrap();
    let audio = OwnedHandle::event().unwrap();
    assert_eq!(wait(handle(&stop), audio.0, 0).unwrap(), Wake::Timeout);
    unsafe {
        SetEvent(handle(&stop)).unwrap();
    }
    assert_eq!(wait(handle(&stop), audio.0, 100).unwrap(), Wake::Stop);
    unsafe {
        SetEvent(audio.0).unwrap();
    }
    assert_eq!(wait(handle(&stop), audio.0, 100).unwrap(), Wake::Stop);
}
#[test]
fn native_format_validation_rejects_conversion_and_bad_layouts() {
    let base = WAVEFORMATEX {
        wFormatTag: 3,
        nChannels: 2,
        nSamplesPerSec: 48000,
        nAvgBytesPerSec: 384000,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    assert!(validate_format(base, true, None, None).is_ok());
    assert!(validate_format(base, true, Some(32), Some(3)).is_ok());
    for bad in [
        WAVEFORMATEX {
            nSamplesPerSec: 44100,
            ..base
        },
        WAVEFORMATEX {
            nChannels: 1,
            ..base
        },
        WAVEFORMATEX {
            nBlockAlign: 4,
            ..base
        },
        WAVEFORMATEX {
            wBitsPerSample: 24,
            ..base
        },
        WAVEFORMATEX {
            nAvgBytesPerSec: 192000,
            ..base
        },
    ] {
        assert_eq!(
            validate_format(bad, true, None, None),
            Err(RenderError::UnsupportedFormat)
        );
    }
    assert_eq!(
        validate_format(base, false, None, None),
        Err(RenderError::UnsupportedFormat)
    );
    assert_eq!(
        validate_format(base, true, Some(24), None),
        Err(RenderError::UnsupportedFormat)
    );
    assert_eq!(
        validate_format(base, true, Some(32), Some(12)),
        Err(RenderError::UnsupportedFormat)
    );
}
#[implement(IAudioRenderClient)]
struct FakeRender {
    data: Rc<UnsafeCell<[f32; 4]>>,
    calls: Rc<RefCell<Vec<(u32, u32)>>>,
    null_buffer: bool,
}
impl IAudioRenderClient_Impl for FakeRender_Impl {
    fn GetBuffer(&self, frames: u32) -> WinResult<*mut u8> {
        assert_eq!(frames, 2);
        self.calls.borrow_mut().push((frames, u32::MAX));
        Ok(if self.null_buffer {
            std::ptr::null_mut()
        } else {
            self.data.get().cast()
        })
    }
    fn ReleaseBuffer(&self, frames: u32, flags: u32) -> WinResult<()> {
        self.calls.borrow_mut().push((frames, flags));
        Ok(())
    }
}
#[test]
fn render_leases_copy_full_pcm_or_cancel_without_submitting_stale_data() {
    for null_buffer in [false, true] {
        let data = Rc::new(UnsafeCell::new([99.0; 4]));
        let calls = Rc::new(RefCell::new(Vec::new()));
        let client: IAudioRenderClient = FakeRender {
            data: Rc::clone(&data),
            calls: Rc::clone(&calls),
            null_buffer,
        }
        .into();
        assert!(submit(&client, &[]).is_ok());
        assert!(calls.borrow().is_empty());
        let result = submit(&client, &[0.1, -0.1, 0.2, -0.2]);
        assert_eq!(result.is_err(), null_buffer);
        assert_eq!(
            *calls.borrow(),
            [(2, u32::MAX), (if null_buffer { 0 } else { 2 }, 0)]
        );
        // SAFETY: Single-threaded test reads after the buffer lease ended.
        assert_eq!(
            unsafe { *data.get() },
            if null_buffer {
                [99.0; 4]
            } else {
                [0.1, -0.1, 0.2, -0.2]
            }
        );
    }
}
