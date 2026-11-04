//! CLAP backend: library entry, host callbacks, and the live instance.
//!
//! The host advertises no extensions (`get_extension` returns null) and the
//! request callbacks are accepted-and-ignored sinks: restart handling is a
//! documented follow-up, not part of the MVP. All FFI happens control-side
//! except `process`, whose plumbing is fully preallocated.

use std::{
    ffi::{CStr, CString, c_char},
    os::raw::c_void,
    path::Path,
    sync::Arc,
};

use libloading::Library;

pub mod abi;

use crate::{
    instance::PluginInstance,
    module::{SharedModule, acquire_module},
    spec::{PluginDescriptor, PluginError, PluginFormat, PluginParamEvent, PluginParameterSpec},
};
use abi::*;

const CLAP_ENTRY_SYMBOL: &[u8] = b"clap_entry";

/// Module-level state shared by every instance of one CLAP file.
struct ClapModule {
    _module: Arc<SharedModule>,
    entry: &'static ClapEntry,
}

fn load_module(path: &Path) -> Result<ClapModule, PluginError> {
    let module = acquire_module(path, |library| {
        // SAFETY: the entry reference is used within this callback, while
        // the library handle passed in is live.
        let entry: &'static ClapEntry = unsafe { entry_of(library)? };
        if entry.clap_version.major != 1 {
            return Err(PluginError::Entry(format!(
                "unsupported CLAP version {}.{}.{}",
                entry.clap_version.major, entry.clap_version.minor, entry.clap_version.revision
            )));
        }
        let path_bytes = CString::new(path.as_os_str().to_string_lossy().as_bytes())
            .map_err(|_| PluginError::Entry("plugin path is not a valid C string".into()))?;
        if !(unsafe { (entry.init)(path_bytes.as_ptr()) }) {
            return Err(PluginError::Init(
                "clap_entry.init rejected the path".into(),
            ));
        }
        Ok(Some(entry.deinit))
    })?;
    // SAFETY: the module handle keeps the library mapped for as long as the
    // returned reference is retained by ClapModule.
    let entry: &'static ClapEntry = unsafe { entry_of(&module.library)? };
    Ok(ClapModule {
        _module: module,
        entry,
    })
}

/// SAFETY contract (call sites): the returned reference must not be used
/// after the passed library handle (or its SharedModule) is gone.
unsafe fn entry_of(library: &Library) -> Result<&'static ClapEntry, PluginError> {
    // SAFETY: symbol lookup on a live library; the plugin guarantees the
    // entry object outlives the library mapping.
    let symbol = unsafe { library.get::<*const ClapEntry>(CLAP_ENTRY_SYMBOL) }
        .map_err(|_| PluginError::Entry("clap_entry symbol missing".into()))?;
    let entry = *symbol;
    if entry.is_null() {
        return Err(PluginError::Entry("clap_entry is null".into()));
    }
    // SAFETY: see above.
    Ok(unsafe { &*entry })
}

/// Host callbacks. Restart/process/callback requests are legal no-ops for
/// this host revision; they must still be callable, so each lands here.
unsafe extern "C" fn host_get_extension(
    _host: *const ClapHost,
    _extension_id: *const c_char,
) -> *const c_void {
    std::ptr::null()
}
unsafe extern "C" fn host_request_restart(_host: *const ClapHost) {}
unsafe extern "C" fn host_request_process(_host: *const ClapHost) {}
unsafe extern "C" fn host_request_callback(_host: *const ClapHost) {}

fn host_boxed() -> Box<ClapHost> {
    let mut host = Box::new(ClapHost {
        clap_version: CLAP_VERSION,
        host_data: std::ptr::null_mut(),
        name: c"Moiren".as_ptr(),
        vendor: c"Moiren".as_ptr(),
        url: c"".as_ptr(),
        version: c"0.1".as_ptr(),
        get_extension: host_get_extension,
        request_restart: host_request_restart,
        request_process: host_request_process,
        request_callback: host_request_callback,
    });
    // Stable Box address for the pointer the plugin retains until destroy.
    host.host_data = std::ptr::addr_of_mut!(*host).cast();
    host
}

