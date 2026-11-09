//! VST3 backend: host COM objects, the instantiation lifecycle, and the
//! realtime instance.
//!
//! Host objects (`IHostApplication`, `IComponentHandler`,
//! `IParameterChanges`/`IParamValueQueue`, `IBStream`) are preallocated with
//! the instance and freed with it; their refcounts never trigger deletion.
//! The parameter-change container is written field-directly before each
//! `process` call (no FFI round trip, no allocation); the plugin only reads
//! it through the vtable during the call.

use std::{
    ffi::c_void,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use libloading::Library;

pub mod abi;

use crate::{
    instance::PluginInstance,
    module::{SharedModule, acquire_module},
    spec::{PluginDescriptor, PluginError, PluginFormat, PluginParamEvent, PluginParameterSpec},
};
use abi::*;

const GET_PLUGIN_FACTORY_SYMBOL: &[u8] = b"GetPluginFactory";
const INIT_DLL_SYMBOL: &[u8] = b"InitDll";
const EXIT_DLL_SYMBOL: &[u8] = b"ExitDll";
/// Changed-parameter events fitting one block; extra diffs are dropped
/// (documented MVP behavior, in line with the engine's event budget).
const MAX_PARAM_EVENTS: usize = 128;
/// Sample-accurate headroom per queue; the host writes one point per block.
const POINTS_PER_QUEUE: usize = 2;

/// Optional module-level `InitDll`/`ExitDll` pair. The SDK's host guide
/// requires `InitDll` before the first `GetPluginFactory` and `ExitDll`
/// after the last module use; plugins that set up process-global state
/// there can refuse to create components when it is skipped. The shared
/// module loader runs this exactly once per actual load and the returned
/// `ExitDll` when the last handle drops.
fn vst3_module_init(library: &Library) -> Result<Option<unsafe extern "C" fn()>, PluginError> {
    // SAFETY: module-level entry points are plain functions on the freshly
    // loaded module; the SDK declares `bool InitDll()` and `void ExitDll()`.
    unsafe {
        if let Ok(init) = library.get::<unsafe extern "C" fn() -> bool>(INIT_DLL_SYMBOL)
            && !init()
        {
            return Err(PluginError::Init("module InitDll failed".into()));
        }
        Ok(library
            .get::<unsafe extern "C" fn()>(EXIT_DLL_SYMBOL)
            .ok()
            .map(|exit| *exit))
    }
}

fn load_factory(path: &Path) -> Result<(Arc<SharedModule>, *mut IPluginFactory), PluginError> {
    let module = acquire_module(path, vst3_module_init)?;
    // SAFETY: the symbol value is copied out; the module handle keeps the
    // library mapped, so the factory stays valid for the module's lifetime.
    let entry = unsafe {
        module
            .library
            .get::<unsafe extern "system" fn() -> *mut IPluginFactory>(GET_PLUGIN_FACTORY_SYMBOL)
    }
    .map_err(|_| PluginError::Entry("GetPluginFactory symbol missing".into()))?;
    // SAFETY: calling the entry on the live module returns the factory.
    let factory = unsafe { (*entry)() };
    if factory.is_null() {
        return Err(PluginError::Entry("GetPluginFactory returned null".into()));
    }
    Ok((module, factory))
}

fn cstr8(raw: &[std::os::raw::c_char]) -> String {
    let bytes = raw.iter().map(|&c| c as u8).collect::<Vec<_>>();
    std::ffi::CStr::from_bytes_until_nul(&bytes)
        .map(|text| text.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn utf16_to_string(text: &String128) -> String {
    let len = text
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(text.len());
    String::from_utf16_lossy(&text[..len])
}

fn cid_to_hex(cid: &TUID) -> String {
    cid.iter().map(|byte| format!("{byte:02X}")).collect()
}

fn hex_to_cid(text: &str) -> Option<TUID> {
    let bytes = text.as_bytes();
    if bytes.len() != 32 || !bytes.iter().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut cid = [0u8; 16];
    for (slot, pair) in cid.iter_mut().zip(bytes.as_chunks::<2>().0) {
        let high = (pair[0] as char).to_digit(16)?;
        let low = (pair[1] as char).to_digit(16)?;
        *slot = ((high << 4) | low) as u8;
    }
    Some(cid)
}

/// Whether an advisory lifecycle call (`setupProcessing`, `setActive`,
/// `setProcessing`) failed. `kNotImplemented` is accepted: the VST3 SDK's
/// own `ComponentBase`/`AudioEffect` base implementations return it for
/// these methods, so it means "the plugin ignores this call", not failure.
/// Real hosts (JUCE ignores the return entirely) continue processing.
fn declined_lifecycle(result: i32) -> bool {
    result != K_RESULT_OK && result != K_NOT_IMPLEMENTED
}

// ---------------------------------------------------------------------------
// Host COM objects
// ---------------------------------------------------------------------------

/// Refcount plumbing: increments never free here; the instance owns every
/// host object and frees it after the plugin's final release. Plugins must
/// release everything they acquire before terminate returns.
fn bump(refs: &AtomicU32) -> u32 {
    refs.fetch_add(1, Ordering::Relaxed).saturating_add(1)
}
fn drop_ref(refs: &AtomicU32) -> u32 {
    refs.fetch_sub(1, Ordering::Relaxed).max(1)
}

#[repr(C)]
struct HostAppObject {
    iface: IHostApplication,
    refs: AtomicU32,
}

static HOST_APP_VTABLE: IHostApplicationVtable = IHostApplicationVtable {
    query_interface: host_app_query_interface,
    add_ref: host_app_add_ref,
    release: host_app_release,
    get_name: host_app_get_name,
    create_instance: host_app_create_instance,
};

unsafe extern "system" fn host_app_query_interface(
    this: *mut c_void,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    if iid.is_null() || obj.is_null() {
        return K_INVALID_ARGUMENT;
    }
    // SAFETY: `this` is a live HostAppObject per the vtable contract; the
    // IID points at 16 readable bytes.
    if unsafe { &*iid.cast::<TUID>() } == &I_HOST_APPLICATION_IID {
        // COM rules: a successful queryInterface yields a new reference.
        // SAFETY: pointer identity holds because `iface` is the first field.
        bump(unsafe { &(*(this as *mut HostAppObject)).refs });
        unsafe { *obj = this };
        K_RESULT_OK
    } else {
        unsafe { *obj = std::ptr::null_mut() };
        K_NO_INTERFACE
    }
}
unsafe extern "system" fn host_app_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: live HostAppObject.
    bump(unsafe { &(*(this as *mut HostAppObject)).refs })
}
unsafe extern "system" fn host_app_release(this: *mut c_void) -> u32 {
    // SAFETY: live HostAppObject; deletion is the owner's job.
    drop_ref(unsafe { &(*(this as *mut HostAppObject)).refs })
}
unsafe extern "system" fn host_app_get_name(
    _this: *mut IHostApplication,
    name: *mut String128,
) -> i32 {
    if name.is_null() {
        return K_INVALID_ARGUMENT;
    }
    for (offset, unit) in "Moiren".encode_utf16().enumerate() {
        // SAFETY: 128 u16 slots of writable memory; 6 written at most.
        unsafe { *(name as *mut TChar).add(offset) = unit };
    }
    // SAFETY: nul terminator within bounds.
    unsafe { *(name as *mut TChar).add("Moiren".encode_utf16().count()) = 0 };
    K_RESULT_OK
}
unsafe extern "system" fn host_app_create_instance(
    _this: *mut IHostApplication,
    _cid: *const u8,
    _iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    if !obj.is_null() {
        // SAFETY: out-pointer is writable per the caller's contract.
        unsafe { *obj = std::ptr::null_mut() };
    }
    K_NOT_IMPLEMENTED
}

#[repr(C)]
struct ComponentHandlerObject {
    iface: IComponentHandler,
    refs: AtomicU32,
}

static COMPONENT_HANDLER_VTABLE: IComponentHandlerVtable = IComponentHandlerVtable {
    query_interface: handler_query_interface,
    add_ref: handler_add_ref,
    release: handler_release,
    begin_edit: handler_noop_id,
    perform_edit: handler_noop_edit,
    end_edit: handler_noop_id,
    restart_component: handler_noop_i32,
};

unsafe extern "system" fn handler_query_interface(
    this: *mut c_void,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    if iid.is_null() || obj.is_null() {
        return K_INVALID_ARGUMENT;
    }
    // SAFETY: live ComponentHandlerObject; IID readable.
    if unsafe { &*iid.cast::<TUID>() } == &I_COMPONENT_HANDLER_IID {
        // SAFETY: first-field pointer identity.
        bump(unsafe { &(*(this as *mut ComponentHandlerObject)).refs });
        unsafe { *obj = this };
        K_RESULT_OK
    } else {
        unsafe { *obj = std::ptr::null_mut() };
        K_NO_INTERFACE
    }
}
unsafe extern "system" fn handler_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: live object.
    bump(unsafe { &(*(this as *mut ComponentHandlerObject)).refs })
}
unsafe extern "system" fn handler_release(this: *mut c_void) -> u32 {
    // SAFETY: live object; owner-managed lifetime.
    drop_ref(unsafe { &(*(this as *mut ComponentHandlerObject)).refs })
}
/// Acknowledge-and-ignore: automation gestures are host-internal here, and
/// restart requests are a documented follow-up.
unsafe extern "system" fn handler_noop_id(_this: *mut IComponentHandler, _id: u32) -> i32 {
    K_RESULT_OK
}
unsafe extern "system" fn handler_noop_i32(_this: *mut IComponentHandler, _flags: i32) -> i32 {
    K_RESULT_OK
}
unsafe extern "system" fn handler_noop_edit(
    _this: *mut IComponentHandler,
    _id: u32,
    _value: ParamValue,
) -> i32 {
    K_RESULT_OK
}

#[repr(C)]
struct QueueObject {
    iface: IParamValueQueue,
    refs: AtomicU32,
    param_id: ParamID,
    offsets: [i32; POINTS_PER_QUEUE],
    values: [f64; POINTS_PER_QUEUE],
    point_count: i32,
}

impl QueueObject {
    fn new() -> Self {
        Self {
            iface: IParamValueQueue {
                vtable: &PARAM_QUEUE_VTABLE,
            },
            refs: AtomicU32::new(1),
            param_id: 0,
            offsets: [0; POINTS_PER_QUEUE],
            values: [0.0; POINTS_PER_QUEUE],
            point_count: 0,
        }
    }
}

#[repr(C)]
struct ParamChangesObject {
    iface: IParameterChanges,
    refs: AtomicU32,
    queues: *mut QueueObject,
    active: i32,
}

static PARAM_CHANGES_VTABLE: IParameterChangesVtable = IParameterChangesVtable {
    query_interface: changes_query_interface,
    add_ref: changes_add_ref,
    release: changes_release,
    get_parameter_count: changes_get_parameter_count,
    get_parameter_data: changes_get_parameter_data,
    add_parameter_data: changes_add_parameter_data,
};

static PARAM_QUEUE_VTABLE: IParamValueQueueVtable = IParamValueQueueVtable {
    query_interface: queue_query_interface,
    add_ref: queue_add_ref,
    release: queue_release,
    get_parameter_id: queue_get_parameter_id,
    get_point_count: queue_get_point_count,
    get_point: queue_get_point,
    add_point: queue_add_point,
};

unsafe extern "system" fn changes_query_interface(
    this: *mut c_void,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    if iid.is_null() || obj.is_null() {
        return K_INVALID_ARGUMENT;
    }
    // SAFETY: live ParamChangesObject; IID readable.
    if unsafe { &*iid.cast::<TUID>() } == &I_PARAM_CHANGES_IID {
        // SAFETY: first-field pointer identity.
        bump(unsafe { &(*(this as *mut ParamChangesObject)).refs });
        unsafe { *obj = this };
        K_RESULT_OK
    } else {
        unsafe { *obj = std::ptr::null_mut() };
        K_NO_INTERFACE
    }
}
unsafe extern "system" fn changes_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: live object.
    bump(unsafe { &(*(this as *mut ParamChangesObject)).refs })
}
unsafe extern "system" fn changes_release(this: *mut c_void) -> u32 {
    // SAFETY: live object; owner-managed lifetime.
    drop_ref(unsafe { &(*(this as *mut ParamChangesObject)).refs })
}
unsafe extern "system" fn changes_get_parameter_count(this: *mut IParameterChanges) -> i32 {
    // SAFETY: live object; `active` is fixed before the process call.
    unsafe { (*(this as *mut ParamChangesObject)).active }
}
unsafe extern "system" fn changes_get_parameter_data(
    this: *mut IParameterChanges,
    index: i32,
) -> *mut IParamValueQueue {
    // SAFETY: live object with the owner's MAX_PARAM_EVENTS allocation.
    let object = unsafe { &*(this as *mut ParamChangesObject) };
    if index < 0 || index >= object.active {
        return std::ptr::null_mut();
    }
    // SAFETY: index is within the active prefix of the queue array.
    unsafe { &mut (*object.queues.add(index as usize)).iface }
}
unsafe extern "system" fn changes_add_parameter_data(
    this: *mut IParameterChanges,
    id: *const ParamID,
    index: *mut i32,
) -> *mut IParamValueQueue {
    // The host fills queues field-directly before process; this entry point
    // exists for spec completeness and stays allocation-free.
    // SAFETY: live object with the owner's queue allocation.
    let object = unsafe { &mut *(this as *mut ParamChangesObject) };
    if id.is_null() || index.is_null() || object.active as usize >= MAX_PARAM_EVENTS {
        return std::ptr::null_mut();
    }
    // SAFETY: the active count bounds the queue slot.
    let slot = unsafe { &mut *object.queues.add(object.active as usize) };
    slot.param_id = unsafe { *id };
    slot.point_count = 0;
    // SAFETY: out-pointer writable per contract.
    unsafe { *index = object.active };
    object.active += 1;
    &mut slot.iface
}

