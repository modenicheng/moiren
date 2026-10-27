pub use crate::process_loopback::ProcessIdentity;
use std::{marker::PhantomData, rc::Rc};
use windows::{
    Win32::{
        Foundation::{CloseHandle, E_INVALIDARG, FILETIME, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::Audio::{
            AUDCLNT_BUFFERFLAGS_SILENT, IAudioCaptureClient, IAudioClient, IAudioRenderClient,
        },
        System::{
            Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
            Threading::{
                AvRevertMmThreadCharacteristics, CreateEventW, GetProcessTimes, OpenProcess,
                PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
                QueryFullProcessImageNameW, WaitForMultipleObjects, WaitForSingleObject,
            },
        },
    },
    core::{PWSTR, Result},
};

pub struct Apartment(PhantomData<Rc<()>>);
impl Apartment {
    pub fn new() -> Result<Self> {
        // Both S_OK and S_FALSE require one matching CoUninitialize.
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        }
        Ok(Self(PhantomData))
    }
}

pub(crate) struct Streaming<'a> {
    client: &'a IAudioClient,
    active: bool,
}
impl<'a> Streaming<'a> {
    pub fn start(client: &'a IAudioClient) -> Result<Self> {
        unsafe {
            client.Start()?;
        }
        Ok(Self {
            client,
            active: true,
        })
    }
    pub fn stop(&mut self) -> Result<()> {
        unsafe {
            self.client.Stop()?;
        }
        self.active = false;
        Ok(())
    }
}
impl Drop for Streaming<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = unsafe { self.client.Stop() };
        }
    }
}

/// WASAPI packet release call for one lease. The render client takes
/// AUDCLNT_BUFFERFLAGS_* flags; the capture client returns packets by frame
/// count alone and has no flags parameter, so its implementation ignores
/// `flags`.
pub(crate) trait LeaseClient {
    fn release_packet(&self, frames: u32, flags: u32) -> Result<()>;
}

impl LeaseClient for IAudioCaptureClient {
    fn release_packet(&self, frames: u32, _flags: u32) -> Result<()> {
        unsafe { self.ReleaseBuffer(frames) }
    }
}

impl LeaseClient for IAudioRenderClient {
    fn release_packet(&self, frames: u32, flags: u32) -> Result<()> {
        unsafe { self.ReleaseBuffer(frames, flags) }
    }
}

/// What Drop returns for a lease abandoned without `release`. Each transport
/// states its own fallback semantics; there is deliberately no shared default.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LeaseFallback {
    /// Capture: the whole packet is returned by its frame count. The capture
    /// ReleaseBuffer API carries no flags; the read stays fully accounted.
    CapturePacket,
    /// Professional render: the reserved frames are returned flagged
    /// AUDCLNT_BUFFERFLAGS_SILENT, so a submission aborted by validation plays
    /// silence instead of unwritten buffer memory.
    RenderSilent,
}

/// One RAII release-buffer lease shared by the WASAPI transports. GetBuffer
/// hands a packet out, `release` returns it on the owner thread, and Drop
/// applies the transport's [`LeaseFallback`] if control flow escaped without
/// an explicit release.
pub(crate) struct PacketLease<'a, C: LeaseClient> {
    client: &'a C,
    frames: u32,
    released: bool,
    fallback: LeaseFallback,
}
impl<'a, C: LeaseClient> PacketLease<'a, C> {
    pub fn new(client: &'a C, frames: u32, fallback: LeaseFallback) -> Self {
        Self {
            client,
            frames,
            released: false,
            fallback,
        }
    }
    /// Owner-thread release of the complete packet. Render submits the
    /// written frames with no flags; capture returns the read frame count.
    pub fn release(mut self) -> Result<()> {
        self.released = true;
        self.client.release_packet(self.frames, 0)
    }
}
impl<C: LeaseClient> Drop for PacketLease<'_, C> {
    fn drop(&mut self) {
        if !self.released {
            let (frames, flags) = match self.fallback {
                LeaseFallback::CapturePacket => (self.frames, 0),
                LeaseFallback::RenderSilent => (self.frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32),
            };
            let _ = self.client.release_packet(frames, flags);
        }
    }
}

