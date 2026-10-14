use slint::winit_030::{
    WinitWindowAccessor,
    winit::raw_window_handle::{HasWindowHandle, RawWindowHandle},
};
use std::cell::Cell;
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::{GetModuleHandleW, GetProcAddress},
        UI::{
            Input::{
                GetCurrentInputMessageSource, IMDT_TOUCHPAD, INPUT_MESSAGE_SOURCE,
                Pointer::GetPointerType,
            },
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{
                DefWindowProcW, POINTER_INPUT_TYPE, PT_TOUCHPAD, WM_MOUSEHWHEEL, WM_MOUSEWHEEL,
                WM_NCDESTROY, WM_POINTERDOWN, WM_POINTERUP, WM_POINTERUPDATE,
            },
        },
    },
    core::{BOOL, PCSTR, w},
};

type RegisterTouchpadThread = unsafe extern "system" fn(BOOL) -> BOOL;

const SUBCLASS_ID: usize = 0x6d6f6972;
const WHEEL_CONTROL_FLAG: usize = 0x0008;

thread_local! {
    static WHEEL_CONTROL: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn wheel_control() -> bool {
    WHEEL_CONTROL.get()
}

struct WheelModifierGuard(bool);

impl WheelModifierGuard {
    fn new(control: bool) -> Self {
        Self(WHEEL_CONTROL.replace(control))
    }
}

impl Drop for WheelModifierGuard {
    fn drop(&mut self) {
        WHEEL_CONTROL.set(self.0);
    }
}

pub(super) struct TouchpadRegistration {
    register: Option<RegisterTouchpadThread>,
    hwnd: Option<HWND>,
}

impl TouchpadRegistration {
    pub(super) fn new() -> Self {
        // The API is ordinal-only on supported Windows 11 builds. Resolve it
        // dynamically so older Windows can still create the application window.
        let register = unsafe {
            GetModuleHandleW(w!("user32.dll")).ok().and_then(|module| {
                GetProcAddress(module, PCSTR(2688usize as *const u8)).map(|address| {
                    std::mem::transmute::<
                        unsafe extern "system" fn() -> isize,
                        RegisterTouchpadThread,
                    >(address)
                })
            })
        };
        // This registration belongs to the UI thread. Unhandled WM_POINTER
        // messages retain Windows gesture recognition and its Ctrl+wheel pinch.
        Self {
            register: register.filter(|register| unsafe { register(BOOL(1)).as_bool() }),
            hwnd: None,
        }
    }

    pub(super) fn attach(&mut self, window: &slint::Window) {
        if self.register.is_none() {
            return;
        }
        let hwnd = window
            .with_winit_window(|window| {
                let RawWindowHandle::Win32(handle) = window.window_handle().ok()?.as_raw() else {
                    return None;
                };
                Some(HWND(handle.hwnd.get() as *mut _))
            })
            .flatten();
        if hwnd == self.hwnd {
            return;
        }
        if let Some(previous) = self.hwnd.take() {
            // No callback data is retained by this subclass, even if the old
            // native window was already destroyed during a hide/show cycle.
            let _ = unsafe { RemoveWindowSubclass(previous, Some(touchpad_subclass), SUBCLASS_ID) };
        }
        if let Some(hwnd) = hwnd {
            // Slint's Winit backend otherwise consumes unknown WM_POINTER
            // types, preventing Windows from recognizing touchpad gestures.
            if unsafe { SetWindowSubclass(hwnd, Some(touchpad_subclass), SUBCLASS_ID, 0) }.as_bool()
            {
                self.hwnd = Some(hwnd);
            }
        }
    }

    pub(super) fn is_touchpad(&self) -> bool {
        let mut source = INPUT_MESSAGE_SOURCE::default();
        // Winit calls this filter while dispatching the current mouse message;
        // the initialized output stays valid for the entire Win32 call.
        self.register.is_some()
            && unsafe { GetCurrentInputMessageSource(&mut source) }.is_ok()
            && source.deviceType == IMDT_TOUCHPAD
    }
}

impl Drop for TouchpadRegistration {
    fn drop(&mut self) {
        if let Some(hwnd) = self.hwnd {
            let _ = unsafe { RemoveWindowSubclass(hwnd, Some(touchpad_subclass), SUBCLASS_ID) };
        }
        if let Some(register) = self.register {
            // The window event filter and its registration are dropped on the
            // same UI thread, balancing Windows' thread registration count.
            let _ = unsafe { register(BOOL(0)) };
        }
    }
}

unsafe extern "system" fn touchpad_subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if matches!(message, WM_MOUSEWHEEL | WM_MOUSEHWHEEL) {
        // Windows pinch supplies MK_CONTROL in the wheel message without
        // necessarily changing keyboard state. Preserve it across Winit's
        // callback, and restore it even for nested message dispatch.
        let _modifiers = WheelModifierGuard::new(wparam.0 & WHEEL_CONTROL_FLAG != 0);
        return unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    }
    if matches!(message, WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP) {
        let mut pointer_type = POINTER_INPUT_TYPE::default();
        // The low word contains the pointer ID; the output lives through the
        // Win32 query. Default processing preserves system scroll/pinch settings.
        if unsafe { GetPointerType((wparam.0 & 0xffff) as u32, &mut pointer_type) }.is_ok()
            && pointer_type == PT_TOUCHPAD
        {
            return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
        }
    }
    if message == WM_NCDESTROY {
        let _ = unsafe { RemoveWindowSubclass(hwnd, Some(touchpad_subclass), SUBCLASS_ID) };
    }
    // All other input and window lifetime messages retain Winit's processing.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinch_modifier_is_scoped_to_the_current_wheel_message() {
        assert!(!wheel_control());
        {
            let _pinch = WheelModifierGuard::new(true);
            assert!(wheel_control());
            {
                let _scroll = WheelModifierGuard::new(false);
                assert!(!wheel_control());
            }
            assert!(wheel_control());
        }
        assert!(!wheel_control());
    }
}