unsafe extern "system" fn queue_query_interface(
    this: *mut c_void,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    if iid.is_null() || obj.is_null() {
        return K_INVALID_ARGUMENT;
    }
    // SAFETY: live QueueObject; IID readable.
    if unsafe { &*iid.cast::<TUID>() } == &I_PARAM_VALUE_QUEUE_IID {
        // SAFETY: first-field pointer identity.
        bump(unsafe { &(*(this as *mut QueueObject)).refs });
        unsafe { *obj = this };
        K_RESULT_OK
    } else {
        unsafe { *obj = std::ptr::null_mut() };
        K_NO_INTERFACE
    }
}
unsafe extern "system" fn queue_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: live object.
    bump(unsafe { &(*(this as *mut QueueObject)).refs })
}
unsafe extern "system" fn queue_release(this: *mut c_void) -> u32 {
    // SAFETY: live object; owner-managed lifetime.
    drop_ref(unsafe { &(*(this as *mut QueueObject)).refs })
}
unsafe extern "system" fn queue_get_parameter_id(this: *mut IParamValueQueue) -> ParamID {
    // SAFETY: live object.
    unsafe { (*(this as *mut QueueObject)).param_id }
}
unsafe extern "system" fn queue_get_point_count(this: *mut IParamValueQueue) -> i32 {
    // SAFETY: live object.
    unsafe { (*(this as *mut QueueObject)).point_count }
}
unsafe extern "system" fn queue_get_point(
    this: *mut IParamValueQueue,
    index: i32,
    sample_offset: *mut i32,
    value: *mut ParamValue,
) -> i32 {
    // SAFETY: live object.
    let queue = unsafe { &*(this as *mut QueueObject) };
    if index < 0 || index >= queue.point_count {
        return K_INVALID_ARGUMENT;
    }
    if !sample_offset.is_null() {
        // SAFETY: out-pointer writable per contract.
        unsafe { *sample_offset = queue.offsets[index as usize] };
    }
    if !value.is_null() {
        // SAFETY: as above.
        unsafe { *value = queue.values[index as usize] };
    }
    K_RESULT_OK
}
unsafe extern "system" fn queue_add_point(
    this: *mut IParamValueQueue,
    sample_offset: i32,
    value: ParamValue,
    index: *mut i32,
) -> i32 {
    // SAFETY: live object.
    let queue = unsafe { &mut *(this as *mut QueueObject) };
    if queue.point_count as usize >= POINTS_PER_QUEUE {
        return K_RESULT_FALSE;
    }
    let slot = queue.point_count as usize;
    queue.offsets[slot] = sample_offset;
    queue.values[slot] = value;
    if !index.is_null() {
        // SAFETY: out-pointer writable per contract.
        unsafe { *index = slot as i32 };
    }
    queue.point_count += 1;
    K_RESULT_OK
}

