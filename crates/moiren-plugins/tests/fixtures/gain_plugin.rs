//! Self-contained gain plugin exporting all three format entry points
//! (`clap_entry`, `VSTPluginMain`, `GetPluginFactory`).
//!
//! Compiled directly with rustc into a cdylib by the integration tests; it
//! is deliberately an independent ABI transcription (no moiren-plugins
//! dependency) so the tests cross-check the host-side mirrors against a
//! second implementation of the same published interfaces.
//!
//! Not part of any cargo target; never compiled by the workspace itself.
#![allow(clippy::missing_safety_doc, unsafe_op_in_unsafe_fn)]

use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// Shared identity and DSP constants
// ---------------------------------------------------------------------------

const CHANNELS: usize = 2;
/// Real-unit gain range for CLAP; VST2/VST3 expose normalized 0..1 mapped
/// over the same span so every format multiplies by the same factor.
const GAIN_MAX: f64 = 4.0;
const DEFAULT_GAIN: f64 = 1.0;
const TEST_LATENCY: u32 = 3;

const DEFAULT_NORMALIZED: f64 = DEFAULT_GAIN / GAIN_MAX;

// ---------------------------------------------------------------------------
// CLAP
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct ClapVersion {
    major: u32,
    minor: u32,
    revision: u32,
}
const CLAP_VERSION_1_2: ClapVersion = ClapVersion {
    major: 1,
    minor: 2,
    revision: 10,
};

#[repr(C)]
pub struct ClapEntry {
    clap_version: ClapVersion,
    init: unsafe extern "C" fn(*const c_char) -> bool,
    deinit: unsafe extern "C" fn(),
    get_factory: unsafe extern "C" fn(*const c_char) -> *const c_void,
}

#[repr(C)]
pub struct ClapPluginDescriptor {
    clap_version: ClapVersion,
    id: *const c_char,
    name: *const c_char,
    vendor: *const c_char,
    url: *const c_char,
    manual_url: *const c_char,
    support_url: *const c_char,
    version: *const c_char,
    description: *const c_char,
    features: *const *const c_char,
}

#[repr(C)]
pub struct ClapPluginFactory {
    get_plugin_count: unsafe extern "C" fn(*const ClapPluginFactory) -> u32,
    get_plugin_descriptor:
        unsafe extern "C" fn(*const ClapPluginFactory, u32) -> *const ClapPluginDescriptor,
    create_plugin: unsafe extern "C" fn(
        *const ClapPluginFactory,
        *const ClapHost,
        *const c_char,
    ) -> *const ClapPlugin,
}

#[repr(C)]
pub struct ClapHost {
    clap_version: ClapVersion,
    host_data: *mut c_void,
    name: *const c_char,
    vendor: *const c_char,
    url: *const c_char,
    version: *const c_char,
    get_extension: unsafe extern "C" fn(*const ClapHost, *const c_char) -> *const c_void,
    request_restart: unsafe extern "C" fn(*const ClapHost),
    request_process: unsafe extern "C" fn(*const ClapHost),
    request_callback: unsafe extern "C" fn(*const ClapHost),
}

#[repr(C)]
pub struct ClapPlugin {
    desc: *const ClapPluginDescriptor,
    plugin_data: *mut c_void,
    init: unsafe extern "C" fn(*const ClapPlugin) -> bool,
    destroy: unsafe extern "C" fn(*const ClapPlugin),
    activate: unsafe extern "C" fn(*const ClapPlugin, f64, u32, u32) -> bool,
    deactivate: unsafe extern "C" fn(*const ClapPlugin),
    start_processing: unsafe extern "C" fn(*const ClapPlugin) -> bool,
    stop_processing: unsafe extern "C" fn(*const ClapPlugin),
    reset: unsafe extern "C" fn(*const ClapPlugin),
    process: unsafe extern "C" fn(*const ClapPlugin, *const ClapProcess) -> i32,
    get_extension: unsafe extern "C" fn(*const ClapPlugin, *const c_char) -> *const c_void,
    on_main_thread: unsafe extern "C" fn(*const ClapPlugin),
}

#[repr(C)]
pub struct ClapAudioBuffer {
    data32: *mut *mut f32,
    data64: *mut *mut f64,
    channel_count: u32,
    latency: u32,
    constant_mask: u64,
}

#[repr(C)]
pub struct ClapProcess {
    steady_time: i64,
    frames_count: u32,
    transport: *const c_void,
    audio_inputs: *const ClapAudioBuffer,
    audio_outputs: *mut ClapAudioBuffer,
    audio_inputs_count: u32,
    audio_outputs_count: u32,
    in_events: *const c_void,
    out_events: *const c_void,
}

#[repr(C)]
pub struct ClapEventHeader {
    size: u32,
    time: u32,
    space_id: u16,
    event_type: u16,
    flags: u32,
}

#[repr(C)]
pub struct ClapEventParamValue {
    header: ClapEventHeader,
    param_id: u32,
    cookie: *mut c_void,
    note_id: i32,
    port_index: i16,
    channel: i16,
    key: i16,
    value: f64,
}

#[repr(C)]
pub struct ClapInputEvents {
    ctx: *mut c_void,
    size: unsafe extern "C" fn(*const ClapInputEvents) -> u32,
    get: unsafe extern "C" fn(*const ClapInputEvents, u32) -> *const ClapEventHeader,
}

#[repr(C)]
pub struct ClapAudioPortInfo {
    id: u32,
    name: [c_char; 256],
    flags: u32,
    channel_count: u32,
    port_type: *const c_char,
    in_place_pair: u32,
}

#[repr(C)]
pub struct ClapAudioPorts {
    count: unsafe extern "C" fn(*const ClapPlugin, bool) -> u32,
    get: unsafe extern "C" fn(*const ClapPlugin, u32, bool, *mut ClapAudioPortInfo) -> bool,
}

#[repr(C)]
pub struct ClapParamInfo {
    id: u32,
    flags: u32,
    cookie: *mut c_void,
    name: [c_char; 256],
    module: [c_char; 1024],
    min_value: f64,
    max_value: f64,
    default_value: f64,
}

#[repr(C)]
pub struct ClapParams {
    count: unsafe extern "C" fn(*const ClapPlugin) -> u32,
    get_info: unsafe extern "C" fn(*const ClapPlugin, u32, *mut ClapParamInfo) -> bool,
    get_value: unsafe extern "C" fn(*const ClapPlugin, u32, *mut f64) -> bool,
    value_to_text: unsafe extern "C" fn(*const ClapPlugin, u32, f64, *mut c_char, u32) -> bool,
    text_to_value: unsafe extern "C" fn(*const ClapPlugin, u32, *const c_char, *mut f64) -> bool,
    flush: unsafe extern "C" fn(*const ClapPlugin, *const ClapInputEvents, *const c_void),
}

#[repr(C)]
pub struct ClapState {
    save: unsafe extern "C" fn(*const ClapPlugin, *const ClapOstream) -> bool,
    load: unsafe extern "C" fn(*const ClapPlugin, *const ClapIstream) -> bool,
}