pub(crate) struct Mmcss(pub HANDLE);
impl Mmcss {
    /// Register the calling thread in the Audio MMCSS class for its
    /// lifetime. Registration failure is non-fatal: the caller keeps its
    /// default scheduling class.
    pub(crate) fn audio() -> Option<Self> {
        Self::audio_detailed().ok()
    }

    /// `audio` with the failure HRESULT preserved, so owners and probes can
    /// record why registration did not happen without re-inlining the call.
    pub(crate) fn audio_detailed() -> windows::core::Result<Self> {
        let mut index = 0;
        unsafe {
            windows::Win32::System::Threading::AvSetMmThreadCharacteristicsW(
                windows::core::w!("Audio"),
                &mut index,
            )
            .map(Self)
        }
    }
}
impl Drop for Mmcss {
    fn drop(&mut self) {
        let _ = unsafe { AvRevertMmThreadCharacteristics(self.0) };
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() }
    }
}

pub struct OwnedHandle(pub HANDLE);
impl OwnedHandle {
    pub fn event() -> Result<Self> {
        unsafe { CreateEventW(None, false, false, None).map(Self) }
    }
}
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WaitOutcome {
    Signaled(usize),
    Timeout,
}

// Win32's WaitForMultipleObjects contract allows at most 64 handles. Keep the
// bound local so this helper does not widen the crate's Windows feature set for
// a constant that is stable across supported Windows versions.
const MAX_WAIT_HANDLES: usize = 64;

/// Wait for a bounded set of Windows handles and normalize the raw wait code.
/// Callers retain ownership and map the signaled index to their own wake
/// semantics; the lowest signaled index wins as defined by Win32.
pub(crate) fn wait_for_handles(
    handles: &[HANDLE],
    timeout_ms: u32,
) -> windows::core::Result<WaitOutcome> {
    if handles.is_empty() || handles.len() > MAX_WAIT_HANDLES {
        return Err(windows::core::Error::from_hresult(E_INVALIDARG));
    }
    match unsafe { WaitForMultipleObjects(handles, false, timeout_ms) } {
        WAIT_TIMEOUT => Ok(WaitOutcome::Timeout),
        event if event.0 < handles.len() as u32 => {
            Ok(WaitOutcome::Signaled((event.0 - WAIT_OBJECT_0.0) as usize))
        }
        _ => Err(windows::core::Error::from_thread()),
    }
}

pub struct TaskMemory<T>(pub *mut T);
impl<T> Drop for TaskMemory<T> {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0.cast())) }
    }
}
pub fn take_string(value: PWSTR) -> Result<String> {
    let _memory = TaskMemory(value.0);
    unsafe { Ok(value.to_string()?) }
}

pub struct Process {
    pub handle: OwnedHandle,
    pub identity: ProcessIdentity,
}

impl Process {
    pub fn open(pid: u32) -> Result<Self> {
        let handle = OwnedHandle(unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )?
        });
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe {
            GetProcessTimes(handle.0, &mut creation, &mut exit, &mut kernel, &mut user)?;
        }
        let mut name = vec![0u16; 32768];
        let mut length = name.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                handle.0,
                PROCESS_NAME_WIN32,
                PWSTR(name.as_mut_ptr()),
                &mut length,
            )?;
        }
        let path = String::from_utf16_lossy(&name[..length as usize]);
        // Persist only basename, not a user's full application path.
        let executable_name = path
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or("unknown")
            .to_owned();
        Ok(Self {
            handle,
            identity: ProcessIdentity {
                pid,
                creation_time_100ns: (u64::from(creation.dwHighDateTime) << 32)
                    | u64::from(creation.dwLowDateTime),
                executable_name,
            },
        })
    }

    pub fn exited(&self) -> Result<bool> {
        match unsafe { WaitForSingleObject(self.handle.0, 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(windows::core::Error::from_thread()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OwnedHandle, WaitOutcome, wait_for_handles};
    use windows::Win32::System::Threading::SetEvent;

    #[test]
    fn wait_for_handles_reports_timeout_and_signaled_index() {
        let first = OwnedHandle::event().unwrap();
        let second = OwnedHandle::event().unwrap();
        assert_eq!(
            wait_for_handles(&[first.0, second.0], 0).unwrap(),
            WaitOutcome::Timeout
        );
        unsafe { SetEvent(second.0).unwrap() };
        assert_eq!(
            wait_for_handles(&[first.0, second.0], 1000).unwrap(),
            WaitOutcome::Signaled(1)
        );
    }
}