/// Control-side byte stream: a growable sink for `getState`, a shrinking
/// source for `setState`. Stack-allocated for the duration of one transfer.
/// Serves both `IBStream` (first field) and `ISizeableStream` (second
/// field) from one object, like a C++ class inheriting both.
#[repr(C)]
struct BStreamObject<'a> {
    iface: IBStream,
    sizeable: ISizeableStream,
    refs: AtomicU32,
    target: StreamTarget<'a>,
}

enum StreamTarget<'a> {
    Write(&'a mut Vec<u8>),
    Read { consumed: usize, source: &'a [u8] },
}

/// Seek modes from `IBStream::IStreamSeekMode`.
const IB_SEEK_SET: i32 = 0;
const IB_SEEK_CUR: i32 = 1;
const IB_SEEK_END: i32 = 2;

static B_STREAM_VTABLE: IBStreamVtable = IBStreamVtable {
    query_interface: bstream_query_interface,
    add_ref: bstream_add_ref,
    release: bstream_release,
    read: bstream_read,
    write: bstream_write,
    seek: bstream_seek,
    tell: bstream_tell,
};

static SIZEABLE_STREAM_VTABLE: ISizeableStreamVtable = ISizeableStreamVtable {
    query_interface: sizeable_query_interface,
    add_ref: bstream_add_ref,
    release: bstream_release,
    get_stream_size: sizeable_get_stream_size,
    set_stream_size: sizeable_set_stream_size,
};

unsafe extern "system" fn bstream_query_interface(
    this: *mut c_void,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    if iid.is_null() || obj.is_null() {
        return K_INVALID_ARGUMENT;
    }
    // SAFETY: live BStreamObject; IID readable.
    if unsafe { &*iid.cast::<TUID>() } == &I_B_STREAM_IID {
        // SAFETY: first-field pointer identity.
        bump(unsafe { &(*(this as *mut BStreamObject)).refs });
        unsafe { *obj = this };
        K_RESULT_OK
    } else if unsafe { &*iid.cast::<TUID>() } == &I_SIZEABLE_STREAM_IID {
        // SAFETY: the sizeable field is at a fixed offset; both interface
        // pointers recover the same object.
        let object = unsafe { &mut *(this as *mut BStreamObject) };
        bump(&object.refs);
        unsafe { *obj = &mut object.sizeable as *mut ISizeableStream as *mut c_void };
        K_RESULT_OK
    } else {
        unsafe { *obj = std::ptr::null_mut() };
        K_NO_INTERFACE
    }
}
/// QI entry of the sizeable interface; same object, offset-adjusted `this`.
unsafe extern "system" fn sizeable_query_interface(
    this: *mut c_void,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    // SAFETY: the sizeable pointer is at a fixed offset inside BStreamObject.
    let base = (this as *mut u8)
        .wrapping_sub(std::mem::offset_of!(BStreamObject, sizeable))
        .cast::<BStreamObject>();
    // SAFETY: base is the object's first interface, whose QI covers all IIDs.
    unsafe { bstream_query_interface(base.cast(), iid, obj) }
}
unsafe extern "system" fn bstream_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: live object.
    bump(unsafe { &(*(this as *mut BStreamObject)).refs })
}
unsafe extern "system" fn bstream_release(this: *mut c_void) -> u32 {
    // SAFETY: live object; stack-allocated by the state transfer.
    drop_ref(unsafe { &(*(this as *mut BStreamObject)).refs })
}
unsafe extern "system" fn bstream_read(
    this: *mut IBStream,
    buffer: *mut c_void,
    num_bytes: i32,
    num_bytes_read: *mut i32,
) -> i32 {
    // SAFETY: live object; the enum tag selects the read side.
    let object = unsafe { &mut *(this as *mut BStreamObject) };
    let StreamTarget::Read { consumed, source } = &mut object.target else {
        return K_NOT_IMPLEMENTED;
    };
    let take = (num_bytes.max(0) as usize).min(source.len() - *consumed);
    if take > 0 && !buffer.is_null() {
        // SAFETY: plugin-owned destination with num_bytes capacity.
        unsafe {
            std::ptr::copy_nonoverlapping(
                source.as_ptr().add(*consumed),
                buffer.cast::<u8>(),
                take,
            );
        };
    }
    *consumed += take;
    if !num_bytes_read.is_null() {
        // SAFETY: out-pointer writable per contract.
        unsafe { *num_bytes_read = take as i32 };
    }
    K_RESULT_OK
}
unsafe extern "system" fn bstream_write(
    this: *mut IBStream,
    buffer: *mut c_void,
    num_bytes: i32,
    num_bytes_written: *mut i32,
) -> i32 {
    // SAFETY: live object; the enum tag selects the write side.
    let object = unsafe { &mut *(this as *mut BStreamObject) };
    let StreamTarget::Write(sink) = &mut object.target else {
        return K_NOT_IMPLEMENTED;
    };
    let bytes: &[u8] = if buffer.is_null() || num_bytes <= 0 {
        &[]
    } else {
        // SAFETY: num_bytes of readable plugin memory per the contract.
        unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), num_bytes as usize) }
    };
    sink.extend_from_slice(bytes);
    if !num_bytes_written.is_null() {
        // SAFETY: out-pointer writable per contract.
        unsafe { *num_bytes_written = bytes.len() as i32 };
    }
    K_RESULT_OK
}
/// Seek. `kResultTrue` equals `kResultOk` (0) in the official enum, so
/// successful seeks answer `K_RESULT_OK`: JUCE's setState bails out through
/// its seek-to-beginning guard unless the seek reports exactly that.
unsafe extern "system" fn bstream_seek(
    this: *mut IBStream,
    pos: i64,
    mode: i32,
    result: *mut i64,
) -> i32 {
    // SAFETY: live object.
    let object = unsafe { &mut *(this as *mut BStreamObject) };
    let outcome = match &mut object.target {
        StreamTarget::Read { consumed, source } => {
            let length = source.len() as i64;
            let target = match mode {
                IB_SEEK_SET => Some(pos),
                IB_SEEK_CUR => i64::try_from(*consumed)
                    .ok()
                    .and_then(|c| c.checked_add(pos)),
                IB_SEEK_END => length.checked_add(pos),
                _ => None,
            };
            let Some(target) = target else {
                return K_INVALID_ARGUMENT;
            };
            if target < 0 || target > length {
                return K_INVALID_ARGUMENT;
            }
            *consumed = target as usize;
            target
        }
        StreamTarget::Write(sink) => {
            if mode != IB_SEEK_SET {
                return K_NOT_IMPLEMENTED;
            }
            let target = pos.clamp(0, i64::try_from(sink.len()).unwrap_or(i64::MAX));
            sink.truncate(target as usize);
            target
        }
    };
    if !result.is_null() {
        // SAFETY: out-pointer writable per contract.
        unsafe { *result = outcome };
    }
    K_RESULT_OK
}
unsafe extern "system" fn bstream_tell(this: *mut IBStream, pos: *mut i64) -> i32 {
    // SAFETY: live object.
    let object = unsafe { &*(this as *mut BStreamObject) };
    let position = match &object.target {
        StreamTarget::Read { consumed, .. } => *consumed as i64,
        StreamTarget::Write(sink) => sink.len() as i64,
    };
    if !pos.is_null() {
        // SAFETY: out-pointer writable per contract.
        unsafe { *pos = position };
    }
    K_RESULT_OK
}
/// JUCE's setState prefers a sized read (readFromMemoryStream) when the
/// stream offers ISizeableStream; serving the size spares every client the
/// probe-and-collect fallback.
unsafe extern "system" fn sizeable_get_stream_size(
    this: *mut ISizeableStream,
    size: *mut i64,
) -> i32 {
    // SAFETY: the sizeable pointer is at a fixed offset inside BStreamObject.
    let object = unsafe {
        &*((this as *mut u8)
            .wrapping_sub(std::mem::offset_of!(BStreamObject, sizeable))
            .cast::<BStreamObject>())
    };
    let length = match &object.target {
        StreamTarget::Read { source, .. } => source.len() as i64,
        StreamTarget::Write(sink) => sink.len() as i64,
    };
    if size.is_null() {
        return K_INVALID_ARGUMENT;
    }
    // SAFETY: out-pointer writable per contract.
    unsafe { *size = length };
    K_RESULT_OK
}
unsafe extern "system" fn sizeable_set_stream_size(_this: *mut ISizeableStream, _size: i64) -> i32 {
    K_NOT_IMPLEMENTED
}