/// Preallocated event list handed to the plugin inside `clap_process`.
struct EventQueue {
    list: ClapInputEvents,
    events: Box<[ClapEventParamValue]>,
    len: u32,
}

unsafe extern "C" fn events_size(list: *const ClapInputEvents) -> u32 {
    let queue = unsafe { (*list).ctx as *const EventQueue };
    if queue.is_null() {
        return 0;
    }
    // SAFETY: ctx points at the EventQueue owning this list (set at
    // construction, before any plugin call).
    unsafe { (*queue).len }
}
unsafe extern "C" fn events_get(
    list: *const ClapInputEvents,
    index: u32,
) -> *const ClapEventHeader {
    let queue = unsafe { (*list).ctx as *const EventQueue };
    if queue.is_null() || index >= unsafe { (*queue).len } {
        return std::ptr::null();
    }
    // SAFETY: index is bounds-checked against len, which only shrinks to the
    // events capacity fixed at construction.
    unsafe { (*queue).events.as_ptr().add(index as usize).cast() }
}

/// Null sink for plugin output events; dropping them is spec-legal.
unsafe extern "C" fn out_events_try_push(
    _list: *const ClapOutputEvents,
    _event: *const ClapEventHeader,
) -> bool {
    false
}

const OUT_EVENTS: ClapOutputEvents = ClapOutputEvents {
    ctx: std::ptr::null_mut(),
    try_push: out_events_try_push,
};

pub(crate) struct ClapInstance {
    descriptor: PluginDescriptor,
    parameters: Vec<PluginParameterSpec>,
    /// Keeps the entry (and the library) alive until Drop; never read.
    _module: Arc<ClapModule>,
    plugin: *const ClapPlugin,
    /// Owns the host object the plugin retains until destroy; never read.
    _host: Box<ClapHost>,
    latency: u32,
    /// Extension table captured at init; plugin-owned, valid until destroy.
    state_ext: Option<&'static ClapState>,
    /// Per-channel planar pointers for the main buses, reused per block.
    input_ptrs: Box<[*mut f32]>,
    output_ptrs: Box<[*mut f32]>,
    input_buffer: ClapAudioBuffer,
    output_buffer: ClapAudioBuffer,
    in_events: Box<EventQueue>,
}

// The plugin object may be created, activated, processed and destroyed on
// different threads as long as no two threads use it concurrently; the CLAP
// spec requires the plugin itself to tolerate exactly that. Every pointer in
// this struct belongs to the plugin or module, both outliving the instance.
unsafe impl Send for ClapInstance {}

