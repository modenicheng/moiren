//! VST3 C ABI declarations, transcribed from the official SDK interface
//! headers (<https://github.com/steinbergmedia/vst3sdk>, pluginterfaces).
//!
//! COM layout: every interface object starts with a vtable pointer. Each
//! interface is mirrored as an opaque object struct whose first (private)
//! field is that pointer, plus typed methods that dispatch through the
//! vtable; the vtable structs list fn pointers in exact C++ declaration
//! order (FUnknown's queryInterface/addRef/release first, then inherited
//! interfaces' methods, then the interface's own). UIDs use the
//! COM-compatible `INLINE_UID` byte order, which equals the Windows GUID
//! memory layout; [`inline_uid`] reproduces it.

use std::os::raw::{c_char, c_void};

pub type TChar = u16;
/// 128-character UTF-16 string (`String128`).
pub type String128 = [TChar; 128];
/// 16-byte interface/class identifier in GUID memory layout.
pub type TUID = [u8; 16];

pub type TBool = u8;
pub type ParamID = u32;
/// Normalized parameter value in [0, 1].
pub type ParamValue = f64;

// Result codes (Windows/COM-compatible values).
pub const K_RESULT_OK: i32 = 0;
pub const K_RESULT_FALSE: i32 = 1;
pub const K_NO_INTERFACE: i32 = 0x8000_4002u32 as i32;
pub const K_INVALID_ARGUMENT: i32 = 0x8007_0057u32 as i32;
pub const K_NOT_IMPLEMENTED: i32 = 0x8000_4001u32 as i32;
pub const K_INTERNAL_ERROR: i32 = 0x8000_4005u32 as i32;