impl<'a> BStreamObject<'a> {
    fn writer(sink: &'a mut Vec<u8>) -> Self {
        Self {
            iface: IBStream {
                vtable: &B_STREAM_VTABLE,
            },
            sizeable: ISizeableStream {
                vtable: &SIZEABLE_STREAM_VTABLE,
            },
            refs: AtomicU32::new(1),
            target: StreamTarget::Write(sink),
        }
    }

    fn reader(source: &'a [u8]) -> Self {
        Self {
            iface: IBStream {
                vtable: &B_STREAM_VTABLE,
            },
            sizeable: ISizeableStream {
                vtable: &SIZEABLE_STREAM_VTABLE,
            },
            refs: AtomicU32::new(1),
            target: StreamTarget::Read {
                consumed: 0,
                source,
            },
        }
    }

    fn stream_mut(&mut self) -> *mut IBStream {
        &mut self.iface
    }
}

// ---------------------------------------------------------------------------
// Instance
// ---------------------------------------------------------------------------

pub(crate) struct Vst3Instance {
    descriptor: PluginDescriptor,
    parameters: Vec<PluginParameterSpec>,
    /// Keeps the library mapped through teardown; never dereferenced.
    _module: Arc<SharedModule>,
    component: *mut IComponent,
    audio: *mut IAudioProcessor,
    /// Null when the plugin exposes no controller.
    controller: *mut IEditController,
    /// True when the controller is a separate object (needs its own
    /// initialize/terminate); false when it is the component itself.
    controller_is_separate: bool,
    host: *mut HostAppObject,
    handler: *mut ComponentHandlerObject,
    /// Connection-point pair linking the component to a separate
    /// controller; null when either side lacks IConnectionPoint. Each
    /// pointer holds one reference, released in Drop after disconnect.
    component_connection: *mut IConnectionPoint,
    controller_connection: *mut IConnectionPoint,
    changes: *mut ParamChangesObject,
    /// The process call's mandatory outputParameterChanges sink; the plugin
    /// fills it via addParameterData and the host only resets its length.
    out_changes: *mut ParamChangesObject,
    /// Owns the queue array `changes.queues` points into; drops after the
    /// Drop body, so the queues outlive every plugin call. Never read.
    _queues_storage: Box<[QueueObject]>,
    /// Owns the queue array `out_changes.queues` points into. Never read.
    _out_queues_storage: Box<[QueueObject]>,
    input_ptrs: Box<[*mut f32]>,
    output_ptrs: Box<[*mut f32]>,
    input_buses: Box<[AudioBusBuffers]>,
    output_buses: Box<[AudioBusBuffers]>,
    latency: u32,
}