pub(crate) fn instantiate(
    source: &PluginDescriptor,
    sample_rate: f64,
    max_block_frames: u32,
) -> Result<ClapInstance, PluginError> {
    let module = Arc::new(load_module(&source.path)?);
    // SAFETY: the module handle keeps the entry mapped.
    let factory_ptr = unsafe {
        module
            .entry
            .get_factory(CLAP_PLUGIN_FACTORY_ID.as_ptr().cast())
    };
    if factory_ptr.is_null() {
        return Err(PluginError::Entry(
            "clap.plugin-factory not provided by module".into(),
        ));
    }
    // SAFETY: the factory belongs to the live module and is used while it
    // stays mapped (ClapModule is held by the instance).
    let factory = unsafe { &*(factory_ptr as *const ClapPluginFactory) };
    let count = factory.plugin_count();
    let id = CString::new(source.id.as_str())
        .map_err(|_| PluginError::Entry("plugin id is not a valid C string".into()))?;
    let mut selected: Option<OwnedDescriptor> = None;
    for index in 0..count {
        let info = factory.plugin_descriptor(index);
        if info.is_null() {
            continue;
        }
        // SAFETY: factory-owned descriptor; copied out immediately.
        let owned = unsafe { OwnedDescriptor::read(info) };
        if owned.id == source.id {
            selected = Some(owned);
            break;
        }
    }
    let Some(owned) = selected else {
        return Err(PluginError::Entry(format!(
            "plugin id {:?} not in factory ({count} plugins)",
            source.id
        )));
    };

    let host = host_boxed();
    // SAFETY: the module keeps the factory live; the host box outlives the
    // plugin through the instance field.
    let plugin = unsafe { factory.create_plugin(&*host, id.as_ptr()) };
    if plugin.is_null() {
        return Err(PluginError::Init("factory refused create_plugin".into()));
    }
    // SAFETY: the plugin object stays valid until destroy, which runs in
    // Drop before the module or host can be released.
    let plugin_ref = unsafe { &*plugin };
    // Every failure below unwinds a fully-created plugin with the
    // documented destroy call so no module-side state leaks.
    // SAFETY: freshly created plugin.
    if !(unsafe { ClapPlugin::init(plugin) }) {
        // The plugin was created but not initialized; destroy releases it.
        // SAFETY: created plugin whose init failed.
        unsafe { ClapPlugin::destroy(plugin) };
        return Err(PluginError::Init("plugin.init failed".into()));
    }
    // SAFETY: plugin initialized; extensions are valid until destroy.
    let audio_ports = unsafe { extension::<ClapAudioPorts>(plugin_ref, CLAP_EXT_AUDIO_PORTS) };
    let (input_channels, output_channels) =
        main_bus_widths(plugin_ref, audio_ports).unwrap_or((2, 2));
    if input_channels == 0 || input_channels != output_channels {
        // Initialized but not activated; destroy is the documented unwind.
        // SAFETY: initialized plugin.
        unsafe { ClapPlugin::destroy(plugin) };
        return Err(PluginError::NoAudioPorts {
            input: input_channels,
            output: output_channels,
        });
    }
    // SAFETY: same lifetime class as audio_ports.
    let parameters = collect_parameters(plugin_ref, unsafe {
        extension::<ClapParams>(plugin_ref, CLAP_EXT_PARAMS)
    });
    let latency = unsafe { extension::<ClapLatency>(plugin_ref, CLAP_EXT_LATENCY) }
        .map(|ext| unsafe { ext.latency(plugin) })
        .unwrap_or(0);
    let state_ext = unsafe { extension::<ClapState>(plugin_ref, CLAP_EXT_STATE) };
    let has_editor = !unsafe { extension_bytes(plugin_ref, b"clap.gui\0") }.is_null();
    let is_synth = owned.is_instrument();

    // SAFETY: initialized plugin.
    if !(unsafe { ClapPlugin::activate(plugin, sample_rate, 1, max_block_frames.max(1)) }) {
        // Initialized; activation failed.
        // SAFETY: initialized plugin.
        unsafe { ClapPlugin::destroy(plugin) };
        return Err(PluginError::Init(format!(
            "activate({sample_rate} Hz, max {max_block_frames} frames) failed"
        )));
    }
    // SAFETY: activated plugin.
    if !(unsafe { ClapPlugin::start_processing(plugin) }) {
        // Activated; unwind activation before destroy.
        // SAFETY: activated plugin.
        unsafe {
            ClapPlugin::deactivate(plugin);
            ClapPlugin::destroy(plugin);
        }
        return Err(PluginError::Init("start_processing failed".into()));
    }

    let descriptor = PluginDescriptor {
        format: PluginFormat::Clap,
        path: source.path.clone(),
        id: owned.id,
        name: owned.name,
        vendor: owned.vendor,
        version: owned.version,
        input_channels,
        output_channels,
        is_synth,
        has_editor,
    };
    let mut input_ptrs =
        vec![std::ptr::null_mut::<f32>(); input_channels as usize].into_boxed_slice();
    let mut output_ptrs =
        vec![std::ptr::null_mut::<f32>(); output_channels as usize].into_boxed_slice();
    // The pointer tables live in Box allocations that never move, so the
    // buffers' data32 pointers stay valid across struct moves.
    let input_buffer = ClapAudioBuffer {
        data32: input_ptrs.as_mut_ptr(),
        data64: std::ptr::null_mut(),
        channel_count: input_channels,
        latency: 0,
        constant_mask: 0,
    };
    let output_buffer = ClapAudioBuffer {
        data32: output_ptrs.as_mut_ptr(),
        data64: std::ptr::null_mut(),
        channel_count: output_channels,
        latency: 0,
        constant_mask: 0,
    };
    let event_capacity = parameters.len().max(1);
    let mut in_events = Box::new(EventQueue {
        list: ClapInputEvents {
            ctx: std::ptr::null_mut(),
            size: events_size,
            get: events_get,
        },
        events: vec![ClapEventParamValue::new(0, std::ptr::null_mut(), 0.0); event_capacity]
            .into_boxed_slice(),
        len: 0,
    });
    // Self-referential ctx: the Box address is stable, so the pointer stays
    // valid for the instance lifetime (plugins only read it during process).
    in_events.list.ctx = std::ptr::addr_of_mut!(*in_events).cast();

    Ok(ClapInstance {
        descriptor,
        parameters,
        _module: module,
        plugin,
        _host: host,
        latency,
        state_ext,
        input_ptrs,
        output_ptrs,
        input_buffer,
        output_buffer,
        in_events,
    })
}