/// COM-compatible `INLINE_UID` byte order (matches the Windows GUID memory
/// layout): l1 little-endian, l2 split big/little, l3 and l4 big-endian.
pub const fn inline_uid(l1: u32, l2: u32, l3: u32, l4: u32) -> TUID {
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

pub const I_PLUGIN_BASE_IID: TUID = inline_uid(0x2288_8DDB, 0x156E_45AE, 0x8358_B348, 0x0819_0625);
pub const I_PLUGIN_FACTORY_IID: TUID =
    inline_uid(0x7A4D_811C, 0x5211_4A1F, 0xAED9_D2EE, 0x0B43_BF9F);
pub const I_PLUGIN_FACTORY2_IID: TUID =
    inline_uid(0x0007_B650, 0xF24B_4C0B, 0xA464_EDB9, 0xF00B_2ABB);
pub const I_COMPONENT_IID: TUID = inline_uid(0xE831_FF31, 0xF2D5_4301, 0x928E_BBEE, 0x2569_7802);
pub const I_AUDIO_PROCESSOR_IID: TUID =
    inline_uid(0x4204_3F99, 0xB7DA_453C, 0xA569_E79D, 0x9AAE_C33D);
pub const I_EDIT_CONTROLLER_IID: TUID =
    inline_uid(0xDCD7_BBE3, 0x7742_448D, 0xA874_AACC, 0x979C_759E);
pub const I_COMPONENT_HANDLER_IID: TUID =
    inline_uid(0x93A0_BEA3, 0x0BD0_45DB, 0x8E89_0B0C, 0xC1E4_6AC6);
pub const I_HOST_APPLICATION_IID: TUID =
    inline_uid(0x58E5_95CC, 0xDB2D_4969, 0x8B6A_AF8C, 0x36A6_64E5);
pub const I_PARAM_CHANGES_IID: TUID =
    inline_uid(0xA477_9663, 0x0BB6_4A56, 0xB443_84A8, 0x466F_EB9D);
pub const I_PARAM_VALUE_QUEUE_IID: TUID =
    inline_uid(0x0126_3A18, 0xED07_4F6F, 0x98C9_D356, 0x4686_F9BA);
pub const I_B_STREAM_IID: TUID = inline_uid(0xC3BF_6EA2, 0x3099_4752, 0x9B6B_F990, 0x1EE3_3E9B);
pub const I_SIZEABLE_STREAM_IID: TUID =
    inline_uid(0x04F9_549E, 0xE02F_4E6E, 0x87E8_6A87, 0x47F4_E17F);
pub const I_CONNECTION_POINT_IID: TUID =
    inline_uid(0x70A4_156F, 0x6E6E_4026, 0x9891_48BF, 0xAA60_D8D1);

/// VST3 effect class category (kVstAudioEffectClass).
pub const VST_AUDIO_EFFECT_CLASS: &str = "Audio Module Class";
/// Subcategory marker for instruments (PClassInfo2 subCategories entry).
pub const VST_INSTRUMENT_SUBCATEGORY: &str = "Instrument";

// MediaTypes / BusDirections / BusTypes.
pub const MEDIA_TYPE_AUDIO: i32 = 0;
pub const BUS_DIRECTION_INPUT: i32 = 0;
pub const BUS_DIRECTION_OUTPUT: i32 = 1;
pub const BUS_TYPE_MAIN: i32 = 0;

// SymbolicSampleSizes / ProcessModes.
pub const SYMBOLIC_SAMPLE_SIZE_32: i32 = 0;
pub const PROCESS_MODE_REALTIME: i32 = 0;

// ParameterInfo flags.
pub const PARAM_CAN_AUTOMATE: i32 = 1 << 0;
pub const PARAM_IS_READ_ONLY: i32 = 1 << 1;
pub const PARAM_IS_HIDDEN: i32 = 1 << 4;

/// `PClassInfo`: one class exposed by a plugin factory.
#[repr(C)]
pub struct PClassInfo {
    pub cid: TUID,
    pub cardinality: i32,
    pub category: [c_char; 32],
    pub name: [c_char; 64],
}

/// `PClassInfo2` (IPluginFactory2): adds vendor/version/subcategories.
#[repr(C)]
pub struct PClassInfo2 {
    pub cid: TUID,
    pub cardinality: i32,
    pub category: [c_char; 32],
    pub name: [c_char; 64],
    pub class_flags: u32,
    pub sub_categories: [c_char; 128],
    pub vendor: [c_char; 64],
    pub version: [c_char; 64],
    pub sdk_version: [c_char; 64],
}

/// `BusInfo` from IComponent::getBusInfo.
#[repr(C)]
pub struct BusInfo {
    pub media_type: i32,
    pub direction: i32,
    pub channel_count: i32,
    pub name: String128,
    pub bus_type: i32,
    pub flags: u32,
}

/// `ProcessSetup` for IAudioProcessor::setupProcessing.
#[repr(C)]
pub struct ProcessSetup {
    pub process_mode: i32,
    pub symbolic_sample_size: i32,
    pub max_samples_per_block: i32,
    pub sample_rate: f64,
}

/// `AudioBusBuffers`: per-bus planar channel pointers.
#[repr(C)]
pub struct AudioBusBuffers {
    pub num_channels: i32,
    pub silence_flags: u64,
    pub channel_buffers32: *mut *mut f32,
}

/// `ProcessData` for IAudioProcessor::process.
#[repr(C)]
pub struct ProcessData {
    pub process_mode: i32,
    pub symbolic_sample_size: i32,
    pub num_samples: i32,
    pub num_inputs: i32,
    pub num_outputs: i32,
    pub inputs: *mut AudioBusBuffers,
    pub outputs: *mut AudioBusBuffers,
    pub input_parameter_changes: *mut IParameterChanges,
    pub output_parameter_changes: *mut IParameterChanges,
    pub input_events: *mut c_void,
    pub output_events: *mut c_void,
    pub process_context: *mut c_void,
}

/// `ParameterInfo` from IEditController::getParameterInfo.
#[repr(C)]
pub struct ParameterInfo {
    pub id: ParamID,
    pub title: String128,
    pub short_title: String128,
    pub units: String128,
    pub step_count: i32,
    pub default_normalized_value: ParamValue,
    pub unit_id: i32,
    pub flags: i32,
}

// ---------------------------------------------------------------------------
// Interface mirrors. Every method carries the same safety contract: `this`
// must be a live object of exactly this interface, obtained from the plugin
// for the matching IID, and must not be used after its final release.
// ---------------------------------------------------------------------------

pub type QueryInterfaceFn =
    unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> i32;
pub type AddRefFn = unsafe extern "system" fn(*mut c_void) -> u32;
pub type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;

macro_rules! interface {
    ($object:ident, $vtable:ident, { $($slot:ident : $signature:ty),* $(,)? }) => {
        #[repr(C)]
        pub struct $object {
            pub(crate) vtable: *const $vtable,
        }
        #[repr(C)]
        pub struct $vtable {
            pub query_interface: QueryInterfaceFn,
            pub add_ref: AddRefFn,
            pub release: ReleaseFn,
            $(pub $slot: $signature,)*
        }
        impl $object {
            /// # Safety
            ///
            /// `this` must be a live object of this interface.
            pub unsafe fn query_interface(
                this: *mut Self,
                iid: &TUID,
                obj: &mut *mut c_void,
            ) -> i32 {
                // SAFETY: the first word of a COM object is its vtable
                // pointer; the slot order matches the C++ declaration.
                let vtable = unsafe { *(this.cast::<*const $vtable>()) };
                unsafe {
                    ((*vtable).query_interface)(this as *mut Self as *mut c_void, iid.as_ptr(), obj)
                }
            }
            /// # Safety
            ///
            /// `this` must be a live object of this interface.
            pub unsafe fn add_ref(this: *mut Self) -> u32 {
                // SAFETY: as above.
                let vtable = unsafe { *(this.cast::<*const $vtable>()) };
                unsafe { ((*vtable).add_ref)(this as *mut Self as *mut c_void) }
            }
            /// # Safety
            ///
            /// `this` must be a live object of this interface and must not
            /// be used afterwards.
            pub unsafe fn release(this: *mut Self) -> u32 {
                // SAFETY: as above.
                let vtable = unsafe { *(this.cast::<*const $vtable>()) };
                unsafe { ((*vtable).release)(this as *mut Self as *mut c_void) }
            }
        }
    };
}

interface!(IPluginFactory, IPluginFactoryVtable, {
    get_factory_info: unsafe extern "system" fn(*mut IPluginFactory, *mut c_void) -> i32,
    count_classes: unsafe extern "system" fn(*mut IPluginFactory) -> i32,
    get_class_info: unsafe extern "system" fn(*mut IPluginFactory, i32, *mut PClassInfo) -> i32,
    create_instance: unsafe extern "system" fn(
        *mut IPluginFactory,
        cid: *const c_char,
        iid: *const c_char,
        obj: *mut *mut c_void,
    ) -> i32,
});

impl IPluginFactory {
    /// # Safety
    ///
    /// `this` must be a live factory object.
    pub unsafe fn count_classes(this: *mut Self) -> i32 {
        // SAFETY: live object; slot order matches the C++ declaration.
        let vtable = unsafe { *(this.cast::<*const IPluginFactoryVtable>()) };
        unsafe { ((*vtable).count_classes)(this) }
    }
    /// # Safety
    ///
    /// `this` must be a live factory object; `info` must be writable.
    pub unsafe fn get_class_info(this: *mut Self, index: i32, info: &mut PClassInfo) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IPluginFactoryVtable>()) };
        unsafe { ((*vtable).get_class_info)(this, index, info) }
    }
    /// # Safety
    ///
    /// `this` must be a live factory object; `cid`/`iid` are 16-byte UIDs
    /// and `obj` receives the new object for `iid`.
    pub unsafe fn create_instance(
        this: *mut Self,
        cid: &TUID,
        iid: &TUID,
        obj: &mut *mut c_void,
    ) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IPluginFactoryVtable>()) };
        unsafe {
            ((*vtable).create_instance)(
                this,
                cid.as_ptr() as *const c_char,
                iid.as_ptr() as *const c_char,
                obj,
            )
        }
    }
    /// # Safety
    ///
    /// `this` must be a live factory object. The returned pointer, when
    /// non-null, must be released like any interface pointer.
    pub unsafe fn query_factory2(this: *mut Self) -> *mut IPluginFactory2 {
        let mut factory2: *mut c_void = std::ptr::null_mut();
        // SAFETY: live object; the IID matches the requested interface.
        let result = unsafe { Self::query_interface(this, &I_PLUGIN_FACTORY2_IID, &mut factory2) };
        if result == K_RESULT_OK && !factory2.is_null() {
            factory2 as *mut IPluginFactory2
        } else {
            std::ptr::null_mut()
        }
    }
}