// Single-thread-at-a-time use on any thread, like every format backend; all
// pointers belong to the plugin, module, or owner-side boxes below, each
// outliving the instance's last use.
unsafe impl Send for Vst3Instance {}

/// Release every plugin pointer acquired by [`instantiate`], in reverse
/// acquisition order. SAFETY: each pointer must be live exactly once.
unsafe fn release_plugin_objects(
    controller: *mut IEditController,
    audio: *mut IAudioProcessor,
    component: *mut IComponent,
) {
    unsafe {
        if !controller.is_null() {
            IEditController::release(controller);
        }
        if !audio.is_null() {
            IAudioProcessor::release(audio);
        }
        if !component.is_null() {
            IComponent::release(component);
        }
    }
}

/// Free the host-side boxes. SAFETY: each pointer must come from
/// Box::into_raw and be unused afterwards.
unsafe fn free_host_objects(
    host: *mut HostAppObject,
    handler: *mut ComponentHandlerObject,
    changes: *mut ParamChangesObject,
    out_changes: *mut ParamChangesObject,
) {
    unsafe {
        if !host.is_null() {
            drop(Box::from_raw(host));
        }
        if !handler.is_null() {
            drop(Box::from_raw(handler));
        }
        if !changes.is_null() {
            drop(Box::from_raw(changes));
        }
        if !out_changes.is_null() {
            drop(Box::from_raw(out_changes));
        }
    }
}

/// Main audio bus `(index, channels)` for one direction.
/// SAFETY: `component` must be initialized.
unsafe fn main_audio_bus(component: *mut IComponent, direction: i32) -> Option<(i32, i32)> {
    let count = unsafe { IComponent::get_bus_count(component, MEDIA_TYPE_AUDIO, direction) };
    let mut info: BusInfo = unsafe { std::mem::zeroed() };
    for index in 0..count {
        // SAFETY: live component; writable info buffer.
        if unsafe {
            IComponent::get_bus_info(component, MEDIA_TYPE_AUDIO, direction, index, &mut info)
        } == K_RESULT_OK
            && info.bus_type == BUS_TYPE_MAIN
        {
            return Some((index, info.channel_count));
        }
    }
    None
}