#[repr(C)]
pub struct ClapIstream {
    ctx: *mut c_void,
    read: unsafe extern "C" fn(*const ClapIstream, *mut c_void, u64) -> i64,
}

#[repr(C)]
pub struct ClapOstream {
    ctx: *mut c_void,
    write: unsafe extern "C" fn(*const ClapOstream, *const c_void, u64) -> i64,
}

#[repr(C)]
pub struct ClapLatency {
    get: unsafe extern "C" fn(*const ClapPlugin) -> u32,
}

static CLAP_INITS: AtomicI32 = AtomicI32::new(0);

unsafe extern "C" fn clap_entry_init(_path: *const c_char) -> bool {
    CLAP_INITS.fetch_add(1, Ordering::SeqCst);
    true
}
unsafe extern "C" fn clap_entry_deinit() {
    CLAP_INITS.fetch_sub(1, Ordering::SeqCst);
}
unsafe extern "C" fn clap_entry_get_factory(factory_id: *const c_char) -> *const c_void {
    if factory_id.is_null() {
        return std::ptr::null();
    }
    let id = unsafe { std::ffi::CStr::from_ptr(factory_id) };
    if id.to_bytes() == b"clap.plugin-factory" {
        &CLAP_FACTORY as *const ClapPluginFactory as *const c_void
    } else {
        std::ptr::null()
    }
}

#[unsafe(no_mangle)]
pub static clap_entry: ClapEntry = ClapEntry {
    clap_version: CLAP_VERSION_1_2,
    init: clap_entry_init,
    deinit: clap_entry_deinit,
    get_factory: clap_entry_get_factory,
};

// Descriptor strings and feature lists are immutable 'static data.
unsafe impl Sync for ClapPluginDescriptor {}

static CLAP_DESCRIPTOR: ClapPluginDescriptor = ClapPluginDescriptor {
    clap_version: CLAP_VERSION_1_2,
    id: c"org.moiren.test.gain".as_ptr(),
    name: c"Moiren Test Gain".as_ptr(),
    vendor: c"Moiren".as_ptr(),
    url: c"".as_ptr(),
    manual_url: c"".as_ptr(),
    support_url: c"".as_ptr(),
    version: c"1.0.0".as_ptr(),
    description: c"gain fixture".as_ptr(),
    features: [c"audio-effect".as_ptr(), std::ptr::null()].as_ptr(),
};

unsafe extern "C" fn factory_count(_factory: *const ClapPluginFactory) -> u32 {
    1
}
unsafe extern "C" fn factory_descriptor(
    _factory: *const ClapPluginFactory,
    index: u32,
) -> *const ClapPluginDescriptor {
    if index == 0 {
        &CLAP_DESCRIPTOR
    } else {
        std::ptr::null()
    }
}
unsafe extern "C" fn factory_create(
    _factory: *const ClapPluginFactory,
    _host: *const ClapHost,
    plugin_id: *const c_char,
) -> *const ClapPlugin {
    let id = unsafe { std::ffi::CStr::from_ptr(plugin_id) };
    if id.to_bytes() != b"org.moiren.test.gain" {
        return std::ptr::null();
    }
    let gain = Box::new(DEFAULT_GAIN);
    let plugin = Box::new(ClapPlugin {
        desc: &CLAP_DESCRIPTOR,
        plugin_data: Box::into_raw(gain).cast(),
        init: clap_plugin_init,
        destroy: clap_plugin_destroy,
        activate: clap_plugin_activate,
        deactivate: clap_plugin_noop,
        start_processing: clap_plugin_start,
        stop_processing: clap_plugin_noop,
        reset: clap_plugin_noop,
        process: clap_plugin_process,
        get_extension: clap_plugin_get_extension,
        on_main_thread: clap_plugin_noop,
    });
    Box::into_raw(plugin)
}

unsafe extern "C" fn clap_plugin_noop(_plugin: *const ClapPlugin) {}
unsafe extern "C" fn clap_plugin_init(_plugin: *const ClapPlugin) -> bool {
    true
}
unsafe extern "C" fn clap_plugin_activate(
    _plugin: *const ClapPlugin,
    _sample_rate: f64,
    _min: u32,
    _max: u32,
) -> bool {
    true
}
unsafe extern "C" fn clap_plugin_start(_plugin: *const ClapPlugin) -> bool {
    true
}

unsafe extern "C" fn clap_plugin_destroy(plugin: *const ClapPlugin) {
    let boxed = unsafe { Box::from_raw(plugin as *mut ClapPlugin) };
    drop(unsafe { Box::from_raw(boxed.plugin_data.cast::<f64>()) });
}

unsafe extern "C" fn clap_plugin_process(
    plugin: *const ClapPlugin,
    process: *const ClapProcess,
) -> i32 {
    let process = unsafe { &*process };
    let mut gain = unsafe { *((*plugin).plugin_data as *const f64) } as f32;
    // Parameter events at frame 0 update the block's effective gain.
    if !process.in_events.is_null() {
        let events = unsafe { &*(process.in_events as *const ClapInputEvents) };
        for index in 0..(events.size)(events) {
            let header = (events.get)(events, index);
            if header.is_null() {
                continue;
            }
            let header = unsafe { &*header };
            if header.event_type == 5 {
                let event =
                    unsafe { &*(std::ptr::from_ref(header) as *const ClapEventParamValue) };
                if event.param_id == 0 {
                    gain = event.value as f32;
                    unsafe {
                        *(*plugin).plugin_data.cast::<f64>() = event.value;
                    }
                }
            }
        }
    }
    if process.audio_inputs_count == 0 || process.audio_outputs_count == 0 {
        return 1;
    }
    let input = unsafe { &*process.audio_inputs };
    let output = unsafe { &mut *process.audio_outputs };
    for channel in 0..(output.channel_count as usize).min(CHANNELS) {
        let source = unsafe { *input.data32.add(channel) };
        let target = unsafe { *output.data32.add(channel) };
        for frame in 0..process.frames_count as usize {
            unsafe { *target.add(frame) = *source.add(frame) * gain };
        }
    }
    1
}

unsafe extern "C" fn clap_plugin_get_extension(
    _plugin: *const ClapPlugin,
    id: *const c_char,
) -> *const c_void {
    let id = unsafe { std::ffi::CStr::from_ptr(id) };
    match id.to_bytes() {
        b"clap.audio-ports" => &CLAP_AUDIO_PORTS as *const ClapAudioPorts as *const c_void,
        b"clap.params" => &CLAP_PARAMS as *const ClapParams as *const c_void,
        b"clap.state" => &CLAP_STATE as *const ClapState as *const c_void,
        b"clap.latency" => &CLAP_LATENCY as *const ClapLatency as *const c_void,
        _ => std::ptr::null(),
    }
}

unsafe extern "C" fn ports_count(_plugin: *const ClapPlugin, _is_input: bool) -> u32 {
    1
}
unsafe extern "C" fn ports_get(
    _plugin: *const ClapPlugin,
    _index: u32,
    is_input: bool,
    info: *mut ClapAudioPortInfo,
) -> bool {
    let info = unsafe { &mut *info };
    info.id = if is_input { 1 } else { 2 };
    info.name = [0; 256];
    info.flags = 1; // CLAP_AUDIO_PORT_IS_MAIN
    info.channel_count = CHANNELS as u32;
    info.port_type = c"stereo".as_ptr();
    info.in_place_pair = 0;
    true
}

