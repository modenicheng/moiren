//! The diagnostic probe uses the same activation/lifetime implementation as
//! formal capture, while retaining its Windows HRESULT result contract.
pub(super) fn activate(
    pid: u32,
) -> windows::core::Result<windows::Win32::Media::Audio::IAudioClient> {
    crate::process_loopback::activation::activate(pid, || Ok(())).map_err(|error| {
        let code = match error {
            crate::capture::CaptureError::Api { hresult, .. } => windows::core::HRESULT(hresult),
            _ => windows::core::HRESULT::from_win32(1460),
        };
        windows::core::Error::from_hresult(code)
    })
}
