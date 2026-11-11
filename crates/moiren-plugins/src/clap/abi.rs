//! CLAP 1.2 C ABI declarations, transcribed from the official headers
//! (<https://github.com/free-audio/clap>, include/clap).
//!
//! Field order and types must match the headers exactly; every struct is
//! `#[repr(C)]` with natural alignment, which matches the CLAP requirement
//! that host and plugin agree on layout without packing pragmas.

use std::os::raw::{c_char, c_void};

pub const CLAP_PLUGIN_FACTORY_ID: &[u8] = b"clap.plugin-factory\0";
pub const CLAP_EXT_AUDIO_PORTS: &[u8] = b"clap.audio-ports\0";
pub const CLAP_EXT_PARAMS: &[u8] = b"clap.params\0";
pub const CLAP_EXT_STATE: &[u8] = b"clap.state\0";
pub const CLAP_EXT_LATENCY: &[u8] = b"clap.latency\0";
pub const CLAP_EXT_THREAD_CHECK: &[u8] = b"clap.thread-check\0";

pub const CLAP_PROCESS_ERROR: i32 = 0;
pub const CLAP_PROCESS_CONTINUE: i32 = 1;

pub const CLAP_EVENT_PARAM_VALUE: u16 = 5;
pub const CLAP_CORE_EVENT_SPACE_ID: u16 = 0;

pub const CLAP_PARAM_IS_STEPPED: u32 = 1 << 0;
pub const CLAP_PARAM_IS_HIDDEN: u32 = 1 << 2;
pub const CLAP_PARAM_IS_READONLY: u32 = 1 << 3;
pub const CLAP_PARAM_IS_BYPASS: u32 = 1 << 4;
pub const CLAP_PARAM_IS_AUTOMATABLE: u32 = 1 << 5;

pub const CLAP_AUDIO_PORT_IS_MAIN: u32 = 1 << 0;

pub const CLAP_NAME_SIZE: usize = 256;
pub const CLAP_PATH_SIZE: usize = 1024;

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ClapVersion {
    pub major: u32,
    pub minor: u32,
    pub revision: u32,
}

pub const CLAP_VERSION: ClapVersion = ClapVersion {
    major: 1,
    minor: 2,
    revision: 10,
};

#[repr(C)]
pub struct ClapEntry {
    pub clap_version: ClapVersion,
    pub init: unsafe extern "C" fn(plugin_path: *const c_char) -> bool,
    pub deinit: unsafe extern "C" fn(),
    pub get_factory: unsafe extern "C" fn(factory_id: *const c_char) -> *const c_void,
}

#[repr(C)]
pub struct ClapPluginDescriptor {
    pub clap_version: ClapVersion,
    pub id: *const c_char,
    pub name: *const c_char,
    pub vendor: *const c_char,
    pub url: *const c_char,
    pub manual_url: *const c_char,
    pub support_url: *const c_char,
    pub version: *const c_char,
    pub description: *const c_char,
    pub features: *const *const c_char,
}

#[repr(C)]
pub struct ClapPluginFactory {
    pub get_plugin_count: unsafe extern "C" fn(factory: *const ClapPluginFactory) -> u32,
    pub get_plugin_descriptor: unsafe extern "C" fn(
        factory: *const ClapPluginFactory,
        index: u32,
    ) -> *const ClapPluginDescriptor,
    pub create_plugin: unsafe extern "C" fn(
        factory: *const ClapPluginFactory,
        host: *const ClapHost,
        plugin_id: *const c_char,
    ) -> *const ClapPlugin,
}

/// The host side of the contract. This host implements no host extensions:
/// `get_extension` returns null, and restart/callback requests are legal
/// no-ops for this host revision (see the host callbacks in `mod.rs`).
#[repr(C)]
pub struct ClapHost {
    pub clap_version: ClapVersion,
    pub host_data: *mut c_void,
    pub name: *const c_char,
    pub vendor: *const c_char,
    pub url: *const c_char,
    pub version: *const c_char,
    pub get_extension:
        unsafe extern "C" fn(host: *const ClapHost, extension_id: *const c_char) -> *const c_void,
    pub request_restart: unsafe extern "C" fn(host: *const ClapHost),
    pub request_process: unsafe extern "C" fn(host: *const ClapHost),
    pub request_callback: unsafe extern "C" fn(host: *const ClapHost),
}