pub(crate) fn instantiate(
    source: &PluginDescriptor,
    sample_rate: f64,
    max_block_frames: u32,
) -> Result<Vst3Instance, PluginError> {
    let (module, factory) = load_factory(&source.path)?;
    let Some(cid) = hex_to_cid(&source.id) else {
        return Err(PluginError::Entry(format!(
            "descriptor id {:?} is not a 32-digit class CID",
            source.id
        )));
    };

    let mut component_obj: *mut c_void = std::ptr::null_mut();
    // SAFETY: live factory; the CID comes from this factory's class table.
    let create_result = unsafe {
        IPluginFactory::create_instance(factory, &cid, &I_COMPONENT_IID, &mut component_obj)
    };
    if create_result != K_RESULT_OK || component_obj.is_null() {
        return Err(PluginError::Init(format!(
            "factory refused to create the component (tresult 0x{create_result:08X}, object null: {})",
            component_obj.is_null()
        )));
    }
    let component = component_obj as *mut IComponent;
    // SAFETY: live component; the IID matches the requested interface.
    let audio = unsafe {
        IComponent::query_interface_as::<IAudioProcessor>(component, &I_AUDIO_PROCESSOR_IID)
    };
    if audio.is_null() {
        // SAFETY: only the component reference was acquired so far.
        unsafe { IComponent::release(component) };
        return Err(PluginError::Init("component has no IAudioProcessor".into()));
    }

    let host = Box::into_raw(Box::new(HostAppObject {
        iface: IHostApplication {
            vtable: &HOST_APP_VTABLE,
        },
        refs: AtomicU32::new(1),
    }));
    let handler = Box::into_raw(Box::new(ComponentHandlerObject {
        iface: IComponentHandler {
            vtable: &COMPONENT_HANDLER_VTABLE,
        },
        refs: AtomicU32::new(1),
    }));

    // Every failure below releases the plugin pointers and host boxes.
    let unwind = |audio: *mut IAudioProcessor,
                  component: *mut IComponent,
                  controller: *mut IEditController,
                  component_connection: *mut IConnectionPoint,
                  controller_connection: *mut IConnectionPoint,
                  error: PluginError|
     -> PluginError {
        // SAFETY: all pointers were acquired above and are live; the RT
        // plumbing (changes boxes) is allocated only after these checks.
        unsafe {
            release_plugin_objects(controller, audio, component);
            if !component_connection.is_null() {
                IConnectionPoint::release(component_connection);
            }
            if !controller_connection.is_null() {
                IConnectionPoint::release(controller_connection);
            }
            free_host_objects(host, handler, std::ptr::null_mut(), std::ptr::null_mut());
        }
        error
    };

    // SAFETY: live component; the host context object is our application.
    // The SDK lifecycle initializes the component before the controller is
    // discovered.
    if unsafe { IComponent::initialize(component, host as *mut c_void) } != K_RESULT_OK {
        return Err(unwind(
            audio,
            component,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            PluginError::Init("IComponent::initialize failed".into()),
        ));
    }

    // Controller: a separate class when the component names one, otherwise
    // the component's own IEditController interface.
    let mut controller: *mut IEditController = std::ptr::null_mut();
    let mut controller_is_separate = false;
    let mut controller_cid = [0u8; 16];
    // SAFETY: live component; writable CID buffer.
    if unsafe { IComponent::get_controller_class_id(component, &mut controller_cid) } == K_RESULT_OK
    {
        let mut controller_obj: *mut c_void = std::ptr::null_mut();
        // SAFETY: live factory; the CID came from the component.
        if unsafe {
            IPluginFactory::create_instance(
                factory,
                &controller_cid,
                &I_EDIT_CONTROLLER_IID,
                &mut controller_obj,
            )
        } == K_RESULT_OK
            && !controller_obj.is_null()
        {
            controller = controller_obj as *mut IEditController;
            controller_is_separate = true;
        }
    }
    if controller.is_null() {
        // SAFETY: live component.
        controller = unsafe {
            IComponent::query_interface_as::<IEditController>(component, &I_EDIT_CONTROLLER_IID)
        };
    }
    if controller_is_separate {
        // SAFETY: live separate controller; same host context.
        if unsafe { IEditController::initialize(controller, host as *mut c_void) } != K_RESULT_OK {
            return Err(unwind(
                audio,
                component,
                controller,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                PluginError::Init("IEditController::initialize failed".into()),
            ));
        }
    }

    // Pair a separate controller with its component: JUCE's controller
    // binds its processor through IConnectionPoint and reports zero
    // parameters until both sides are connected. Each QI'd pointer keeps
    // one reference, released in Drop after the disconnect.
    let mut component_connection: *mut IConnectionPoint = std::ptr::null_mut();
    let mut controller_connection: *mut IConnectionPoint = std::ptr::null_mut();
    if controller_is_separate {
        // SAFETY: live initialized objects; the IIDs match the requests.
        component_connection = unsafe {
            IComponent::query_interface_as::<IConnectionPoint>(component, &I_CONNECTION_POINT_IID)
        };
        let mut controller_conn_obj: *mut c_void = std::ptr::null_mut();
        if unsafe {
            IEditController::query_interface(
                controller,
                &I_CONNECTION_POINT_IID,
                &mut controller_conn_obj,
            )
        } == K_RESULT_OK
            && !controller_conn_obj.is_null()
        {
            controller_connection = controller_conn_obj as *mut IConnectionPoint;
        }
        if component_connection.is_null() || controller_connection.is_null() {
            // One side has no connection point: there is nothing to pair.
            // SAFETY: drop whichever reference was acquired.
            unsafe {
                if !component_connection.is_null() {
                    IConnectionPoint::release(component_connection);
                }
                if !controller_connection.is_null() {
                    IConnectionPoint::release(controller_connection);
                }
            }
            component_connection = std::ptr::null_mut();
            controller_connection = std::ptr::null_mut();
        } else {
            // SAFETY: both pointers are live connection points.
            unsafe {
                IConnectionPoint::connect(component_connection, controller_connection);
                IConnectionPoint::connect(controller_connection, component_connection);
            }
        }
    }

    if !controller.is_null() {
        // SAFETY: live controller; the handler object is host-owned.
        unsafe { IEditController::set_component_handler(controller, handler as *mut c_void) };
    }

    // SAFETY: live processor; 32-bit floats are this host's only precision.
    if unsafe { IAudioProcessor::can_process_sample_size(audio, SYMBOLIC_SAMPLE_SIZE_32) }
        != K_RESULT_OK
    {
        return Err(unwind(
            audio,
            component,
            controller,
            component_connection,
            controller_connection,
            PluginError::Init("plugin refuses 32-bit processing".into()),
        ));
    }

    // Main buses: activate the main pair, silence every other audio bus.
    // SAFETY: initialized component throughout.
    let input = unsafe { main_audio_bus(component, BUS_DIRECTION_INPUT) };
    let output = unsafe { main_audio_bus(component, BUS_DIRECTION_OUTPUT) };
    let (input_channels, output_channels) = match (input, output) {
        (Some((_, in_channels)), Some((_, out_channels))) => (in_channels, out_channels),
        (None, Some((_, out_channels))) => (0, out_channels),
        (Some((_, in_channels)), None) => (in_channels, 0),
        (None, None) => (0, 0),
    };
    if input_channels <= 0 || input_channels != output_channels {
        return Err(unwind(
            audio,
            component,
            controller,
            component_connection,
            controller_connection,
            PluginError::NoAudioPorts {
                input: input_channels.max(0) as u32,
                output: output_channels.max(0) as u32,
            },
        ));
    }
    for direction in [BUS_DIRECTION_INPUT, BUS_DIRECTION_OUTPUT] {
        // SAFETY: initialized component; activation states are advisory.
        let count = unsafe { IComponent::get_bus_count(component, MEDIA_TYPE_AUDIO, direction) };
        for index in 0..count {
            // SAFETY: as above.
            unsafe {
                IComponent::activate_bus(
                    component,
                    MEDIA_TYPE_AUDIO,
                    direction,
                    index,
                    u8::from(direction_is_main(component, direction, index)),
                )
            };
        }
    }

    let mut setup = ProcessSetup {
        process_mode: PROCESS_MODE_REALTIME,
        symbolic_sample_size: SYMBOLIC_SAMPLE_SIZE_32,
        max_samples_per_block: max_block_frames.max(1) as i32,
        sample_rate,
    };
    // SAFETY: live processor; the setup matches the host's engine config.
    if declined_lifecycle(unsafe { IAudioProcessor::setup_processing(audio, &mut setup) }) {
        return Err(unwind(
            audio,
            component,
            controller,
            component_connection,
            controller_connection,
            PluginError::Init("setupProcessing failed".into()),
        ));
    }
    // SAFETY: live component/processor.
    unsafe {
        if declined_lifecycle(IComponent::set_active(component, 1)) {
            return Err(unwind(
                audio,
                component,
                controller,
                component_connection,
                controller_connection,
                PluginError::Init("setActive(true) failed".into()),
            ));
        }
        if declined_lifecycle(IAudioProcessor::set_processing(audio, 1)) {
            return Err(unwind(
                audio,
                component,
                controller,
                component_connection,
                controller_connection,
                PluginError::Init("setProcessing(true) failed".into()),
            ));
        }
    }

    // SAFETY: live controller (or none); writable info buffer per call.
    let parameters = if controller.is_null() {
        Vec::new()
    } else {
        let count = unsafe { IEditController::get_parameter_count(controller) }.clamp(0, 4096);
        let mut specs = Vec::with_capacity(count as usize);
        for index in 0..count {
            let mut info: ParameterInfo = unsafe { std::mem::zeroed() };
            if unsafe { IEditController::get_parameter_info(controller, index, &mut info) }
                != K_RESULT_OK
            {
                continue;
            }
            if info.flags & (PARAM_IS_READ_ONLY | PARAM_IS_HIDDEN) != 0 {
                continue;
            }
            specs.push(PluginParameterSpec {
                id: info.id,
                name: utf16_to_string(&info.title),
                min: 0.0,
                max: 1.0,
                default: info.default_normalized_value.clamp(0.0, 1.0),
                automatable: info.flags & PARAM_CAN_AUTOMATE != 0,
                stepped: info.step_count > 0,
            });
        }
        specs
    };
    // SAFETY: live processor.
    let latency = unsafe { IAudioProcessor::get_latency_samples(audio) };

    let channels = output_channels as usize;
    let mut input_ptrs = vec![std::ptr::null_mut::<f32>(); channels].into_boxed_slice();
    let output_ptrs = vec![std::ptr::null_mut::<f32>(); channels].into_boxed_slice();
    // Box allocations never move, so the buffers' pointer tables stay valid
    // across the struct's moves; both bus buffers carry the same width.
    let input_buses = vec![AudioBusBuffers {
        num_channels: output_channels,
        silence_flags: 0,
        channel_buffers32: input_ptrs.as_mut_ptr(),
    }]
    .into_boxed_slice();
    let output_buses = vec![AudioBusBuffers {
        num_channels: output_channels,
        silence_flags: 0,
        channel_buffers32: output_ptrs.as_ptr() as *mut *mut f32,
    }]
    .into_boxed_slice();
    let mut queues_storage: Box<[QueueObject]> = (0..MAX_PARAM_EVENTS)
        .map(|_| QueueObject::new())
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let queues = queues_storage.as_mut_ptr();
    let changes = Box::into_raw(Box::new(ParamChangesObject {
        iface: IParameterChanges {
            vtable: &PARAM_CHANGES_VTABLE,
        },
        refs: AtomicU32::new(1),
        queues,
        active: 0,
    }));
    let mut out_queues_storage: Box<[QueueObject]> = (0..MAX_PARAM_EVENTS)
        .map(|_| QueueObject::new())
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let out_queues = out_queues_storage.as_mut_ptr();
    let out_changes = Box::into_raw(Box::new(ParamChangesObject {
        iface: IParameterChanges {
            vtable: &PARAM_CHANGES_VTABLE,
        },
        refs: AtomicU32::new(1),
        queues: out_queues,
        active: 0,
    }));

    let descriptor = PluginDescriptor {
        format: PluginFormat::Vst3,
        path: source.path.clone(),
        id: source.id.clone(),
        name: source.name.clone(),
        vendor: source.vendor.clone(),
        version: source.version.clone(),
        input_channels: input_channels as u32,
        output_channels: output_channels as u32,
        is_synth: false,
        has_editor: false,
    };

    Ok(Vst3Instance {
        descriptor,
        parameters,
        _module: module,
        component,
        audio,
        controller,
        controller_is_separate,
        host,
        handler,
        component_connection,
        controller_connection,
        changes,
        out_changes,
        _queues_storage: queues_storage,
        _out_queues_storage: out_queues_storage,
        input_ptrs,
        output_ptrs,
        input_buses,
        output_buses,
        latency,
    })
}

