//! Read-only, short-lived MMDevice owners; no exclusive client for name reads.
use super::*;
use crate::rename::{NativeName, stream::NativeFailure};
use windows::Win32::Media::Audio::AUDCLNT_E_DEVICE_INVALIDATED;

pub(crate) fn observe_name(
    stable: StableEndpointId,
    intent: Intent,
    deadline: Instant,
) -> anyhow::Result<NativeName> {
    let _apartment = Apartment::new()?;
    let interfaces = discover_present_interfaces()?;
    ensure!(
        interfaces.len() == 1,
        "streaming rename requires one present VAD root"
    );
    let root = identity::root_instance(&interfaces[0])?;
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    loop {
        ensure!(
            Instant::now() < deadline,
            "streaming name enumeration deadline"
        );
        let (candidates, errors) = identity::candidates(&enumerator)?;
        if let Some(candidate) = validation::select(&candidates, stable, intent, &root)? {
            let device = exact_device(&enumerator, candidate, stable, &root)?;
            let observed = NativeName {
                root_instance: root,
                stable_id: stable.to_string(),
                direction: format!("{intent:?}"),
                mmdevice_id: identity::take_string(unsafe { device.GetId()? })?,
                adapter_instance: candidate.raw_instance.clone(),
                windows_friendly_name: windows_friendly_name(&device)?,
                device_desc: optional_property(&device, &PKEY_Device_DeviceDesc),
                device_interface_friendly_name: optional_property(
                    &device,
                    &PKEY_DeviceInterface_FriendlyName,
                ),
                topology: None,
            };
            ensure!(
                Instant::now() < deadline,
                "streaming name read exceeded deadline"
            );
            // IMMDevice/store/enumerator drop before the apartment and before
            // caller Apply. No IAudioClient was activated by this observation.
            return Ok(observed);
        }
        ensure!(Instant::now() < deadline, "MMDevice absent: {errors:?}");
        thread::sleep(Duration::from_millis(20));
    }
}

pub(crate) fn failure(stage: &'static str, error: &anyhow::Error) -> NativeFailure {
    let hresult = error.chain().find_map(|cause| {
        cause
            .downcast_ref::<windows::core::Error>()
            .map(|error| error.code().0)
    });
    NativeFailure {
        stage,
        hresult,
        message: format!("{error:#}"),
        invalidated: hresult == Some(AUDCLNT_E_DEVICE_INVALIDATED.0),
    }
}

impl Pair {
    pub(crate) fn both_running(&self) -> bool {
        self.render.active && self.capture.active
    }

    pub(crate) fn target_mmdevice_id(&self, intent: Intent) -> String {
        if intent == Intent::Playback {
            &self.identity.playback_mmdevice_id
        } else {
            &self.identity.microphone_mmdevice_id
        }
        .clone()
    }

    /// Attempt both Stops and retain both failures, then the caller drops Pair
    /// even after a partial Start or invalidated old endpoint. No COM escapes.
    pub(crate) fn stop_for_rename(&mut self) -> Vec<NativeFailure> {
        let mut errors = Vec::new();
        for (stage, stream) in [
            ("render_stop", &mut self.render),
            ("capture_stop", &mut self.capture),
        ] {
            if let Err(error) = stream.stop() {
                errors.push(failure(stage, &error));
            }
        }
        errors
    }
}