#[repr(C)]
pub struct ClapPlugin {
    pub desc: *const ClapPluginDescriptor,
    pub plugin_data: *mut c_void,
    pub init: unsafe extern "C" fn(plugin: *const ClapPlugin) -> bool,
    pub destroy: unsafe extern "C" fn(plugin: *const ClapPlugin),
    pub activate: unsafe extern "C" fn(
        plugin: *const ClapPlugin,
        sample_rate: f64,
        min_frames_count: u32,
        max_frames_count: u32,
    ) -> bool,
    pub deactivate: unsafe extern "C" fn(plugin: *const ClapPlugin),
    pub start_processing: unsafe extern "C" fn(plugin: *const ClapPlugin) -> bool,
    pub stop_processing: unsafe extern "C" fn(plugin: *const ClapPlugin),
    pub reset: unsafe extern "C" fn(plugin: *const ClapPlugin),
    pub process:
        unsafe extern "C" fn(plugin: *const ClapPlugin, process: *const ClapProcess) -> i32,
    pub get_extension:
        unsafe extern "C" fn(plugin: *const ClapPlugin, id: *const c_char) -> *const c_void,
    pub on_main_thread: unsafe extern "C" fn(plugin: *const ClapPlugin),
}

#[repr(C)]
pub struct ClapAudioBuffer {
    pub data32: *mut *mut f32,
    pub data64: *mut *mut f64,
    pub channel_count: u32,
    pub latency: u32,
    pub constant_mask: u64,
}