/// SAFETY: initialized component; `(direction, index)` within the bus table.
unsafe fn direction_is_main(component: *mut IComponent, direction: i32, index: i32) -> bool {
    let mut info: BusInfo = unsafe { std::mem::zeroed() };
    // SAFETY: live component; writable buffer.
    let answer = unsafe {
        IComponent::get_bus_info(component, MEDIA_TYPE_AUDIO, direction, index, &mut info)
    };
    answer == K_RESULT_OK && info.bus_type == BUS_TYPE_MAIN
}

impl PluginInstance for Vst3Instance {
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
        // Fill the parameter-change container field-directly: no FFI round
        // trip, no allocation, and the plugin reads it through the vtable.
        // SAFETY: both boxes are owned by this instance and stay live for
        // the whole process call.
        let changes = unsafe { &mut *self.changes };
        let active = events.len().min(MAX_PARAM_EVENTS) as i32;
        for (slot, event) in (0..active).zip(events) {
            // SAFETY: slot < MAX_PARAM_EVENTS bounds the queues allocation.
            let queue = unsafe { &mut *changes.queues.add(slot as usize) };
            queue.param_id = event.parameter;
            queue.offsets[0] = 0;
            queue.values[0] = event.value;
            queue.point_count = 1;
        }
        changes.active = active;
        // The output container is plugin-written via addParameterData; reset
        // it per block so queues never carry over between process calls.
        // SAFETY: the box is owned by this instance and stays live for the
        // whole process call.
        let out_changes = unsafe { &mut *self.out_changes };
        out_changes.active = 0;