// `IPluginFactory2 : public IPluginFactory`, so the real vtable is the full
// IPluginFactory vtable (FUnknown's 3 slots + its 4 methods) with
// getClassInfo2 appended — 8 slots in declaration order. A mirror with only
// getClassInfo2 calls getFactoryInfo when asked for getClassInfo2.
interface!(IPluginFactory2, IPluginFactory2Vtable, {
    get_factory_info: unsafe extern "system" fn(*mut IPluginFactory2, *mut c_void) -> i32,
    count_classes: unsafe extern "system" fn(*mut IPluginFactory2) -> i32,
    get_class_info: unsafe extern "system" fn(*mut IPluginFactory2, i32, *mut PClassInfo) -> i32,
    create_instance: unsafe extern "system" fn(
        *mut IPluginFactory2,
        cid: *const c_char,
        iid: *const c_char,
        obj: *mut *mut c_void,
    ) -> i32,
    get_class_info2: unsafe extern "system" fn(*mut IPluginFactory2, i32, *mut PClassInfo2) -> i32,
});

impl IPluginFactory2 {
    /// # Safety
    ///
    /// `this` must be a live factory object; `info` must be writable.
    pub unsafe fn get_class_info2(this: *mut Self, index: i32, info: &mut PClassInfo2) -> i32 {
        // SAFETY: live object; slot order matches the C++ declaration.
        let vtable = unsafe { *(this.cast::<*const IPluginFactory2Vtable>()) };
        unsafe { ((*vtable).get_class_info2)(this, index, info) }
    }
}