#[repr(C)]
pub struct ClapProcess {
    pub steady_time: i64,
    pub frames_count: u32,
    pub transport: *const c_void,
    pub audio_inputs: *const ClapAudioBuffer,
    pub audio_outputs: *mut ClapAudioBuffer,
    pub audio_inputs_count: u32,
    pub audio_outputs_count: u32,
    pub in_events: *const ClapInputEvents,
    pub out_events: *const ClapOutputEvents,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClapEventHeader {
    pub size: u32,
    pub time: u32,
    pub space_id: u16,
    pub event_type: u16,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClapEventParamValue {
    pub header: ClapEventHeader,
    pub param_id: u32,
    pub cookie: *mut c_void,
    pub note_id: i32,
    pub port_index: i16,
    pub channel: i16,
    pub key: i16,
    pub value: f64,
}

impl ClapEventParamValue {
    pub const fn new(param_id: u32, cookie: *mut c_void, value: f64) -> Self {
        Self {
            header: ClapEventHeader {
                size: std::mem::size_of::<Self>() as u32,
                time: 0,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                event_type: CLAP_EVENT_PARAM_VALUE,
                flags: 0,
            },
            param_id,
            cookie,
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value,
        }
    }
}

#[repr(C)]
pub struct ClapInputEvents {
    pub ctx: *mut c_void,
    pub size: unsafe extern "C" fn(list: *const ClapInputEvents) -> u32,
    pub get:
        unsafe extern "C" fn(list: *const ClapInputEvents, index: u32) -> *const ClapEventHeader,
}

#[repr(C)]
pub struct ClapOutputEvents {
    pub ctx: *mut c_void,
    pub try_push:
        unsafe extern "C" fn(list: *const ClapOutputEvents, event: *const ClapEventHeader) -> bool,
}

#[repr(C)]
pub struct ClapAudioPorts {
    pub count: unsafe extern "C" fn(plugin: *const ClapPlugin, is_input: bool) -> u32,
    pub get: unsafe extern "C" fn(
        plugin: *const ClapPlugin,
        index: u32,
        is_input: bool,
        info: *mut ClapAudioPortInfo,
    ) -> bool,
}

#[repr(C)]
pub struct ClapAudioPortInfo {
    pub id: u32,
    pub name: [c_char; CLAP_NAME_SIZE],
    pub flags: u32,
    pub channel_count: u32,
    pub port_type: *const c_char,
    pub in_place_pair: u32,
}

#[repr(C)]
pub struct ClapParams {
    pub count: unsafe extern "C" fn(plugin: *const ClapPlugin) -> u32,
    pub get_info: unsafe extern "C" fn(
        plugin: *const ClapPlugin,
        param_index: u32,
        info: *mut ClapParamInfo,
    ) -> bool,
    pub get_value:
        unsafe extern "C" fn(plugin: *const ClapPlugin, param_id: u32, out_value: *mut f64) -> bool,
    pub value_to_text: unsafe extern "C" fn(
        plugin: *const ClapPlugin,
        param_id: u32,
        value: f64,
        out_buffer: *mut c_char,
        out_buffer_capacity: u32,
    ) -> bool,
    pub text_to_value: unsafe extern "C" fn(
        plugin: *const ClapPlugin,
        param_id: u32,
        text: *const c_char,
        out_value: *mut f64,
    ) -> bool,
    pub flush: unsafe extern "C" fn(
        plugin: *const ClapPlugin,
        input_events: *const ClapInputEvents,
        output_events: *const ClapOutputEvents,
    ),
}

#[repr(C)]
pub struct ClapParamInfo {
    pub id: u32,
    pub flags: u32,
    pub cookie: *mut c_void,
    pub name: [c_char; CLAP_NAME_SIZE],
    pub module: [c_char; CLAP_PATH_SIZE],
    pub min_value: f64,
    pub max_value: f64,
    pub default_value: f64,
}

#[repr(C)]
pub struct ClapState {
    pub save: unsafe extern "C" fn(plugin: *const ClapPlugin, stream: *const ClapOstream) -> bool,
    pub load: unsafe extern "C" fn(plugin: *const ClapPlugin, stream: *const ClapIstream) -> bool,
}

#[repr(C)]
pub struct ClapIstream {
    pub ctx: *mut c_void,
    pub read:
        unsafe extern "C" fn(stream: *const ClapIstream, buffer: *mut c_void, size: u64) -> i64,
}

#[repr(C)]
pub struct ClapOstream {
    pub ctx: *mut c_void,
    pub write:
        unsafe extern "C" fn(stream: *const ClapOstream, buffer: *const c_void, size: u64) -> i64,
}

#[repr(C)]
pub struct ClapLatency {
    pub get: unsafe extern "C" fn(plugin: *const ClapPlugin) -> u32,
}

// Safe dispatch wrappers: one `unsafe` per slot, at one site each. Every
// method carries the same contract: the plugin (or factory) must be live,
// and — for plugin methods — the plugin must be in the lifecycle state the
// CLAP spec requires for that call.

impl ClapEntry {
    /// # Safety
    ///
    /// The entry must belong to a live library mapping; `factory_id` must
    /// be nul-terminated for the call's duration.
    pub unsafe fn get_factory(&self, factory_id: *const c_char) -> *const c_void {
        // SAFETY: the entry belongs to a mapped library.
        unsafe { (self.get_factory)(factory_id) }
    }
}

impl ClapPluginFactory {
    pub fn plugin_count(&self) -> u32 {
        // SAFETY: factory from a live clap_entry.
        unsafe { (self.get_plugin_count)(self) }
    }
    pub fn plugin_descriptor(&self, index: u32) -> *const ClapPluginDescriptor {
        // SAFETY: as above.
        unsafe { (self.get_plugin_descriptor)(self, index) }
    }
    /// # Safety
    ///
    /// Live factory; `host` must outlive the returned plugin and
    /// `plugin_id` must be nul-terminated.
    pub unsafe fn create_plugin(
        &self,
        host: *const ClapHost,
        plugin_id: *const c_char,
    ) -> *const ClapPlugin {
        // SAFETY: as above.
        unsafe { (self.create_plugin)(self, host, plugin_id) }
    }
}

impl ClapPlugin {
    /// # Safety
    ///
    /// All methods: `this` must be a live plugin in the lifecycle state
    /// the CLAP spec requires for the call; `destroy` ends its lifetime.
    pub unsafe fn init(this: *const Self) -> bool {
        // SAFETY: created, not yet initialized.
        unsafe { ((*this).init)(this) }
    }
    /// # Safety
    ///
    /// Plugin lifecycle state per the impl-level contract.
    pub unsafe fn destroy(this: *const Self) {
        // SAFETY: created (and initialized on the failing paths that use
        // this); must not be used afterwards.
        unsafe { ((*this).destroy)(this) }
    }
    /// # Safety
    ///
    /// Plugin lifecycle state per the impl-level contract.
    pub unsafe fn activate(
        this: *const Self,
        sample_rate: f64,
        min_frames: u32,
        max_frames: u32,
    ) -> bool {
        // SAFETY: initialized, not activated.
        unsafe { ((*this).activate)(this, sample_rate, min_frames, max_frames) }
    }
    /// # Safety
    ///
    /// Plugin lifecycle state per the impl-level contract.
    pub unsafe fn deactivate(this: *const Self) {
        // SAFETY: activated.
        unsafe { ((*this).deactivate)(this) }
    }
    /// # Safety
    ///
    /// Plugin lifecycle state per the impl-level contract.
    pub unsafe fn start_processing(this: *const Self) -> bool {
        // SAFETY: activated.
        unsafe { ((*this).start_processing)(this) }
    }
    /// # Safety
    ///
    /// Plugin lifecycle state per the impl-level contract.
    pub unsafe fn stop_processing(this: *const Self) {
        // SAFETY: processing started.
        unsafe { ((*this).stop_processing)(this) }
    }
    /// # Safety
    ///
    /// Plugin lifecycle state per the impl-level contract; `process`
    /// references valid buffers and host objects for the call.
    pub unsafe fn process(this: *const Self, process: *const ClapProcess) -> i32 {
        // SAFETY: processing started; `process` references valid buffers and
        // host objects for the duration of the call.
        unsafe { ((*this).process)(this, process) }
    }
    /// # Safety
    ///
    /// Plugin lifecycle state per the impl-level contract; `id` is a
    /// nul-terminated literal.
    pub unsafe fn extension(this: *const Self, id: &[u8]) -> *const c_void {
        debug_assert!(id.ends_with(b"\0"));
        // SAFETY: initialized plugin; id is a compile-time literal.
        unsafe { ((*this).get_extension)(this, id.as_ptr().cast()) }
    }
}

impl ClapAudioPorts {
    /// # Safety
    ///
    /// Both methods: live plugin whose extension this is.
    pub unsafe fn count(&self, plugin: *const ClapPlugin, is_input: bool) -> u32 {
        // SAFETY: extension queried from an initialized plugin.
        unsafe { (self.count)(plugin, is_input) }
    }
    /// # Safety
    ///
    /// Plugin lifecycle state per the impl-level contract.
    pub unsafe fn get(
        &self,
        plugin: *const ClapPlugin,
        index: u32,
        is_input: bool,
        info: &mut ClapAudioPortInfo,
    ) -> bool {
        // SAFETY: as above.
        unsafe { (self.get)(plugin, index, is_input, info) }
    }
}

impl ClapParams {
    /// # Safety
    ///
    /// Both methods: live plugin whose extension this is.
    pub unsafe fn count(&self, plugin: *const ClapPlugin) -> u32 {
        // SAFETY: extension queried from an initialized plugin.
        unsafe { (self.count)(plugin) }
    }
    /// # Safety
    ///
    /// Plugin lifecycle state per the impl-level contract.
    pub unsafe fn info(
        &self,
        plugin: *const ClapPlugin,
        index: u32,
        info: &mut ClapParamInfo,
    ) -> bool {
        // SAFETY: as above.
        unsafe { (self.get_info)(plugin, index, info) }
    }
}

impl ClapLatency {
    /// # Safety
    ///
    /// Live plugin whose extension this is.
    pub unsafe fn latency(&self, plugin: *const ClapPlugin) -> u32 {
        // SAFETY: extension queried from an initialized plugin.
        unsafe { (self.get)(plugin) }
    }
}

impl ClapState {
    /// # Safety
    ///
    /// Both methods: live initialized plugin; `stream` valid for the call.
    pub unsafe fn save(&self, plugin: *const ClapPlugin, stream: &ClapOstream) -> bool {
        // SAFETY: plugin initialized; stream valid for the call.
        unsafe { (self.save)(plugin, stream) }
    }
    /// # Safety
    ///
    /// Plugin lifecycle state per the impl-level contract.
    pub unsafe fn load(&self, plugin: *const ClapPlugin, stream: &ClapIstream) -> bool {
        // SAFETY: as above.
        unsafe { (self.load)(plugin, stream) }
    }
}
