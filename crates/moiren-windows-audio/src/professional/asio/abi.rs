//! Independently declared wire/COM ABI, checked against Steinberg's official
//! ASIO-SDK_2.3.4_2025-10-15.zip: common/{asio.h,asiosys.h,iasiodrv.h}.
//! Reference download: https://www.steinberg.net/asiosdk
//! No SDK sources are compiled or redistributed by this backend.

#![cfg_attr(not(all(windows, target_pointer_width = "64")), allow(dead_code))]

use std::ffi::c_void;

pub const OK: i32 = 0;

// Windows ASIO uses 32-bit long, even in a 64-bit process, and pack(push,4).
#[repr(C, packed(4))]
#[derive(Debug, Default, Clone, Copy)]
pub struct ChannelInfo {
    pub channel: i32,
    pub is_input: i32,
    pub is_active: i32,
    pub channel_group: i32,
    pub sample_type: i32,
    pub name: [u8; 32],
}

#[repr(C, packed(4))]
#[derive(Clone, Copy)]
pub struct BufferInfo {
    pub is_input: i32,
    pub channel_num: i32,
    pub buffers: [*mut c_void; 2],
}

#[repr(C, packed(4))]
#[derive(Clone, Copy)]
pub struct Callbacks {
    pub buffer_switch: unsafe extern "C" fn(i32, i32),
    pub sample_rate_changed: unsafe extern "C" fn(f64),
    pub message: unsafe extern "C" fn(i32, i32, *mut c_void, *mut f64) -> i32,
    pub buffer_switch_time_info: unsafe extern "C" fn(*mut Time, i32, i32) -> *mut Time,
}

// NATIVE_INT64 is 0 on Windows: these are hi/lo words, not native u64 fields.
#[repr(C, packed(4))]
pub struct Samples {
    pub hi: u32,
    pub lo: u32,
}

#[repr(C, packed(4))]
pub struct TimeInfo {
    pub speed: f64,
    pub system_time: Samples,
    pub sample_position: Samples,
    pub sample_rate: f64,
    pub flags: u32,
    pub reserved: [u8; 12],
}

#[repr(C, packed(4))]
pub struct TimeCode {
    pub speed: f64,
    pub samples: Samples,
    pub flags: u32,
    pub future: [u8; 64],
}

#[repr(C, packed(4))]
pub struct Time {
    pub reserved: [i32; 4],
    pub info: TimeInfo,
    pub code: TimeCode,
}

#[repr(C, packed(4))]
pub struct ClockSource {
    pub index: i32,
    pub associated_channel: i32,
    pub associated_group: i32,
    pub is_current_source: i32,
    pub name: [u8; 32],
}

#[repr(C)]
pub struct Driver {
    pub vtable: *const VTable,
}

// IASIO adds ordinary C++ virtual methods after IUnknown. Windows x64 has a
// unified calling convention; using this layout on Win32 would be incorrect.
#[repr(C)]
pub struct VTable {
    pub query_interface:
        unsafe extern "system" fn(*mut Driver, *const c_void, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut Driver) -> u32,
    pub release: unsafe extern "system" fn(*mut Driver) -> u32,
    pub init: unsafe extern "system" fn(*mut Driver, *mut c_void) -> i32,
    pub get_driver_name: unsafe extern "system" fn(*mut Driver, *mut u8),
    pub get_driver_version: unsafe extern "system" fn(*mut Driver) -> i32,
    pub get_error_message: unsafe extern "system" fn(*mut Driver, *mut u8),
    pub start: unsafe extern "system" fn(*mut Driver) -> i32,
    pub stop: unsafe extern "system" fn(*mut Driver) -> i32,
    pub get_channels: unsafe extern "system" fn(*mut Driver, *mut i32, *mut i32) -> i32,
    pub get_latencies: unsafe extern "system" fn(*mut Driver, *mut i32, *mut i32) -> i32,
    pub get_buffer_size:
        unsafe extern "system" fn(*mut Driver, *mut i32, *mut i32, *mut i32, *mut i32) -> i32,
    pub can_sample_rate: unsafe extern "system" fn(*mut Driver, f64) -> i32,
    pub get_sample_rate: unsafe extern "system" fn(*mut Driver, *mut f64) -> i32,
    pub set_sample_rate: unsafe extern "system" fn(*mut Driver, f64) -> i32,
    pub get_clock_sources:
        unsafe extern "system" fn(*mut Driver, *mut ClockSource, *mut i32) -> i32,
    pub set_clock_source: unsafe extern "system" fn(*mut Driver, i32) -> i32,
    pub get_sample_position:
        unsafe extern "system" fn(*mut Driver, *mut Samples, *mut Samples) -> i32,
    pub get_channel_info: unsafe extern "system" fn(*mut Driver, *mut ChannelInfo) -> i32,
    pub create_buffers:
        unsafe extern "system" fn(*mut Driver, *mut BufferInfo, i32, i32, *mut Callbacks) -> i32,
    pub dispose_buffers: unsafe extern "system" fn(*mut Driver) -> i32,
    pub control_panel: unsafe extern "system" fn(*mut Driver) -> i32,
    pub future: unsafe extern "system" fn(*mut Driver, i32, *mut c_void) -> i32,
    pub output_ready: unsafe extern "system" fn(*mut Driver) -> i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    fn steinberg_windows_wire_layouts_use_pack_four_and_long32() {
        assert_eq!(
            (size_of::<ChannelInfo>(), align_of::<ChannelInfo>()),
            (52, 4)
        );
        assert_eq!(offset_of!(ChannelInfo, sample_type), 16);
        assert_eq!(size_of::<Samples>(), 8);
        assert_eq!((size_of::<TimeInfo>(), align_of::<TimeInfo>()), (48, 4));
        assert_eq!(size_of::<TimeCode>(), 84);
        assert_eq!(size_of::<Time>(), 148);
        assert_eq!(offset_of!(Time, code), 64);
        assert_eq!(size_of::<ClockSource>(), 48);
        assert_eq!(size_of::<BufferInfo>(), 8 + 2 * size_of::<usize>());
        assert_eq!(align_of::<BufferInfo>(), 4);
        assert_eq!(offset_of!(BufferInfo, buffers), 8);
        assert_eq!(size_of::<Callbacks>(), 4 * size_of::<usize>());
        assert_eq!(align_of::<Callbacks>(), 4);
    }

    #[test]
    fn iasio_vtable_retains_iunknown_prefix_and_all_intervening_methods() {
        let slot = size_of::<usize>();
        assert_eq!(size_of::<VTable>(), 24 * slot);
        assert_eq!(offset_of!(VTable, init), 3 * slot);
        assert_eq!(offset_of!(VTable, start), 7 * slot);
        assert_eq!(offset_of!(VTable, get_channels), 9 * slot);
        assert_eq!(offset_of!(VTable, set_sample_rate), 14 * slot);
        assert_eq!(offset_of!(VTable, create_buffers), 19 * slot);
        assert_eq!(offset_of!(VTable, dispose_buffers), 20 * slot);
        assert_eq!(offset_of!(VTable, output_ready), 23 * slot);
    }
}