static CLAP_AUDIO_PORTS: ClapAudioPorts = ClapAudioPorts {
    count: ports_count,
    get: ports_get,
};

unsafe extern "C" fn params_count(_plugin: *const ClapPlugin) -> u32 {
    1
}
unsafe extern "C" fn params_info(
    _plugin: *const ClapPlugin,
    index: u32,
    info: *mut ClapParamInfo,
) -> bool {
    if index != 0 {
        return false;
    }
    let info = unsafe { &mut *info };
    info.id = 0;
    info.flags = 1 << 5; // CLAP_PARAM_IS_AUTOMATABLE
    info.cookie = std::ptr::null_mut();
    info.name = [0; 256];
    for (slot, byte) in b"Gain\0".iter().enumerate() {
        info.name[slot] = *byte as c_char;
    }
    info.module = [0; 1024];
    info.min_value = 0.0;
    info.max_value = GAIN_MAX;
    info.default_value = DEFAULT_GAIN;
    true
}
unsafe extern "C" fn params_value(plugin: *const ClapPlugin, _id: u32, out: *mut f64) -> bool {
    *unsafe { &mut *out } = unsafe { *((*plugin).plugin_data as *const f64) };
    true
}
unsafe extern "C" fn params_value_to_text(
    _plugin: *const ClapPlugin,
    _id: u32,
    _value: f64,
    _out: *mut c_char,
    _capacity: u32,
) -> bool {
    false
}
unsafe extern "C" fn params_text_to_value(
    _plugin: *const ClapPlugin,
    _id: u32,
    _text: *const c_char,
    _out: *mut f64,
) -> bool {
    false
}
unsafe extern "C" fn params_flush(
    _plugin: *const ClapPlugin,
    _input: *const ClapInputEvents,
    _output: *const c_void,
) {
}

static CLAP_PARAMS: ClapParams = ClapParams {
    count: params_count,
    get_info: params_info,
    get_value: params_value,
    value_to_text: params_value_to_text,
    text_to_value: params_text_to_value,
    flush: params_flush,
};

unsafe extern "C" fn state_save(plugin: *const ClapPlugin, stream: *const ClapOstream) -> bool {
    let gain = unsafe { *((*plugin).plugin_data as *const f64) };
    let bytes = gain.to_le_bytes();
    unsafe {
        ((*stream).write)(stream, bytes.as_ptr() as *const c_void, 8);
    }
    true
}
unsafe extern "C" fn state_load(plugin: *const ClapPlugin, stream: *const ClapIstream) -> bool {
    let mut bytes = [0u8; 8];
    let read = unsafe { ((*stream).read)(stream, bytes.as_mut_ptr().cast(), 8) };
    if read != 8 {
        return false;
    }
    unsafe {
        *(*plugin).plugin_data.cast::<f64>() = f64::from_le_bytes(bytes);
    }
    true
}

static CLAP_STATE: ClapState = ClapState {
    save: state_save,
    load: state_load,
};

unsafe extern "C" fn latency_get(_plugin: *const ClapPlugin) -> u32 {
    TEST_LATENCY
}

static CLAP_LATENCY: ClapLatency = ClapLatency { get: latency_get };

static CLAP_FACTORY: ClapPluginFactory = ClapPluginFactory {
    get_plugin_count: factory_count,
    get_plugin_descriptor: factory_descriptor,
    create_plugin: factory_create,
};

// ---------------------------------------------------------------------------
// VST2
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct AEffect {
    magic: i32,
    dispatcher: unsafe extern "C" fn(
        effect: *mut AEffect,
        opcode: i32,
        index: i32,
        value: isize,
        ptr: *mut c_void,
        opt: f32,
    ) -> isize,
    process: unsafe extern "C" fn(*mut AEffect, *const *const f32, *mut *mut f32, i32),
    set_parameter: unsafe extern "C" fn(*mut AEffect, i32, f32),
    get_parameter: unsafe extern "C" fn(*mut AEffect, i32) -> f32,
    num_programs: i32,
    num_params: i32,
    num_inputs: i32,
    num_outputs: i32,
    flags: i32,
    reserved1: isize,
    reserved2: isize,
    initial_delay: i32,
    real_qualities: i32,
    off_qualities: i32,
    io_ratio: f32,
    object: *mut c_void,
    user: *mut c_void,
    unique_id: i32,
    version: i32,
    process_replacing: Option<
        unsafe extern "C" fn(*mut AEffect, *const *const f32, *mut *mut f32, i32),
    >,
    process_replacing_f64: *const c_void,
    future: [u8; 56],
}

#[allow(dead_code)]
unsafe extern "C" fn vst2_audio_master(
    _effect: *mut AEffect,
    opcode: i32,
    _index: i32,
    _value: isize,
    _ptr: *mut c_void,
    _opt: f32,
) -> isize {
    if opcode == 1 {
        2400 // audioMasterVersion
    } else {
        0
    }
}