/// Owned copy of a factory descriptor, read once while the factory is live.
struct OwnedDescriptor {
    id: String,
    name: String,
    vendor: String,
    version: String,
    features: Vec<String>,
}

impl OwnedDescriptor {
    /// SAFETY: `info` must point at a factory-owned descriptor that stays
    /// valid for the duration of this call.
    unsafe fn read(info: *const ClapPluginDescriptor) -> Self {
        // SAFETY: caller guarantees validity; strings are copied immediately.
        let raw = unsafe { &*info };
        Self {
            id: cstr(raw.id),
            name: cstr(raw.name),
            vendor: cstr(raw.vendor),
            version: cstr(raw.version),
            features: feature_list(raw.features),
        }
    }

    fn is_instrument(&self) -> bool {
        self.features.iter().any(|f| f == "instrument")
    }
}

fn cstr(raw: *const c_char) -> String {
    if raw.is_null() {
        return String::new();
    }
    // SAFETY: CLAP strings are nul-terminated for the descriptor lifetime.
    unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned()
}

fn feature_list(mut features: *const *const c_char) -> Vec<String> {
    let mut collected = Vec::new();
    while !features.is_null() {
        // SAFETY: the features array is nul-pointer terminated.
        unsafe {
            if (*features).is_null() {
                break;
            }
            collected.push(cstr(*features));
            features = features.add(1);
        }
    }
    collected
}

/// SAFETY contract (call sites): the plugin must be initialized and must not
/// be destroyed while the returned extension reference is used. Extensions
/// live exactly as long as the plugin object.
unsafe fn extension<T>(plugin: &ClapPlugin, id: &[u8]) -> Option<&'static T> {
    let ptr = unsafe { extension_bytes(plugin, id) };
    (!ptr.is_null()).then(|| {
        // SAFETY: non-null plugin-owned extension, lifetime per contract.
        unsafe { &*ptr.cast::<T>() }
    })
}

/// SAFETY contract: same lifetime rules as [`extension`].
unsafe fn extension_bytes(plugin: &ClapPlugin, id: &[u8]) -> *const c_void {
    debug_assert!(id.ends_with(b"\0"));
    // SAFETY: the id slice is a compile-time nul-terminated literal; the
    // call itself is a plain ABI dispatch on an initialized plugin.
    unsafe { (plugin.get_extension)(plugin, id.as_ptr().cast()) }
}