        let mut data = ProcessData {
            process_mode: PROCESS_MODE_REALTIME,
            symbolic_sample_size: SYMBOLIC_SAMPLE_SIZE_32,
            num_samples: outputs.first().map_or(0, |c| c.len()) as i32,
            num_inputs: 1,
            num_outputs: 1,
            inputs: self.input_buses.as_mut_ptr(),
            outputs: self.output_buses.as_mut_ptr(),
            input_parameter_changes: &mut changes.iface as *mut IParameterChanges,
            output_parameter_changes: &mut out_changes.iface as *mut IParameterChanges,
            input_events: std::ptr::null_mut(),
            output_events: std::ptr::null_mut(),
            process_context: std::ptr::null_mut(),
        };
        // SAFETY: live processor; all buffers and host objects above are
        // valid for the duration of the call.
        let result = unsafe { IAudioProcessor::process(self.audio, &mut data) };
        if result != K_RESULT_OK && result != K_RESULT_FALSE {
            return Err(PluginError::Process(format!(
                "IAudioProcessor::process returned 0x{result:08X}"
            )));
        }
        Ok(())
    }

    fn save_state(&self) -> Result<Option<Vec<u8>>, PluginError> {
        let mut component_bytes = Vec::new();
        {
            let mut stream = BStreamObject::writer(&mut component_bytes);
            // SAFETY: live component; stream valid for this call.
            if unsafe { IComponent::get_state(self.component, stream.stream_mut()) } != K_RESULT_OK
            {
                return Err(PluginError::State("component getState failed".into()));
            }
        }
        let mut controller_bytes = Vec::new();
        if !self.controller.is_null() {
            let mut stream = BStreamObject::writer(&mut controller_bytes);
            // SAFETY: live controller; stream valid for this call.
            let result =
                unsafe { IEditController::get_state(self.controller, stream.stream_mut()) };
            if result != K_RESULT_OK {
                // Single-component plugins keep all state in IComponent and
                // answer kNotImplemented/kResultFalse here (e.g. every DPF
                // build); that means "no controller segment", not an error.
                if result != K_NOT_IMPLEMENTED && result != K_RESULT_FALSE {
                    return Err(PluginError::State(format!(
                        "controller getState returned 0x{result:08X}"
                    )));
                }
                controller_bytes.clear();
            }
        }
        let mut blob = Vec::with_capacity(8 + component_bytes.len() + controller_bytes.len());
        blob.extend_from_slice(&(component_bytes.len() as u32).to_le_bytes());
        blob.extend_from_slice(&component_bytes);
        blob.extend_from_slice(&(controller_bytes.len() as u32).to_le_bytes());
        blob.extend_from_slice(&controller_bytes);
        Ok(Some(blob))
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
        let (component_bytes, rest) = split_segment(state)?;
        let (controller_bytes, _) = split_segment(rest)?;
        {
            let mut stream = BStreamObject::reader(component_bytes);
            // SAFETY: live component; stream valid for this call.
            if unsafe { IComponent::set_state(self.component, stream.stream_mut()) } != K_RESULT_OK
            {
                return Err(PluginError::State("component setState failed".into()));
            }
        }
        if !self.controller.is_null() && !component_bytes.is_empty() {
            let mut stream = BStreamObject::reader(component_bytes);
            // SAFETY: live controller; stream valid for this call.
            let result = unsafe {
                IEditController::set_component_state(self.controller, stream.stream_mut())
            };
            // A separate controller may decline the component view of the
            // state; the component state itself is already authoritative.
            if result != K_RESULT_OK && result != K_NOT_IMPLEMENTED && result != K_RESULT_FALSE {
                return Err(PluginError::State(format!(
                    "controller setComponentState returned 0x{result:08X}"
                )));
            }
        }
        if !self.controller.is_null() && !controller_bytes.is_empty() {
            let mut stream = BStreamObject::reader(controller_bytes);
            // SAFETY: live controller; stream valid for this call.
            if unsafe { IEditController::set_state(self.controller, stream.stream_mut()) }
                != K_RESULT_OK
            {
                return Err(PluginError::State("controller setState failed".into()));
            }
        }
        Ok(())
    }
}

impl Drop for Vst3Instance {
    fn drop(&mut self) {
        // SAFETY: construction completed, so every pointer is live; the
        // documented shutdown order runs before any release.
        unsafe {
            IAudioProcessor::set_processing(self.audio, 0);
            IComponent::set_active(self.component, 0);
            if self.controller_is_separate {
                if !self.component_connection.is_null() && !self.controller_connection.is_null() {
                    // SAFETY: both sides are still live connection points.
                    IConnectionPoint::disconnect(
                        self.controller_connection,
                        self.component_connection,
                    );
                    IConnectionPoint::disconnect(
                        self.component_connection,
                        self.controller_connection,
                    );
                }
                if !self.component_connection.is_null() {
                    IConnectionPoint::release(self.component_connection);
                }
                if !self.controller_connection.is_null() {
                    IConnectionPoint::release(self.controller_connection);
                }
                IEditController::terminate(self.controller);
            }
            IComponent::terminate(self.component);
            release_plugin_objects(self.controller, self.audio, self.component);
            free_host_objects(self.host, self.handler, self.changes, self.out_changes);
        }
    }
}

/// Split one `[u32 len][bytes]` framing segment off the front of `blob`,
/// returning `(segment, remainder)`.
fn split_segment(blob: &[u8]) -> Result<(&[u8], &[u8]), PluginError> {
    let length = blob
        .get(..4)
        .map(|prefix| u32::from_le_bytes(prefix.try_into().expect("four bytes")) as usize)
        .ok_or_else(|| PluginError::State("state blob truncated".into()))?;
    blob.get(4..4 + length)
        .map(|segment| (segment, &blob[4 + length..]))
        .ok_or_else(|| PluginError::State("state blob segment overruns".into()))
}

/// Scan every Audio Module Class in a VST3 binary (or bundle binary).
pub(crate) fn scan_descriptors(path: &Path) -> Result<Vec<PluginDescriptor>, PluginError> {
    let (_module, factory) = load_factory(path)?;
    // SAFETY: live factory; the upgrade is optional.
    let factory2 = unsafe { IPluginFactory::query_factory2(factory) };
    // SAFETY: live factory.
    let count = unsafe { IPluginFactory::count_classes(factory) };
    let mut descriptors = Vec::new();
    for index in 0..count {
        let mut info: PClassInfo = unsafe { std::mem::zeroed() };
        // SAFETY: live factory; writable buffer.
        if unsafe { IPluginFactory::get_class_info(factory, index, &mut info) } != K_RESULT_OK {
            continue;
        }
        if cstr8(&info.category) != VST_AUDIO_EFFECT_CLASS {
            continue;
        }
        let mut vendor = String::new();
        let mut version = String::new();
        let mut is_synth = false;
        if !factory2.is_null() {
            let mut info2: PClassInfo2 = unsafe { std::mem::zeroed() };
            // SAFETY: live factory2; writable buffer.
            if unsafe { IPluginFactory2::get_class_info2(factory2, index, &mut info2) }
                == K_RESULT_OK
            {
                vendor = cstr8(&info2.vendor);
                version = cstr8(&info2.version);
                let subcategories = cstr8(&info2.sub_categories);
                is_synth = subcategories
                    .split('|')
                    .any(|s| s == VST_INSTRUMENT_SUBCATEGORY);
            }
        }
        descriptors.push(PluginDescriptor {
            format: PluginFormat::Vst3,
            path: path.to_path_buf(),
            id: cid_to_hex(&info.cid),
            name: cstr8(&info.name),
            vendor,
            version,
            // Bus layouts need an instantiated component; scanning reports
            // a stereo default and the live descriptor carries the truth.
            input_channels: 2,
            output_channels: 2,
            is_synth,
            has_editor: false,
        });
    }
    // SAFETY: the factory2 pointer, when non-null, holds one reference.
    if !factory2.is_null() {
        unsafe { IPluginFactory2::release(factory2) };
    }
    Ok(descriptors)
}