unsafe extern "C" fn vst2_process_accum(
    _effect: *mut AEffect,
    _inputs: *const *const f32,
    _outputs: *mut *mut f32,
    _frames: i32,
) {
}
unsafe extern "C" fn vst2_set_parameter(effect: *mut AEffect, index: i32, value: f32) {
    if index == 0 {
        unsafe {
            *(*effect).object.cast::<f64>() = value as f64 * GAIN_MAX;
        }
    }
}
unsafe extern "C" fn vst2_get_parameter(effect: *mut AEffect, index: i32) -> f32 {
    if index == 0 {
        let gain = unsafe { *((*effect).object as *const f64) };
        (gain / GAIN_MAX) as f32
    } else {
        0.0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn VSTPluginMain(
    _callback: unsafe extern "C" fn(
        effect: *mut AEffect,
        opcode: i32,
        index: i32,
        value: isize,
        ptr: *mut c_void,
        opt: f32,
    ) -> isize,
) -> *mut AEffect {
    let state = Box::new(DEFAULT_GAIN);
    let effect = Box::new(AEffect {
        magic: 0x5673_7450, // 'VstP'
        dispatcher: vst2_dispatcher_raw,
        process: vst2_process_accum,
        set_parameter: vst2_set_parameter,
        get_parameter: vst2_get_parameter,
        num_programs: 1,
        num_params: 1,
        num_inputs: CHANNELS as i32,
        num_outputs: CHANNELS as i32,
        flags: (1 << 4) | (1 << 5), // CAN_REPLACING | PROGRAM_CHUNKS
        reserved1: 0,
        reserved2: 0,
        initial_delay: TEST_LATENCY as i32,
        real_qualities: 0,
        off_qualities: 0,
        io_ratio: 0.0,
        object: Box::into_raw(state).cast(),
        user: std::ptr::null_mut(),
        unique_id: 0x4D74_6731, // 'Mtg1'
        version: 0x0001_0000,
        process_replacing: Some(vst2_process_replacing),
        process_replacing_f64: std::ptr::null(),
        future: [0; 56],
    });
    Box::into_raw(effect)
}

static CHUNK_BITS: AtomicU64 = AtomicU64::new(0);

unsafe extern "C" fn vst2_dispatcher_raw(
    effect: *mut AEffect,
    opcode: i32,
    index: i32,
    value: isize,
    ptr: *mut c_void,
    _opt: f32,
) -> isize {
    match opcode {
        0 => 0, // effOpen
        1 => {
            // effClose: free the effect and its state.
            let boxed = unsafe { Box::from_raw(effect) };
            drop(unsafe { Box::from_raw(boxed.object.cast::<f64>()) });
            0
        }
        8 => {
            // effGetParamName
            if index == 0 && !ptr.is_null() {
                unsafe {
                    std::ptr::copy_nonoverlapping(b"Gain\0".as_ptr(), ptr.cast::<u8>(), 5)
                };
            }
            0
        }
        23 => {
            // effGetChunk (program)
            let gain = unsafe { *((*effect).object as *const f64) };
            CHUNK_BITS.store(gain.to_bits(), Ordering::SeqCst);
            unsafe {
                *(ptr as *mut *mut c_void) =
                    std::ptr::from_ref(&CHUNK_BITS) as *mut c_void
            };
            8
        }
        24 => {
            // effSetChunk (program): returns 0 like JUCE and much of the
            // real ecosystem — hosts must not read success from this value.
            if value == 8 && !ptr.is_null() {
                let bytes = ptr as *const u8;
                let gain = unsafe { std::ptr::read_unaligned(bytes.cast::<[u8; 8]>()) };
                let gain = f64::from_le_bytes(gain);
                unsafe { *(*effect).object.cast::<f64>() = gain };
            }
            0
        }
        26 => 1, // effCanBeAutomated
        45 => {
            // effGetEffectName
            unsafe {
                std::ptr::copy_nonoverlapping(
                    b"Moiren Test Gain\0".as_ptr(),
                    ptr.cast::<u8>(),
                    17,
                )
            };
            1
        }
        47 => {
            // effGetVendorString
            unsafe { std::ptr::copy_nonoverlapping(b"Moiren\0".as_ptr(), ptr.cast::<u8>(), 7) };
            1
        }
        49 => 0x0001_0000, // effGetVendorVersion
        _ => 0,
    }
}

unsafe extern "C" fn vst2_process_replacing(
    effect: *mut AEffect,
    inputs: *const *const f32,
    outputs: *mut *mut f32,
    frames: i32,
) {
    let gain = unsafe { *((*effect).object as *const f64) } as f32;
    for channel in 0..CHANNELS {
        let source = unsafe { *inputs.add(channel) };
        let target = unsafe { *outputs.add(channel) };
        for frame in 0..frames.max(0) as usize {
            unsafe { *target.add(frame) = *source.add(frame) * gain };
        }
    }
}

// ---------------------------------------------------------------------------
// VST3
// ---------------------------------------------------------------------------

pub type TChar = u16;
pub type String128 = [TChar; 128];
pub type TUID = [u8; 16];

const fn uid(l1: u32, l2: u32, l3: u32, l4: u32) -> TUID {
    [
        (l1 & 0xff) as u8,
        ((l1 >> 8) & 0xff) as u8,
        ((l1 >> 16) & 0xff) as u8,
        ((l1 >> 24) & 0xff) as u8,
        ((l2 >> 16) & 0xff) as u8,
        ((l2 >> 24) & 0xff) as u8,
        (l2 & 0xff) as u8,
        ((l2 >> 8) & 0xff) as u8,
        ((l3 >> 24) & 0xff) as u8,
        ((l3 >> 16) & 0xff) as u8,
        ((l3 >> 8) & 0xff) as u8,
        (l3 & 0xff) as u8,
        ((l4 >> 24) & 0xff) as u8,
        ((l4 >> 16) & 0xff) as u8,
        ((l4 >> 8) & 0xff) as u8,
        (l4 & 0xff) as u8,
    ]
}

const I_COMPONENT_IID: TUID = uid(0xE831_FF31, 0xF2D5_4301, 0x928E_BBEE, 0x2569_7802);
const I_AUDIO_PROCESSOR_IID: TUID = uid(0x4204_3F99, 0xB7DA_453C, 0xA569_E79D, 0x9AAE_C33D);
const I_EDIT_CONTROLLER_IID: TUID = uid(0xDCD7_BBE3, 0x7742_448D, 0xA874_AACC, 0x979C_759E);
const I_PLUGIN_FACTORY_IID: TUID = uid(0x7A4D_811C, 0x5211_4A1F, 0xAED9_D2EE, 0x0B43_BF9F);
const I_PLUGIN_FACTORY2_IID: TUID = uid(0x0007_B650, 0xF24B_4C0B, 0xA464_EDB9, 0xF00B_2ABB);
const TEST_CID: TUID = uid(0x6B1C_A3D4, 0x51F0_4E2B, 0x8A45_C7D9, 0xE3B2_60F1);

pub type QueryInterfaceFn = unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> i32;
pub type AddRefFn = unsafe extern "system" fn(*mut c_void) -> u32;
pub type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;

#[repr(C)]
pub struct PClassInfo {
    cid: TUID,
    cardinality: i32,
    category: [c_char; 32],
    name: [c_char; 64],
}

#[repr(C)]
pub struct PClassInfo2 {
    cid: TUID,
    cardinality: i32,
    category: [c_char; 32],
    name: [c_char; 64],
    class_flags: u32,
    sub_categories: [c_char; 128],
    vendor: [c_char; 64],
    version: [c_char; 64],
    sdk_version: [c_char; 64],
}

#[repr(C)]
pub struct AudioBusBuffers {
    num_channels: i32,
    silence_flags: u64,
    channel_buffers32: *mut *mut f32,
}

#[repr(C)]
pub struct ProcessData {
    process_mode: i32,
    symbolic_sample_size: i32,
    num_samples: i32,
    num_inputs: i32,
    num_outputs: i32,
    inputs: *mut AudioBusBuffers,
    outputs: *mut AudioBusBuffers,
    input_parameter_changes: *mut c_void,
    output_parameter_changes: *mut c_void,
    input_events: *mut c_void,
    output_events: *mut c_void,
    process_context: *mut c_void,
}

#[repr(C)]
pub struct BusInfo {
    media_type: i32,
    direction: i32,
    channel_count: i32,
    name: String128,
    bus_type: i32,
    flags: u32,
}

#[repr(C)]
pub struct ParameterInfo {
    id: u32,
    title: String128,
    short_title: String128,
    units: String128,
    step_count: i32,
    default_normalized_value: f64,
    unit_id: i32,
    flags: i32,
}

/// Single-object plugin implementing IComponent, IAudioProcessor and
/// IEditController; interface pointers are fixed offsets into the object.
#[repr(C)]
pub struct TestPlugin {
    component_vtable: *const ComponentVtable,
    audio_vtable: *const AudioVtable,
    edit_vtable: *const EditVtable,
    refs: AtomicU32,
    /// Normalized 0..1 gain (real gain = normalized * GAIN_MAX).
    normalized: f64,
}

const COMPONENT_OFFSET: usize = 0;
const AUDIO_OFFSET: usize = 8;
const EDIT_OFFSET: usize = 16;

/// Recover the outer object from any of its three interface pointers by
/// matching the first word (vtable pointer) against the known statics.
fn plugin_from_interface(ptr: *mut c_void) -> *mut TestPlugin {
    if ptr.is_null() {
        return std::ptr::null_mut();
    }
    let vtable = unsafe { *(ptr as *const *const c_void) };
    let component = &COMPONENT_VTABLE as *const ComponentVtable as *const c_void;
    let audio = &AUDIO_VTABLE as *const AudioVtable as *const c_void;
    let edit = &EDIT_VTABLE as *const EditVtable as *const c_void;
    let offset = if vtable == component {
        COMPONENT_OFFSET
    } else if vtable == audio {
        AUDIO_OFFSET
    } else if vtable == edit {
        EDIT_OFFSET
    } else {
        return std::ptr::null_mut();
    };
    (ptr as *mut u8).wrapping_byte_sub(offset) as *mut TestPlugin
}

unsafe extern "system" fn plugin_query_interface(
    this: *mut c_void,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    let plugin = plugin_from_interface(this);
    if plugin.is_null() {
        unsafe { *obj = std::ptr::null_mut() };
        return 0x8000_4002u32 as i32;
    }
    let iid = unsafe { &*(iid as *const TUID) };
    let base = plugin as usize;
    let target: *mut c_void = if iid == &I_COMPONENT_IID {
        base as *mut c_void
    } else if iid == &I_AUDIO_PROCESSOR_IID {
        (base + AUDIO_OFFSET) as *mut c_void
    } else if iid == &I_EDIT_CONTROLLER_IID {
        (base + EDIT_OFFSET) as *mut c_void
    } else {
        std::ptr::null_mut()
    };
    if target.is_null() {
        unsafe { *obj = std::ptr::null_mut() };
        return 0x8000_4002u32 as i32;
    }
    unsafe {
        (*plugin).refs.fetch_add(1, Ordering::SeqCst);
        *obj = target;
    }
    0
}

unsafe extern "system" fn plugin_add_ref(this: *mut c_void) -> u32 {
    let plugin = plugin_from_interface(this);
    (unsafe { (*plugin).refs.fetch_add(1, Ordering::SeqCst) }) + 1
}

unsafe extern "system" fn plugin_release(this: *mut c_void) -> u32 {
    let plugin = plugin_from_interface(this);
    let remaining = unsafe { (*plugin).refs.fetch_sub(1, Ordering::SeqCst) } - 1;
    if remaining == 0 {
        drop(unsafe { Box::from_raw(plugin) });
        0
    } else {
        remaining as u32
    }
}

// --- IComponent ---

unsafe extern "system" fn comp_initialize(_this: *mut TestPlugin, _context: *mut c_void) -> i32 {
    0
}
unsafe extern "system" fn comp_terminate(_this: *mut TestPlugin) -> i32 {
    0
}
unsafe extern "system" fn comp_get_controller_class_id(
    _this: *mut TestPlugin,
    _class_id: *mut u8,
) -> i32 {
    1 // kResultFalse: single-object plugin
}
unsafe extern "system" fn comp_set_io_mode(_this: *mut TestPlugin, _mode: i32) -> i32 {
    0
}
unsafe extern "system" fn comp_get_bus_count(
    _this: *mut TestPlugin,
    _media: i32,
    _direction: i32,
) -> i32 {
    1
}
unsafe extern "system" fn comp_get_bus_info(
    _this: *mut TestPlugin,
    _media: i32,
    direction: i32,
    _index: i32,
    info: *mut BusInfo,
) -> i32 {
    let info = unsafe { &mut *info };
    info.media_type = 0;
    info.direction = direction;
    info.channel_count = CHANNELS as i32;
    info.name = [0; 128];
    info.bus_type = 0; // main
    info.flags = 1; // default active
    0
}
unsafe extern "system" fn comp_get_routing_info(
    _this: *mut TestPlugin,
    _in_info: *mut c_void,
    _out_info: *mut c_void,
) -> i32 {
    0x8000_4001u32 as i32
}
unsafe extern "system" fn comp_activate_bus(
    _this: *mut TestPlugin,
    _media: i32,
    _direction: i32,
    _index: i32,
    _state: u8,
) -> i32 {
    0
}
unsafe extern "system" fn comp_set_active(_this: *mut TestPlugin, _state: u8) -> i32 {
    0
}

fn read_normalized(stream: *mut BStream) -> Option<f64> {
    let mut bytes = [0u8; 8];
    let read = unsafe {
        ((*(*stream).vtable).read)(stream, bytes.as_mut_ptr().cast(), 8, std::ptr::null_mut())
    };
    (read == 0).then(|| f64::from_le_bytes(bytes))
}

fn write_normalized(plugin: *mut TestPlugin, stream: *mut BStream) -> i32 {
    let bytes = unsafe { (*plugin).normalized }.to_le_bytes();
    unsafe {
        ((*(*stream).vtable).write)(
            stream,
            bytes.as_ptr() as *mut c_void,
            8,
            std::ptr::null_mut(),
        );
    }
    0
}

unsafe extern "system" fn comp_set_state(this: *mut TestPlugin, stream: *mut BStream) -> i32 {
    match read_normalized(stream) {
        Some(value) => {
            unsafe { (*this).normalized = value };
            0
        }
        None => 0x8000_4005u32 as i32,
    }
}
unsafe extern "system" fn comp_get_state(this: *mut TestPlugin, stream: *mut BStream) -> i32 {
    write_normalized(this, stream)
}

#[repr(C)]
pub struct ComponentVtable {
    query_interface: QueryInterfaceFn,
    add_ref: AddRefFn,
    release: ReleaseFn,
    initialize: unsafe extern "system" fn(*mut TestPlugin, *mut c_void) -> i32,
    terminate: unsafe extern "system" fn(*mut TestPlugin) -> i32,
    get_controller_class_id: unsafe extern "system" fn(*mut TestPlugin, *mut u8) -> i32,
    set_io_mode: unsafe extern "system" fn(*mut TestPlugin, i32) -> i32,
    get_bus_count: unsafe extern "system" fn(*mut TestPlugin, i32, i32) -> i32,
    get_bus_info: unsafe extern "system" fn(*mut TestPlugin, i32, i32, i32, *mut BusInfo) -> i32,
    get_routing_info:
        unsafe extern "system" fn(*mut TestPlugin, *mut c_void, *mut c_void) -> i32,
    activate_bus: unsafe extern "system" fn(*mut TestPlugin, i32, i32, i32, u8) -> i32,
    set_active: unsafe extern "system" fn(*mut TestPlugin, u8) -> i32,
    set_state: unsafe extern "system" fn(*mut TestPlugin, *mut BStream) -> i32,
    get_state: unsafe extern "system" fn(*mut TestPlugin, *mut BStream) -> i32,
}

static COMPONENT_VTABLE: ComponentVtable = ComponentVtable {
    query_interface: plugin_query_interface,
    add_ref: plugin_add_ref,
    release: plugin_release,
    initialize: comp_initialize,
    terminate: comp_terminate,
    get_controller_class_id: comp_get_controller_class_id,
    set_io_mode: comp_set_io_mode,
    get_bus_count: comp_get_bus_count,
    get_bus_info: comp_get_bus_info,
    get_routing_info: comp_get_routing_info,
    activate_bus: comp_activate_bus,
    set_active: comp_set_active,
    set_state: comp_set_state,
    get_state: comp_get_state,
};

// --- IAudioProcessor ---

unsafe extern "system" fn audio_set_bus_arrangements(
    _this: *mut TestPlugin,
    _inputs: *mut u64,
    _num_ins: i32,
    _outputs: *mut u64,
    _num_outs: i32,
) -> i32 {
    0
}
unsafe extern "system" fn audio_get_bus_arrangement(
    _this: *mut TestPlugin,
    _direction: i32,
    _index: i32,
    arrangement: *mut u64,
) -> i32 {
    unsafe { *arrangement = 3 }; // stereo
    0
}
unsafe extern "system" fn audio_can_process_sample_size(_this: *mut TestPlugin, size: i32) -> i32 {
    if size == 0 {
        0
    } else {
        1
    }
}
unsafe extern "system" fn audio_get_latency_samples(_this: *mut TestPlugin) -> u32 {
    TEST_LATENCY
}
unsafe extern "system" fn audio_setup_processing(
    _this: *mut TestPlugin,
    _setup: *mut c_void,
) -> i32 {
    0
}
unsafe extern "system" fn audio_set_processing(_this: *mut TestPlugin, _state: u8) -> i32 {
    0
}

// Host-side parameter-change container mirrors (accessed through their
// vtables only; the objects behind them are host-owned).
#[repr(C)]
pub struct ParamValueQueue {
    vtable: *const QueueVtable,
}
#[repr(C)]
pub struct QueueVtable {
    query_interface: QueryInterfaceFn,
    add_ref: AddRefFn,
    release: ReleaseFn,
    get_parameter_id: unsafe extern "system" fn(*mut ParamValueQueue) -> u32,
    get_point_count: unsafe extern "system" fn(*mut ParamValueQueue) -> i32,
    get_point: unsafe extern "system" fn(*mut ParamValueQueue, i32, *mut i32, *mut f64) -> i32,
    add_point: unsafe extern "system" fn(*mut ParamValueQueue, i32, f64, *mut i32) -> i32,
}
#[repr(C)]
pub struct ParameterChanges {
    vtable: *const ChangesVtable,
}
#[repr(C)]
pub struct ChangesVtable {
    query_interface: QueryInterfaceFn,
    add_ref: AddRefFn,
    release: ReleaseFn,
    get_parameter_count: unsafe extern "system" fn(*mut ParameterChanges) -> i32,
    get_parameter_data:
        unsafe extern "system" fn(*mut ParameterChanges, i32) -> *mut ParamValueQueue,
    add_parameter_data:
        unsafe extern "system" fn(*mut ParameterChanges, *const u32, *mut i32) -> *mut ParamValueQueue,
}
#[repr(C)]
pub struct BStream {
    vtable: *const BStreamVtable,
}
#[repr(C)]
pub struct BStreamVtable {
    query_interface: QueryInterfaceFn,
    add_ref: AddRefFn,
    release: ReleaseFn,
    read: unsafe extern "system" fn(*mut BStream, *mut c_void, i32, *mut i32) -> i32,
    write: unsafe extern "system" fn(*mut BStream, *mut c_void, i32, *mut i32) -> i32,
    seek: unsafe extern "system" fn(*mut BStream, i64, i32, *mut i64) -> i32,
    tell: unsafe extern "system" fn(*mut BStream, *mut i64) -> i32,
}

unsafe extern "system" fn audio_process(interface: *mut TestPlugin, data: *mut ProcessData) -> i32 {
    // The audio interface pointer is base+8; recover the object first.
    let this = plugin_from_interface(interface.cast::<c_void>()) as *mut TestPlugin;
    let data = unsafe { &mut *data };
    let mut normalized = unsafe { (*this).normalized };
    // Parameter changes: the last point of each queue for param 0 wins.
    if !data.input_parameter_changes.is_null() {
        let changes = unsafe { &*(data.input_parameter_changes as *const ParameterChanges) };
        let count = unsafe { (*changes.vtable).get_parameter_count }(std::ptr::from_ref(changes).cast_mut());
        for index in 0..count {
            let queue = unsafe { (*changes.vtable).get_parameter_data }(std::ptr::from_ref(changes).cast_mut(), index);
            if queue.is_null() {
                continue;
            }
            let queue = unsafe { &*queue };
            if unsafe { (*queue.vtable).get_parameter_id }(std::ptr::from_ref(queue).cast_mut()) == 0 {
                let points = unsafe { (*queue.vtable).get_point_count }(std::ptr::from_ref(queue).cast_mut());
                let mut value = 0.0;
                let mut offset = 0;
                if points > 0
                    && (unsafe { (*queue.vtable).get_point })(
                        std::ptr::from_ref(queue).cast_mut(),
                        points - 1,
                        &mut offset,
                        &mut value,
                    ) == 0
                {
                    normalized = value;
                }
            }
        }
    }
    unsafe { (*this).normalized = normalized };
    let gain = (normalized * GAIN_MAX) as f32;
    for bus in 0..(data.num_inputs.min(data.num_outputs)).max(0) as usize {
        let input = unsafe { &*data.inputs.add(bus) };
        let output = unsafe { &mut *data.outputs.add(bus) };
        for channel in 0..(output.num_channels.max(0) as usize).min(CHANNELS) {
            let source = unsafe { *input.channel_buffers32.add(channel) };
            let target = unsafe { *output.channel_buffers32.add(channel) };
            for frame in 0..(data.num_samples.max(0) as usize) {
                unsafe { *target.add(frame) = *source.add(frame) * gain };
            }
        }
    }
    0
}
unsafe extern "system" fn audio_get_tail_samples(_this: *mut TestPlugin) -> u32 {
    0
}

#[repr(C)]
pub struct AudioVtable {
    query_interface: QueryInterfaceFn,
    add_ref: AddRefFn,
    release: ReleaseFn,
    set_bus_arrangements:
        unsafe extern "system" fn(*mut TestPlugin, *mut u64, i32, *mut u64, i32) -> i32,
    get_bus_arrangement: unsafe extern "system" fn(*mut TestPlugin, i32, i32, *mut u64) -> i32,
    can_process_sample_size: unsafe extern "system" fn(*mut TestPlugin, i32) -> i32,
    get_latency_samples: unsafe extern "system" fn(*mut TestPlugin) -> u32,
    setup_processing: unsafe extern "system" fn(*mut TestPlugin, *mut c_void) -> i32,
    set_processing: unsafe extern "system" fn(*mut TestPlugin, u8) -> i32,
    process: unsafe extern "system" fn(*mut TestPlugin, *mut ProcessData) -> i32,
    get_tail_samples: unsafe extern "system" fn(*mut TestPlugin) -> u32,
}

static AUDIO_VTABLE: AudioVtable = AudioVtable {
    query_interface: plugin_query_interface,
    add_ref: plugin_add_ref,
    release: plugin_release,
    set_bus_arrangements: audio_set_bus_arrangements,
    get_bus_arrangement: audio_get_bus_arrangement,
    can_process_sample_size: audio_can_process_sample_size,
    get_latency_samples: audio_get_latency_samples,
    setup_processing: audio_setup_processing,
    set_processing: audio_set_processing,
    process: audio_process,
    get_tail_samples: audio_get_tail_samples,
};

// --- IEditController ---

unsafe extern "system" fn edit_initialize(_this: *mut TestPlugin, _context: *mut c_void) -> i32 {
    0
}
unsafe extern "system" fn edit_terminate(_this: *mut TestPlugin) -> i32 {
    0
}
unsafe extern "system" fn edit_set_component_state(
    interface: *mut TestPlugin,
    stream: *mut BStream,
) -> i32 {
    // The edit interface pointer is base+16; recover the object first.
    let this = plugin_from_interface(interface.cast::<c_void>()) as *mut TestPlugin;
    match read_normalized(stream) {
        Some(value) => {
            unsafe { (*this).normalized = value };
            0
        }
        None => 0x8000_4005u32 as i32,
    }
}
unsafe extern "system" fn edit_set_state(interface: *mut TestPlugin, stream: *mut BStream) -> i32 {
    let this = plugin_from_interface(interface.cast::<c_void>()) as *mut TestPlugin;
    edit_set_component_state(this.cast(), stream)
}
unsafe extern "system" fn edit_get_state(interface: *mut TestPlugin, stream: *mut BStream) -> i32 {
    // The edit interface pointer is base+16; recover the object first.
    let this = plugin_from_interface(interface.cast::<c_void>()) as *mut TestPlugin;
    write_normalized(this, stream)
}
unsafe extern "system" fn edit_get_parameter_count(_this: *mut TestPlugin) -> i32 {
    1
}
unsafe extern "system" fn edit_get_parameter_info(
    _this: *mut TestPlugin,
    index: i32,
    info: *mut ParameterInfo,
) -> i32 {
    if index != 0 {
        return 1;
    }
    let info = unsafe { &mut *info };
    *info = ParameterInfo {
        id: 0,
        title: [0; 128],
        short_title: [0; 128],
        units: [0; 128],
        step_count: 0,
        default_normalized_value: DEFAULT_NORMALIZED,
        unit_id: 0,
        flags: 1, // kCanAutomate
    };
    for (slot, unit) in "Gain".encode_utf16().enumerate() {
        info.title[slot] = unit;
    }
    0
}
unsafe extern "system" fn edit_get_param_string_by_value(
    _this: *mut TestPlugin,
    _id: u32,
    _value: f64,
    _string: *mut String128,
) -> i32 {
    1
}
unsafe extern "system" fn edit_get_param_value_by_string(
    _this: *mut TestPlugin,
    _id: u32,
    _string: *const TChar,
    _value: *mut f64,
) -> i32 {
    1
}
unsafe extern "system" fn edit_normalized_param_to_plain(
    _this: *mut TestPlugin,
    _id: u32,
    value: f64,
) -> f64 {
    value * GAIN_MAX
}
unsafe extern "system" fn edit_plain_param_to_normalized(
    _this: *mut TestPlugin,
    _id: u32,
    value: f64,
) -> f64 {
    value / GAIN_MAX
}
unsafe extern "system" fn edit_get_param_normalized(
    interface: *mut TestPlugin,
    _id: u32,
) -> f64 {
    let this = plugin_from_interface(interface.cast::<c_void>()) as *mut TestPlugin;
    unsafe { (*this).normalized }
}
unsafe extern "system" fn edit_set_param_normalized(
    interface: *mut TestPlugin,
    _id: u32,
    value: f64,
) -> i32 {
    let this = plugin_from_interface(interface.cast::<c_void>()) as *mut TestPlugin;
    unsafe { (*this).normalized = value };
    0
}
unsafe extern "system" fn edit_set_component_handler(
    _this: *mut TestPlugin,
    _handler: *mut c_void,
) -> i32 {
    0
}
unsafe extern "system" fn edit_create_view(
    _this: *mut TestPlugin,
    _name: *const c_char,
) -> *mut c_void {
    std::ptr::null_mut()
}

#[repr(C)]
pub struct EditVtable {
    query_interface: QueryInterfaceFn,
    add_ref: AddRefFn,
    release: ReleaseFn,
    initialize: unsafe extern "system" fn(*mut TestPlugin, *mut c_void) -> i32,
    terminate: unsafe extern "system" fn(*mut TestPlugin) -> i32,
    set_component_state: unsafe extern "system" fn(*mut TestPlugin, *mut BStream) -> i32,
    set_state: unsafe extern "system" fn(*mut TestPlugin, *mut BStream) -> i32,
    get_state: unsafe extern "system" fn(*mut TestPlugin, *mut BStream) -> i32,
    get_parameter_count: unsafe extern "system" fn(*mut TestPlugin) -> i32,
    get_parameter_info:
        unsafe extern "system" fn(*mut TestPlugin, i32, *mut ParameterInfo) -> i32,
    get_param_string_by_value:
        unsafe extern "system" fn(*mut TestPlugin, u32, f64, *mut String128) -> i32,
    get_param_value_by_string:
        unsafe extern "system" fn(*mut TestPlugin, u32, *const TChar, *mut f64) -> i32,
    normalized_param_to_plain: unsafe extern "system" fn(*mut TestPlugin, u32, f64) -> f64,
    plain_param_to_normalized: unsafe extern "system" fn(*mut TestPlugin, u32, f64) -> f64,
    get_param_normalized: unsafe extern "system" fn(*mut TestPlugin, u32) -> f64,
    set_param_normalized: unsafe extern "system" fn(*mut TestPlugin, u32, f64) -> i32,
    set_component_handler: unsafe extern "system" fn(*mut TestPlugin, *mut c_void) -> i32,
    create_view: unsafe extern "system" fn(*mut TestPlugin, *const c_char) -> *mut c_void,
}

static EDIT_VTABLE: EditVtable = EditVtable {
    query_interface: plugin_query_interface,
    add_ref: plugin_add_ref,
    release: plugin_release,
    initialize: edit_initialize,
    terminate: edit_terminate,
    set_component_state: edit_set_component_state,
    set_state: edit_set_state,
    get_state: edit_get_state,
    get_parameter_count: edit_get_parameter_count,
    get_parameter_info: edit_get_parameter_info,
    get_param_string_by_value: edit_get_param_string_by_value,
    get_param_value_by_string: edit_get_param_value_by_string,
    normalized_param_to_plain: edit_normalized_param_to_plain,
    plain_param_to_normalized: edit_plain_param_to_normalized,
    get_param_normalized: edit_get_param_normalized,
    set_param_normalized: edit_set_param_normalized,
    set_component_handler: edit_set_component_handler,
    create_view: edit_create_view,
};

// --- Factory ---

unsafe extern "system" fn factory_query_interface(
    this: *mut c_void,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    let iid = unsafe { &*(iid as *const TUID) };
    // IPluginFactory2 is served by the same single object, like the SDK's
    // CPluginFactory.
    if iid == &I_PLUGIN_FACTORY_IID || iid == &I_PLUGIN_FACTORY2_IID {
        unsafe { *obj = this };
        0
    } else {
        unsafe { *obj = std::ptr::null_mut() };
        0x8000_4002u32 as i32
    }
}
unsafe extern "system" fn factory_add_ref(_this: *mut c_void) -> u32 {
    1
}
unsafe extern "system" fn factory_release(_this: *mut c_void) -> u32 {
    1
}
unsafe extern "system" fn factory_get_factory_info(
    _this: *mut IPluginFactory,
    _info: *mut c_void,
) -> i32 {
    0
}
unsafe extern "system" fn factory_count_classes(_this: *mut IPluginFactory) -> i32 {
    1
}
unsafe extern "system" fn factory_get_class_info(
    _this: *mut IPluginFactory,
    index: i32,
    info: *mut PClassInfo,
) -> i32 {
    if index != 0 {
        return 1;
    }
    let info = unsafe { &mut *info };
    *info = PClassInfo {
        cid: TEST_CID,
        cardinality: 1,
        category: [0; 32],
        name: [0; 64],
    };
    for (slot, byte) in b"Audio Module Class\0".iter().enumerate() {
        info.category[slot] = *byte as c_char;
    }
    for (slot, byte) in b"Moiren Test Gain\0".iter().enumerate() {
        info.name[slot] = *byte as c_char;
    }
    0
}
unsafe extern "system" fn factory_get_class_info2(
    _this: *mut IPluginFactory,
    index: i32,
    info: *mut PClassInfo2,
) -> i32 {
    if index != 0 {
        return 1;
    }
    let info = unsafe { &mut *info };
    *info = PClassInfo2 {
        cid: TEST_CID,
        cardinality: 1,
        category: [0; 32],
        name: [0; 64],
        class_flags: 0,
        sub_categories: [0; 128],
        vendor: [0; 64],
        version: [0; 64],
        sdk_version: [0; 64],
    };
    for (slot, byte) in b"Audio Module Class\0".iter().enumerate() {
        info.category[slot] = *byte as c_char;
    }
    for (slot, byte) in b"Moiren Test Gain\0".iter().enumerate() {
        info.name[slot] = *byte as c_char;
    }
    for (slot, byte) in b"Fx\0".iter().enumerate() {
        info.sub_categories[slot] = *byte as c_char;
    }
    for (slot, byte) in b"Moiren\0".iter().enumerate() {
        info.vendor[slot] = *byte as c_char;
    }
    for (slot, byte) in b"1.0.0\0".iter().enumerate() {
        info.version[slot] = *byte as c_char;
    }
    for (slot, byte) in b"VST 3.8.1\0".iter().enumerate() {
        info.sdk_version[slot] = *byte as c_char;
    }
    0
}
unsafe extern "system" fn factory_create_instance(
    _this: *mut IPluginFactory,
    cid: *const c_char,
    iid: *const c_char,
    obj: *mut *mut c_void,
) -> i32 {
    let cid = unsafe { &*(cid as *const TUID) };
    let iid = unsafe { &*(iid as *const TUID) };
    if cid != &TEST_CID {
        unsafe { *obj = std::ptr::null_mut() };
        return 0x8000_4005u32 as i32;
    }
    let plugin = Box::new(TestPlugin {
        component_vtable: &COMPONENT_VTABLE,
        audio_vtable: &AUDIO_VTABLE,
        edit_vtable: &EDIT_VTABLE,
        refs: AtomicU32::new(1),
        normalized: DEFAULT_NORMALIZED,
    });
    let base = Box::into_raw(plugin) as usize;
    let target = if iid == &I_COMPONENT_IID {
        base
    } else if iid == &I_AUDIO_PROCESSOR_IID {
        base + AUDIO_OFFSET
    } else if iid == &I_EDIT_CONTROLLER_IID {
        base + EDIT_OFFSET
    } else {
        drop(unsafe { Box::from_raw(base as *mut TestPlugin) });
        unsafe { *obj = std::ptr::null_mut() };
        return 0x8000_4002u32 as i32;
    };
    unsafe { *obj = target as *mut c_void };
    0
}

#[repr(C)]
pub struct IPluginFactory {
    vtable: *const FactoryVtable,
}
#[repr(C)]
pub struct FactoryVtable {
    query_interface: QueryInterfaceFn,
    add_ref: AddRefFn,
    release: ReleaseFn,
    get_factory_info: unsafe extern "system" fn(*mut IPluginFactory, *mut c_void) -> i32,
    count_classes: unsafe extern "system" fn(*mut IPluginFactory) -> i32,
    get_class_info: unsafe extern "system" fn(*mut IPluginFactory, i32, *mut PClassInfo) -> i32,
    create_instance: unsafe extern "system" fn(
        *mut IPluginFactory,
        cid: *const c_char,
        iid: *const c_char,
        obj: *mut *mut c_void,
    ) -> i32,
    // IPluginFactory2's own slot sits after the full IPluginFactory prefix,
    // exactly as in a C++ object inheriting IPluginFactory2.
    get_class_info2: unsafe extern "system" fn(*mut IPluginFactory, i32, *mut PClassInfo2) -> i32,
}

// The factory and its vtable are immutable static state.
unsafe impl Sync for IPluginFactory {}

static FACTORY_VTABLE: FactoryVtable = FactoryVtable {
    query_interface: factory_query_interface,
    add_ref: factory_add_ref,
    release: factory_release,
    get_factory_info: factory_get_factory_info,
    count_classes: factory_count_classes,
    get_class_info: factory_get_class_info,
    create_instance: factory_create_instance,
    get_class_info2: factory_get_class_info2,
};

static FACTORY: IPluginFactory = IPluginFactory {
    vtable: &FACTORY_VTABLE,
};

#[unsafe(no_mangle)]
pub extern "system" fn GetPluginFactory() -> *mut IPluginFactory {
    &FACTORY as *const IPluginFactory as *mut IPluginFactory
}

// VST3 module-level init pair plus exported counters, so a test can verify
// the host runs `InitDll` once per actual load and defers `ExitDll` to the
// last release. Copies loaded under other names (CLAP/VST2) simply never
// call these exports.
static MODULE_INITS: AtomicI32 = AtomicI32::new(0);
static MODULE_EXITS: AtomicI32 = AtomicI32::new(0);

#[unsafe(no_mangle)]
pub extern "C" fn InitDll() -> bool {
    MODULE_INITS.fetch_add(1, Ordering::SeqCst);
    true
}

#[unsafe(no_mangle)]
pub extern "C" fn ExitDll() {
    MODULE_EXITS.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
pub extern "C" fn moiren_module_init_count() -> i32 {
    MODULE_INITS.load(Ordering::SeqCst)
}

#[unsafe(no_mangle)]
pub extern "C" fn moiren_module_exit_count() -> i32 {
    MODULE_EXITS.load(Ordering::SeqCst)
}