/// Main input/output bus channel counts. A plugin without the audio-ports
/// extension is a stereo 2/2 processor per spec default.
fn main_bus_widths(
    plugin: &ClapPlugin,
    audio_ports: Option<&ClapAudioPorts>,
) -> Option<(u32, u32)> {
    let ext = audio_ports?;
    let mut info = ClapAudioPortInfo {
        id: 0,
        name: [0; CLAP_NAME_SIZE],
        flags: 0,
        channel_count: 0,
        port_type: std::ptr::null(),
        in_place_pair: 0,
    };
    let mut main_width = |is_input: bool| -> Option<u32> {
        for index in 0..unsafe { ext.count(plugin, is_input) } {
            if unsafe { ext.get(plugin, index, is_input, &mut info) }
                && info.flags & CLAP_AUDIO_PORT_IS_MAIN != 0
            {
                return Some(info.channel_count);
            }
        }
        None
    };
    Some((main_width(true)?, main_width(false)?))
}

fn collect_parameters(plugin: &ClapPlugin, ext: Option<&ClapParams>) -> Vec<PluginParameterSpec> {
    let Some(ext) = ext else { return Vec::new() };
    let mut specs = Vec::new();
    for index in 0..unsafe { ext.count(plugin) } {
        let mut info = ClapParamInfo {
            id: 0,
            flags: 0,
            cookie: std::ptr::null_mut(),
            name: [0; CLAP_NAME_SIZE],
            module: [0; CLAP_PATH_SIZE],
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.0,
        };
        if !(unsafe { ext.info(plugin, index, &mut info) }) {
            continue;
        }
        if info.flags & (CLAP_PARAM_IS_HIDDEN | CLAP_PARAM_IS_READONLY) != 0 {
            continue;
        }
        specs.push(PluginParameterSpec {
            id: info.id,
            name: cstr(info.name.as_ptr()),
            min: info.min_value,
            max: info.max_value,
            default: info.default_value,
            automatable: info.flags & CLAP_PARAM_IS_AUTOMATABLE != 0,
            stepped: info.flags & CLAP_PARAM_IS_STEPPED != 0,
        });
    }
    specs
}

impl PluginInstance for ClapInstance {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn parameters(&self) -> &[PluginParameterSpec] {
        &self.parameters
    }

    fn latency_frames(&self) -> u32 {
        self.latency
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: &[PluginParamEvent],
    ) -> Result<(), PluginError> {
        debug_assert_eq!(self.input_ptrs.len(), inputs.len());
        debug_assert_eq!(self.output_ptrs.len(), outputs.len());
        for (slot, channel) in self.input_ptrs.iter_mut().zip(inputs) {
            *slot = channel.as_ptr() as *mut f32;
        }
        for (slot, channel) in self.output_ptrs.iter_mut().zip(outputs.iter_mut()) {
            *slot = channel.as_mut_ptr();
        }
        let count = events.len().min(self.in_events.events.len());
        for (slot, event) in self.in_events.events[..count].iter_mut().zip(events) {
            *slot = ClapEventParamValue::new(event.parameter, std::ptr::null_mut(), event.value);
        }
        self.in_events.len = count as u32;

        let process = ClapProcess {
            steady_time: -1,
            frames_count: outputs.first().map_or(0, |c| c.len() as u32),
            transport: std::ptr::null(),
            audio_inputs: &self.input_buffer,
            audio_outputs: &mut self.output_buffer,
            audio_inputs_count: 1,
            audio_outputs_count: 1,
            in_events: &self.in_events.list,
            out_events: &OUT_EVENTS,
        };
        // The plugin is live until Drop (which runs before the module can
        // unload) and the process struct points only at preallocated
        // per-instance storage.
        // SAFETY: processing started; all plumbing is preallocated.
        let status = unsafe { ClapPlugin::process(self.plugin, &process) };
        if status == CLAP_PROCESS_ERROR {
            return Err(PluginError::Process("clap process returned error".into()));
        }
        Ok(())
    }