interface!(IComponent, IComponentVtable, {
    initialize: unsafe extern "system" fn(*mut IComponent, *mut c_void) -> i32,
    terminate: unsafe extern "system" fn(*mut IComponent) -> i32,
    get_controller_class_id: unsafe extern "system" fn(*mut IComponent, *mut u8) -> i32,
    set_io_mode: unsafe extern "system" fn(*mut IComponent, i32) -> i32,
    get_bus_count: unsafe extern "system" fn(*mut IComponent, i32, i32) -> i32,
    get_bus_info: unsafe extern "system" fn(*mut IComponent, i32, i32, i32, *mut BusInfo) -> i32,
    get_routing_info: unsafe extern "system" fn(*mut IComponent, *mut c_void, *mut c_void) -> i32,
    activate_bus: unsafe extern "system" fn(
        *mut IComponent,
        media_type: i32,
        direction: i32,
        index: i32,
        state: TBool,
    ) -> i32,
    set_active: unsafe extern "system" fn(*mut IComponent, TBool) -> i32,
    set_state: unsafe extern "system" fn(*mut IComponent, *mut IBStream) -> i32,
    get_state: unsafe extern "system" fn(*mut IComponent, *mut IBStream) -> i32,
});

impl IComponent {
    /// # Safety
    ///
    /// `this` must be a live component; `context` is a host FUnknown
    /// (usually the IHostApplication object).
    pub unsafe fn initialize(this: *mut Self, context: *mut c_void) -> i32 {
        // SAFETY: live object; slot order matches the C++ declaration.
        let vtable = unsafe { *(this.cast::<*const IComponentVtable>()) };
        unsafe { ((*vtable).initialize)(this, context) }
    }
    /// # Safety
    ///
    /// `this` must be a live, initialized component and must not be used
    /// afterwards.
    pub unsafe fn terminate(this: *mut Self) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IComponentVtable>()) };
        unsafe { ((*vtable).terminate)(this) }
    }
    /// # Safety
    ///
    /// `this` must be a live component; `class_id` receives the controller
    /// CID when the method succeeds.
    pub unsafe fn get_controller_class_id(this: *mut Self, class_id: &mut TUID) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IComponentVtable>()) };
        unsafe { ((*vtable).get_controller_class_id)(this, class_id.as_mut_ptr()) }
    }
    /// # Safety
    ///
    /// `this` must be a live component.
    pub unsafe fn get_bus_count(this: *mut Self, media_type: i32, direction: i32) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IComponentVtable>()) };
        unsafe { ((*vtable).get_bus_count)(this, media_type, direction) }
    }
    /// # Safety
    ///
    /// `this` must be a live component; `info` must be writable.
    pub unsafe fn get_bus_info(
        this: *mut Self,
        media_type: i32,
        direction: i32,
        index: i32,
        info: &mut BusInfo,
    ) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IComponentVtable>()) };
        unsafe { ((*vtable).get_bus_info)(this, media_type, direction, index, info) }
    }
    /// # Safety
    ///
    /// `this` must be a live component.
    pub unsafe fn activate_bus(
        this: *mut Self,
        media_type: i32,
        direction: i32,
        index: i32,
        state: TBool,
    ) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IComponentVtable>()) };
        unsafe { ((*vtable).activate_bus)(this, media_type, direction, index, state) }
    }
    /// # Safety
    ///
    /// `this` must be a live component.
    pub unsafe fn set_active(this: *mut Self, state: TBool) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IComponentVtable>()) };
        unsafe { ((*vtable).set_active)(this, state) }
    }
    /// # Safety
    ///
    /// `this` must be a live component; `state` must be a live IBStream.
    pub unsafe fn set_state(this: *mut Self, state: *mut IBStream) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IComponentVtable>()) };
        unsafe { ((*vtable).set_state)(this, state) }
    }
    /// # Safety
    ///
    /// `this` must be a live component; `state` must be a live IBStream.
    pub unsafe fn get_state(this: *mut Self, state: *mut IBStream) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IComponentVtable>()) };
        unsafe { ((*vtable).get_state)(this, state) }
    }
    /// # Safety
    ///
    /// `this` must be a live component. The returned pointer, when non-null,
    /// must be released like any interface pointer.
    pub unsafe fn query_interface_as<T>(this: *mut Self, iid: &TUID) -> *mut T {
        let mut obj: *mut c_void = std::ptr::null_mut();
        // SAFETY: live object; the IID matches the requested interface.
        let result = unsafe { Self::query_interface(this, iid, &mut obj) };
        if result == K_RESULT_OK && !obj.is_null() {
            obj as *mut T
        } else {
            std::ptr::null_mut()
        }
    }
}

