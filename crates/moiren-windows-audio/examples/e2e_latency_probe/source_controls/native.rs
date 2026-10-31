//! Existing-client getters only; interfaces remain on the source owner thread.
use super::{Preparation, ReadControls, Reading, SourceControls};
use moiren_windows_audio::latency::QpcClock;
use windows::{
    Win32::{
        Foundation::FILETIME,
        Media::Audio::{
            AudioSessionState, Endpoints::IAudioEndpointVolume, IAudioClient, IAudioSessionControl,
            IMMDevice, ISimpleAudioVolume,
        },
        System::{
            Com::CLSCTX_ALL,
            Threading::{GetCurrentProcess, GetProcessTimes},
        },
    },
    core::{BOOL, Interface},
};

pub(in super::super) struct NativeControls {
    session: Option<IAudioSessionControl>,
    volume: Option<ISimpleAudioVolume>,
    endpoint: Option<IAudioEndpointVolume>,
    pub evidence: SourceControls,
}

// SAFETY: only successful COM out-parameters transfer one owned interface ref.
// Retain the original HRESULT separately, including success with a null output.
unsafe fn acquired<T: Interface>(hr: i32, raw: *mut std::ffi::c_void) -> Option<T> {
    (hr >= 0 && !raw.is_null()).then(|| unsafe { T::from_raw(raw) })
}

impl NativeControls {
    /// Opt-in preparation after Initialize, outside the event loop and any lease.
    /// These synchronous COM calls have no cancellable hard deadline.
    pub(in super::super) fn prepare(
        client: &IAudioClient,
        device: &IMMDevice,
        id: &str,
        clock: Option<&QpcClock>,
        stopped: impl Fn() -> bool,
    ) -> Self {
        let (session, session_hr) = if stopped() {
            (None, None)
        } else {
            let mut raw = std::ptr::null_mut();
            // SAFETY: live initialized owner-thread client, supported IID/output.
            let hr = unsafe {
                (Interface::vtable(client).GetService)(
                    Interface::as_raw(client),
                    &IAudioSessionControl::IID,
                    &mut raw,
                )
            }
            .0;
            (unsafe { acquired(hr, raw) }, Some(hr))
        };
        let (volume, volume_hr) = if stopped() {
            (None, None)
        } else {
            let mut raw = std::ptr::null_mut();
            let hr = unsafe {
                (Interface::vtable(client).GetService)(
                    Interface::as_raw(client),
                    &ISimpleAudioVolume::IID,
                    &mut raw,
                )
            }
            .0;
            (unsafe { acquired(hr, raw) }, Some(hr))
        };
        let (endpoint, endpoint_hr) = if stopped() {
            (None, None)
        } else {
            let mut raw = std::ptr::null_mut();
            // Same exact device already identity/flow-validated by the owner.
            let hr = unsafe {
                (Interface::vtable(device).Activate)(
                    Interface::as_raw(device),
                    &IAudioEndpointVolume::IID,
                    CLSCTX_ALL,
                    std::ptr::null(),
                    &mut raw,
                )
            }
            .0;
            (unsafe { acquired(hr, raw) }, Some(hr))
        };
        let creation = if stopped() {
            Reading::default()
        } else {
            let (mut created, mut exited, mut kernel, mut user) = (
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
            );
            match unsafe {
                GetProcessTimes(
                    GetCurrentProcess(),
                    &mut created,
                    &mut exited,
                    &mut kernel,
                    &mut user,
                )
            } {
                Ok(()) => Reading::observed(
                    0,
                    (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
                ),
                Err(error) => Reading::observed(error.code().0, 0),
            }
        };
        Self {
            session,
            volume,
            endpoint,
            evidence: SourceControls {
                qualified: false,
                mmdevice_id: id.into(),
                process_id: std::process::id(),
                process_creation_100ns: creation,
                qpc_frequency_hz: clock.and_then(QpcClock::frequency_hz),
                preparation: Preparation {
                    session_control_hresult: session_hr,
                    session_volume_hresult: volume_hr,
                    endpoint_volume_hresult: endpoint_hr,
                },
                snapshot: None,
            },
        }
    }
}

impl ReadControls for NativeControls {
    fn session_state(&self) -> Reading<i32> {
        let Some(session) = self.session.as_ref() else {
            return Reading::default();
        };
        let mut value = AudioSessionState(0);
        let hr = unsafe {
            (Interface::vtable(session).GetState)(Interface::as_raw(session), &mut value)
        };
        Reading::observed(hr.0, value.0)
    }
    fn session_volume(&self) -> Reading<f32> {
        let Some(volume) = self.volume.as_ref() else {
            return Reading::default();
        };
        let mut value = 0.;
        let hr = unsafe {
            (Interface::vtable(volume).GetMasterVolume)(Interface::as_raw(volume), &mut value)
        };
        Reading::observed(hr.0, value)
    }
    fn session_mute(&self) -> Reading<bool> {
        let Some(volume) = self.volume.as_ref() else {
            return Reading::default();
        };
        let mut value = BOOL::default();
        let hr =
            unsafe { (Interface::vtable(volume).GetMute)(Interface::as_raw(volume), &mut value) };
        Reading::observed(hr.0, value.as_bool())
    }
    fn endpoint_volume(&self) -> Reading<f32> {
        let Some(endpoint) = self.endpoint.as_ref() else {
            return Reading::default();
        };
        let mut value = 0.;
        let hr = unsafe {
            (Interface::vtable(endpoint).GetMasterVolumeLevelScalar)(
                Interface::as_raw(endpoint),
                &mut value,
            )
        };
        Reading::observed(hr.0, value)
    }
    fn endpoint_mute(&self) -> Reading<bool> {
        let Some(endpoint) = self.endpoint.as_ref() else {
            return Reading::default();
        };
        let mut value = BOOL::default();
        let hr = unsafe {
            (Interface::vtable(endpoint).GetMute)(Interface::as_raw(endpoint), &mut value)
        };
        Reading::observed(hr.0, value.as_bool())
    }
}
