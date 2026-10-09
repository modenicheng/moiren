pub use crate::process_loopback::ProcessIdentity;
use std::{marker::PhantomData, rc::Rc};
use windows::{
    Win32::{
        Foundation::{CloseHandle, FILETIME, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::Audio::{IAudioCaptureClient, IAudioClient},
        System::{
            Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
            Threading::{
                AvRevertMmThreadCharacteristics, CreateEventW, GetProcessTimes, OpenProcess,
                PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
                QueryFullProcessImageNameW, WaitForSingleObject,
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

pub(crate) struct PacketLease<'a> {
    pub client: &'a IAudioCaptureClient,
    pub frames: u32,
    pub released: bool,
}
impl PacketLease<'_> {
    pub fn release(mut self) -> Result<()> {
        self.released = true;
        unsafe { self.client.ReleaseBuffer(self.frames) }
    }
}
impl Drop for PacketLease<'_> {
    fn drop(&mut self) {
        if !self.released {
            let _ = unsafe { self.client.ReleaseBuffer(self.frames) };
        }
    }
}

pub(crate) struct Mmcss(pub HANDLE);
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