interface!(IAudioProcessor, IAudioProcessorVtable, {
    set_bus_arrangements: unsafe extern "system" fn(
        *mut IAudioProcessor,
        *mut u64,
        i32,
        *mut u64,
        i32,
    ) -> i32,
    get_bus_arrangement: unsafe extern "system" fn(*mut IAudioProcessor, i32, i32, *mut u64) -> i32,
    can_process_sample_size: unsafe extern "system" fn(*mut IAudioProcessor, i32) -> i32,
    get_latency_samples: unsafe extern "system" fn(*mut IAudioProcessor) -> u32,
    setup_processing: unsafe extern "system" fn(*mut IAudioProcessor, *mut ProcessSetup) -> i32,
    set_processing: unsafe extern "system" fn(*mut IAudioProcessor, TBool) -> i32,
    process: unsafe extern "system" fn(*mut IAudioProcessor, *mut ProcessData) -> i32,
    get_tail_samples: unsafe extern "system" fn(*mut IAudioProcessor) -> u32,
});

impl IAudioProcessor {
    /// # Safety
    ///
    /// `this` must be a live processor.
    pub unsafe fn can_process_sample_size(this: *mut Self, size: i32) -> i32 {
        // SAFETY: live object; slot order matches the C++ declaration.
        let vtable = unsafe { *(this.cast::<*const IAudioProcessorVtable>()) };
        unsafe { ((*vtable).can_process_sample_size)(this, size) }
    }
    /// # Safety
    ///
    /// `this` must be a live processor.
    pub unsafe fn get_latency_samples(this: *mut Self) -> u32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IAudioProcessorVtable>()) };
        unsafe { ((*vtable).get_latency_samples)(this) }
    }
    /// # Safety
    ///
    /// `this` must be a live processor; `setup` must be fully populated.
    pub unsafe fn setup_processing(this: *mut Self, setup: &mut ProcessSetup) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IAudioProcessorVtable>()) };
        unsafe { ((*vtable).setup_processing)(this, setup) }
    }
    /// # Safety
    ///
    /// `this` must be a live processor.
    pub unsafe fn set_processing(this: *mut Self, state: TBool) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IAudioProcessorVtable>()) };
        unsafe { ((*vtable).set_processing)(this, state) }
    }
    /// # Safety
    ///
    /// `this` must be a live processor; `data` must reference valid buffers
    /// and host objects for the duration of the call.
    pub unsafe fn process(this: *mut Self, data: &mut ProcessData) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IAudioProcessorVtable>()) };
        unsafe { ((*vtable).process)(this, data) }
    }
}

