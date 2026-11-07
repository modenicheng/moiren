//! VST 2.4 C ABI declarations, transcribed from the published interface
//! (aeffectx.h; cross-checked against the rust-vst reference
//! <https://github.com/rust-dsp/rust-vst>, src/api.rs).
//!
//! The official VST2 SDK is no longer distributed by Steinberg; these
//! declarations are the public, widely documented binary interface that
//! existing plugins were built against. No SDK code is included.

use std::os::raw::c_void;

pub const VST_MAGIC: i32 = 0x5673_7450; // 'VstP'

// Plugin flags (AEffect.flags).
pub const EFF_FLAGS_HAS_EDITOR: i32 = 1;
pub const EFF_FLAGS_CAN_REPLACING: i32 = 1 << 4;
pub const EFF_FLAGS_PROGRAM_CHUNKS: i32 = 1 << 5;
pub const EFF_FLAGS_IS_SYNTH: i32 = 1 << 8;
pub const EFF_FLAGS_CAN_DOUBLE_REPLACING: i32 = 1 << 12;

// Host -> plugin opcodes (AEffect.dispatcher).
pub const EFF_OPEN: i32 = 0;
pub const EFF_CLOSE: i32 = 1;
pub const EFF_GET_PARAM_NAME: i32 = 8;
pub const EFF_SET_SAMPLE_RATE: i32 = 10;
pub const EFF_SET_BLOCK_SIZE: i32 = 11;
pub const EFF_MAINS_CHANGED: i32 = 12;
pub const EFF_GET_CHUNK: i32 = 23;
pub const EFF_SET_CHUNK: i32 = 24;
pub const EFF_CAN_BE_AUTOMATED: i32 = 26;
pub const EFF_GET_EFFECT_NAME: i32 = 45;
pub const EFF_GET_VENDOR_STRING: i32 = 47;
pub const EFF_GET_PRODUCT_STRING: i32 = 48;
pub const EFF_GET_VENDOR_VERSION: i32 = 49;
pub const EFF_START_PROCESS: i32 = 71;
pub const EFF_STOP_PROCESS: i32 = 72;

// Plugin -> host opcodes (audioMaster callback).
pub const AUDIO_MASTER_AUTOMATE: i32 = 0;
pub const AUDIO_MASTER_VERSION: i32 = 1;
pub const AUDIO_MASTER_CURRENT_ID: i32 = 2;
pub const AUDIO_MASTER_IDLE: i32 = 3;
pub const AUDIO_MASTER_GET_SAMPLE_RATE: i32 = 16;
pub const AUDIO_MASTER_GET_BLOCK_SIZE: i32 = 17;
pub const AUDIO_MASTER_GET_CURRENT_PROCESS_LEVEL: i32 = 23;
pub const AUDIO_MASTER_CAN_DO: i32 = 37;
pub const AUDIO_MASTER_UPDATE_DISPLAY: i32 = 41;
pub const AUDIO_MASTER_BEGIN_EDIT: i32 = 42;
pub const AUDIO_MASTER_END_EDIT: i32 = 43;

pub const HOST_VST_VERSION: isize = 2400;
pub const PROCESS_LEVEL_REALTIME: isize = 2;

/// Host callback passed to the plugin entry point.
pub type HostCallbackProc = unsafe extern "C" fn(
    effect: *mut AEffect,
    opcode: i32,
    index: i32,
    value: isize,
    ptr: *mut c_void,
    opt: f32,
) -> isize;

/// Plugin dispatcher (host -> plugin control calls).
pub type DispatcherProc = unsafe extern "C" fn(
    effect: *mut AEffect,
    opcode: i32,
    index: i32,
    value: isize,
    ptr: *mut c_void,
    opt: f32,
) -> isize;

pub type ProcessProc = unsafe extern "C" fn(
    effect: *mut AEffect,
    inputs: *const *const f32,
    outputs: *mut *mut f32,
    sample_frames: i32,
);

pub type SetParameterProc = unsafe extern "C" fn(effect: *mut AEffect, index: i32, parameter: f32);

pub type GetParameterProc = unsafe extern "C" fn(effect: *mut AEffect, index: i32) -> f32;

/// `VSTPluginMain` entry point; the legacy name is `main`.
pub type PluginMain = unsafe extern "C" fn(callback: HostCallbackProc) -> *mut AEffect;

/// VST 2.4 `AEffect` structure, field order and types matching aeffectx.h.
#[repr(C)]
pub struct AEffect {
    pub magic: i32,
    pub dispatcher: DispatcherProc,
    /// Deprecated accumulate mode; never called by this host.
    pub process: ProcessProc,
    pub set_parameter: SetParameterProc,
    pub get_parameter: GetParameterProc,
    pub num_programs: i32,
    pub num_params: i32,
    pub num_inputs: i32,
    pub num_outputs: i32,
    pub flags: i32,
    /// Host-reserved; this host stores a pointer to its per-instance
    /// context here so the audioMaster callback can find it.
    pub reserved1: isize,
    pub reserved2: isize,
    pub initial_delay: i32,
    pub real_qualities: i32,
    pub off_qualities: i32,
    pub io_ratio: f32,
    pub object: *mut c_void,
    pub user: *mut c_void,
    pub unique_id: i32,
    pub version: i32,
    /// Mandatory in VST 2.4; `Option` keeps the null check cheap and safe.
    pub process_replacing: Option<ProcessProc>,
    pub process_replacing_f64: Option<
        unsafe extern "C" fn(
            effect: *mut AEffect,
            inputs: *const *const f64,
            outputs: *mut *mut f64,
            sample_frames: i32,
        ),
    >,
    pub future: [u8; 56],
}