    fn save_state(&self) -> Result<Option<Vec<u8>>, PluginError> {
        let Some(ext) = self.state_ext else {
            return Ok(None);
        };
        let mut sink = Vec::new();
        let stream = ClapOstream {
            ctx: std::ptr::from_mut(&mut sink).cast(),
            write: ostream_write,
        };
        // The plugin is initialized; the stream points at a local Vec.
        if !(unsafe { ext.save(self.plugin, &stream) }) {
            return Err(PluginError::State("clap_state save returned false".into()));
        }
        Ok(Some(sink))
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
        let Some(ext) = self.state_ext else {
            return Ok(());
        };
        let mut remaining = state;
        let stream = ClapIstream {
            ctx: std::ptr::from_mut(&mut remaining).cast(),
            read: istream_read,
        };
        // The plugin is initialized; the stream points at the local
        // remaining slice.
        if !(unsafe { ext.load(self.plugin, &stream) }) {
            return Err(PluginError::State("clap_state load returned false".into()));
        }
        Ok(())
    }
}

unsafe extern "C" fn ostream_write(
    stream: *const ClapOstream,
    buffer: *const c_void,
    size: u64,
) -> i64 {
    let sink = unsafe { (*stream).ctx as *mut Vec<u8> };
    if sink.is_null() || buffer.is_null() || size > isize::MAX as u64 {
        return -1;
    }
    // SAFETY: the plugin owns `buffer` for the duration of the call; the Vec
    // belongs to save_state's frame.
    let bytes = unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), size as usize) };
    unsafe { (*sink).extend_from_slice(bytes) };
    size as i64
}

unsafe extern "C" fn istream_read(
    stream: *const ClapIstream,
    buffer: *mut c_void,
    size: u64,
) -> i64 {
    let remaining = unsafe { (*stream).ctx as *mut &[u8] };
    if remaining.is_null() || buffer.is_null() {
        return -1;
    }
    // SAFETY: the source slice belongs to load_state's frame and is only
    // advanced here; the destination is plugin-owned memory with at least
    // `size` valid bytes per the stream contract.
    unsafe {
        let source: &[u8] = &*remaining;
        let take = (size as usize).min(source.len());
        if take > 0 {
            std::ptr::copy_nonoverlapping(source.as_ptr(), buffer.cast::<u8>(), take);
            *remaining = &source[take..];
        }
        take as i64
    }
}

impl Drop for ClapInstance {
    fn drop(&mut self) {
        // Construction succeeded, so the plugin exists, is activated and is
        // processing; this is the documented unwind. Fields drop after this
        // body, so the host box outlives destroy (plugins must not touch
        // the host afterwards) and the module is released last.
        // SAFETY: activated, processing plugin; documented unwind.
        unsafe {
            ClapPlugin::stop_processing(self.plugin);
            ClapPlugin::deactivate(self.plugin);
            ClapPlugin::destroy(self.plugin);
        }
    }
}

/// Scan every plugin descriptor exposed by a CLAP module.
pub(crate) fn scan_descriptors(path: &Path) -> Result<Vec<PluginDescriptor>, PluginError> {
    let module = load_module(path)?;
    // SAFETY: the module handle keeps the entry mapped.
    let factory_ptr = unsafe {
        module
            .entry
            .get_factory(CLAP_PLUGIN_FACTORY_ID.as_ptr().cast())
    };
    if factory_ptr.is_null() {
        return Err(PluginError::Entry(
            "clap.plugin-factory not provided by module".into(),
        ));
    }
    // SAFETY: the factory belongs to the module, which stays alive through
    // this function; descriptors are copied out immediately.
    let factory = unsafe { &*(factory_ptr as *const ClapPluginFactory) };
    let mut descriptors = Vec::new();
    for index in 0..factory.plugin_count() {
        let info = factory.plugin_descriptor(index);
        if info.is_null() {
            continue;
        }
        // SAFETY: factory-owned descriptor, copied immediately.
        let owned = unsafe { OwnedDescriptor::read(info) };
        let is_synth = owned.is_instrument();
        descriptors.push(PluginDescriptor {
            format: PluginFormat::Clap,
            path: path.to_path_buf(),
            id: owned.id.clone(),
            name: owned.name.clone(),
            vendor: owned.vendor,
            version: owned.version,
            // The channel counts require instantiation; scanning reports a
            // stereo default and the live descriptor carries the truth.
            input_channels: 2,
            output_channels: 2,
            is_synth,
            has_editor: false,
        });
    }
    Ok(descriptors)
}