interface!(IEditController, IEditControllerVtable, {
    initialize: unsafe extern "system" fn(*mut IEditController, *mut c_void) -> i32,
    terminate: unsafe extern "system" fn(*mut IEditController) -> i32,
    set_component_state: unsafe extern "system" fn(*mut IEditController, *mut IBStream) -> i32,
    set_state: unsafe extern "system" fn(*mut IEditController, *mut IBStream) -> i32,
    get_state: unsafe extern "system" fn(*mut IEditController, *mut IBStream) -> i32,
    get_parameter_count: unsafe extern "system" fn(*mut IEditController) -> i32,
    get_parameter_info:
        unsafe extern "system" fn(*mut IEditController, i32, *mut ParameterInfo) -> i32,
    get_param_string_by_value:
        unsafe extern "system" fn(*mut IEditController, ParamID, ParamValue, *mut String128) -> i32,
    get_param_value_by_string: unsafe extern "system" fn(
        *mut IEditController,
        ParamID,
        *const TChar,
        *mut ParamValue,
    ) -> i32,
    normalized_param_to_plain:
        unsafe extern "system" fn(*mut IEditController, ParamID, ParamValue) -> ParamValue,
    plain_param_to_normalized:
        unsafe extern "system" fn(*mut IEditController, ParamID, ParamValue) -> ParamValue,
    get_param_normalized: unsafe extern "system" fn(*mut IEditController, ParamID) -> ParamValue,
    set_param_normalized:
        unsafe extern "system" fn(*mut IEditController, ParamID, ParamValue) -> i32,
    set_component_handler: unsafe extern "system" fn(*mut IEditController, *mut c_void) -> i32,
    create_view: unsafe extern "system" fn(*mut IEditController, *const c_char) -> *mut c_void,
});

impl IEditController {
    /// # Safety
    ///
    /// `this` must be a live controller; `context` is a host FUnknown.
    pub unsafe fn initialize(this: *mut Self, context: *mut c_void) -> i32 {
        // SAFETY: live object; slot order matches the C++ declaration.
        let vtable = unsafe { *(this.cast::<*const IEditControllerVtable>()) };
        unsafe { ((*vtable).initialize)(this, context) }
    }
    /// # Safety
    ///
    /// `this` must be a live, initialized controller and must not be used
    /// afterwards.
    pub unsafe fn terminate(this: *mut Self) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IEditControllerVtable>()) };
        unsafe { ((*vtable).terminate)(this) }
    }
    /// # Safety
    ///
    /// `this` must be a live controller.
    pub unsafe fn get_parameter_count(this: *mut Self) -> i32 {
        // SAFETY: live object; slot order matches the C++ declaration.
        let vtable = unsafe { *(this.cast::<*const IEditControllerVtable>()) };
        unsafe { ((*vtable).get_parameter_count)(this) }
    }
    /// # Safety
    ///
    /// `this` must be a live controller; `info` must be writable.
    pub unsafe fn get_parameter_info(this: *mut Self, index: i32, info: &mut ParameterInfo) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IEditControllerVtable>()) };
        unsafe { ((*vtable).get_parameter_info)(this, index, info) }
    }
    /// # Safety
    ///
    /// `this` must be a live controller; `state` must be a live IBStream.
    pub unsafe fn set_component_state(this: *mut Self, state: *mut IBStream) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IEditControllerVtable>()) };
        unsafe { ((*vtable).set_component_state)(this, state) }
    }
    /// # Safety
    ///
    /// `this` must be a live controller; `state` must be a live IBStream.
    pub unsafe fn set_state(this: *mut Self, state: *mut IBStream) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IEditControllerVtable>()) };
        unsafe { ((*vtable).set_state)(this, state) }
    }
    /// # Safety
    ///
    /// `this` must be a live controller; `state` must be a live IBStream.
    pub unsafe fn get_state(this: *mut Self, state: *mut IBStream) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IEditControllerVtable>()) };
        unsafe { ((*vtable).get_state)(this, state) }
    }
    /// # Safety
    ///
    /// `this` must be a live controller; `handler` is a host
    /// IComponentHandler (or null to detach).
    pub unsafe fn set_component_handler(this: *mut Self, handler: *mut c_void) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IEditControllerVtable>()) };
        unsafe { ((*vtable).set_component_handler)(this, handler) }
    }
}

// Host-implemented interfaces: same object layout, but this side owns the
// vtable instances (see vst3/mod.rs) and frees them with the instance
// rather than on a zero refcount.

interface!(IComponentHandler, IComponentHandlerVtable, {
    begin_edit: unsafe extern "system" fn(*mut IComponentHandler, ParamID) -> i32,
    perform_edit: unsafe extern "system" fn(*mut IComponentHandler, ParamID, ParamValue) -> i32,
    end_edit: unsafe extern "system" fn(*mut IComponentHandler, ParamID) -> i32,
    restart_component: unsafe extern "system" fn(*mut IComponentHandler, i32) -> i32,
});

interface!(IHostApplication, IHostApplicationVtable, {
    get_name: unsafe extern "system" fn(*mut IHostApplication, *mut String128) -> i32,
    create_instance: unsafe extern "system" fn(
        *mut IHostApplication,
        *const u8,
        *const u8,
        *mut *mut c_void,
    ) -> i32,
});

interface!(IParameterChanges, IParameterChangesVtable, {
    get_parameter_count: unsafe extern "system" fn(*mut IParameterChanges) -> i32,
    get_parameter_data:
        unsafe extern "system" fn(*mut IParameterChanges, i32) -> *mut IParamValueQueue,
    add_parameter_data:
        unsafe extern "system" fn(*mut IParameterChanges, *const ParamID, *mut i32) -> *mut IParamValueQueue,
});

interface!(IParamValueQueue, IParamValueQueueVtable, {
    get_parameter_id: unsafe extern "system" fn(*mut IParamValueQueue) -> ParamID,
    get_point_count: unsafe extern "system" fn(*mut IParamValueQueue) -> i32,
    get_point: unsafe extern "system" fn(
        *mut IParamValueQueue,
        i32,
        *mut i32,
        *mut ParamValue,
    ) -> i32,
    add_point: unsafe extern "system" fn(*mut IParamValueQueue, i32, ParamValue, *mut i32) -> i32,
});

interface!(IBStream, IBStreamVtable, {
    read: unsafe extern "system" fn(*mut IBStream, *mut c_void, i32, *mut i32) -> i32,
    write: unsafe extern "system" fn(*mut IBStream, *mut c_void, i32, *mut i32) -> i32,
    seek: unsafe extern "system" fn(*mut IBStream, i64, i32, *mut i64) -> i32,
    tell: unsafe extern "system" fn(*mut IBStream, *mut i64) -> i32,
});

// `ISizeableStream`: served by the host's state stream so plugins' sized
// reads (JUCE's readFromMemoryStream) can query the exact byte count
// instead of probing.
interface!(ISizeableStream, ISizeableStreamVtable, {
    get_stream_size: unsafe extern "system" fn(*mut ISizeableStream, *mut i64) -> i32,
    set_stream_size: unsafe extern "system" fn(*mut ISizeableStream, i64) -> i32,
});

// `IConnectionPoint` (pluginterfaces/vst/ivstmessage.h): pairs a component
// with its separate edit controller. The host connects both sides; `notify`
// stays plugin-side and is never dispatched by this host.
interface!(IConnectionPoint, IConnectionPointVtable, {
    connect: unsafe extern "system" fn(*mut IConnectionPoint, *mut IConnectionPoint) -> i32,
    disconnect: unsafe extern "system" fn(*mut IConnectionPoint, *mut IConnectionPoint) -> i32,
    notify: unsafe extern "system" fn(*mut IConnectionPoint, *mut c_void) -> i32,
});

impl IConnectionPoint {
    /// # Safety
    ///
    /// `this` and `other` must be live connection points.
    pub unsafe fn connect(this: *mut Self, other: *mut Self) -> i32 {
        // SAFETY: live object; slot order matches the C++ declaration.
        let vtable = unsafe { *(this.cast::<*const IConnectionPointVtable>()) };
        unsafe { ((*vtable).connect)(this, other) }
    }
    /// # Safety
    ///
    /// `this` and `other` must be live connection points.
    pub unsafe fn disconnect(this: *mut Self, other: *mut Self) -> i32 {
        // SAFETY: as above.
        let vtable = unsafe { *(this.cast::<*const IConnectionPointVtable>()) };
        unsafe { ((*vtable).disconnect)(this, other) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand expansion of the official COM-compatible `INLINE_UID` macro for
    /// `DECLARE_CLASS_IID (IComponent, 0xE831FF31, 0xF2D54301, 0x928EBBEE,
    /// 0x25697802)`: l1 little-endian, l2 split (high half big-endian, low
    /// half little-endian), l3/l4 big-endian. A byte-order drift here keeps a
    /// self-built fixture green while every real plugin's queryInterface
    /// answers `E_NOINTERFACE`.
    #[test]
    fn inline_uid_matches_official_macro_expansion() {
        assert_eq!(
            I_COMPONENT_IID,
            [
                0x31, 0xFF, 0x31, 0xE8, // l1 little-endian
                0xD5, 0xF2, 0x01, 0x43, // l2: high half BE, low half LE
                0x92, 0x8E, 0xBB, 0xEE, // l3 big-endian
                0x25, 0x69, 0x78, 0x02, // l4 big-endian
            ]
        );
    }

    /// Pin every vtable to the slot count of its official C++ declaration:
    /// 3 FUnknown slots, then each inherited interface's methods, then the
    /// interface's own. A dropped inherited prefix (the IPluginFactory2 bug:
    /// its getClassInfo2 landed on the getFactoryInfo slot and segfaulted
    /// real plugins while fixtures without factory2 stayed green) shifts
    /// every dispatch and changes this size.
    #[test]
    fn vtable_slot_counts_match_official_inheritance() {
        let slot = std::mem::size_of::<usize>();
        assert_eq!(std::mem::size_of::<IPluginFactoryVtable>(), slot * 7); // FUnknown + 4
        assert_eq!(std::mem::size_of::<IPluginFactory2Vtable>(), slot * 8); // IPluginFactory + 1
        assert_eq!(std::mem::size_of::<IComponentVtable>(), slot * 14); // IPluginBase + 9
        assert_eq!(std::mem::size_of::<IAudioProcessorVtable>(), slot * 11); // FUnknown + 8
        assert_eq!(std::mem::size_of::<IEditControllerVtable>(), slot * 18); // IPluginBase + 13
        assert_eq!(std::mem::size_of::<IComponentHandlerVtable>(), slot * 7); // FUnknown + 4
        assert_eq!(std::mem::size_of::<IHostApplicationVtable>(), slot * 5); // FUnknown + 2
        assert_eq!(std::mem::size_of::<IParameterChangesVtable>(), slot * 6); // FUnknown + 3
        assert_eq!(std::mem::size_of::<IParamValueQueueVtable>(), slot * 7); // FUnknown + 4
        assert_eq!(std::mem::size_of::<IBStreamVtable>(), slot * 7); // FUnknown + 4
        assert_eq!(std::mem::size_of::<ISizeableStreamVtable>(), slot * 5); // FUnknown + 2
        assert_eq!(std::mem::size_of::<IConnectionPointVtable>(), slot * 6); // FUnknown + 3
    }
}
