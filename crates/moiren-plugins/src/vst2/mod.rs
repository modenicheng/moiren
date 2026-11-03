//! VST2 backend. One module hosts one effect; the entry point is
//! `VSTPluginMain` (legacy `main`). The host callback answers only the
//! opcodes plugins need to initialize: version, sample rate, block size,
//! process level. Parameter automation is applied at block starts through
//! `setParameter` on the audio thread, the classic host pattern.

use std::{
    ffi::{CStr, c_void},
    path::Path,
    sync::Arc,
};

pub mod abi;

use crate::{
    instance::PluginInstance,
    module::{SharedModule, acquire_module},
    spec::{PluginDescriptor, PluginError, PluginFormat, PluginParamEvent, PluginParameterSpec},
};
use abi::*;

/// Per-instance host context; `AEffect.reserved1` points here from before
/// `effOpen` until after `effClose`, so the audioMaster callback can resolve
/// host answers for this effect.
struct HostContext {
    sample_rate: isize,
    max_block_frames: isize,
}

unsafe extern "C" fn audio_master(
    effect: *mut AEffect,
    opcode: i32,
    _index: i32,
    _value: isize,
    _ptr: *mut c_void,
    _opt: f32,
) -> isize {
    let context = |effect: *mut AEffect| -> Option<&'static HostContext> {
        if effect.is_null() {
            return None;
        }
        // SAFETY: reserved1 is set by this host to a live HostContext box
        // before effOpen and cleared only after effClose has returned.
        let raw = unsafe { (*effect).reserved1 } as *const HostContext;
        (!raw.is_null()).then(|| unsafe { &*raw })
    };
    match opcode {
        AUDIO_MASTER_VERSION => HOST_VST_VERSION,
        AUDIO_MASTER_CURRENT_ID => 0,
        AUDIO_MASTER_GET_SAMPLE_RATE => context(effect).map(|ctx| ctx.sample_rate).unwrap_or(0),
        AUDIO_MASTER_GET_BLOCK_SIZE => context(effect).map(|ctx| ctx.max_block_frames).unwrap_or(0),
        AUDIO_MASTER_GET_CURRENT_PROCESS_LEVEL => PROCESS_LEVEL_REALTIME,
        // Automate/idle/canDo/updateDisplay/beginEdit/endEdit and everything
        // else: acknowledged with no action. Reporting support we do not
        // have (canDo) would push plugins onto unimplemented paths.
        AUDIO_MASTER_AUTOMATE
        | AUDIO_MASTER_IDLE
        | AUDIO_MASTER_CAN_DO
        | AUDIO_MASTER_UPDATE_DISPLAY
        | AUDIO_MASTER_BEGIN_EDIT
        | AUDIO_MASTER_END_EDIT => 0,
        _ => 0,
    }
}

fn string_from_cbuf(buffer: &[u8]) -> String {
    CStr::from_bytes_until_nul(buffer)
        .map(|text| text.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Loaded module plus its resolved entry point.
struct Vst2Module {
    _module: Arc<SharedModule>,
    entry: PluginMain,
}

fn load_module(path: &Path) -> Result<Vst2Module, PluginError> {
    let module = acquire_module(path, |_| Ok(None))?;
    let resolve = |name: &[u8]| -> Option<PluginMain> {
        // SAFETY: the symbol value is copied out; the module handle keeps
        // the library mapped for as long as Vst2Module exists.
        unsafe { module.library.get::<PluginMain>(name) }
            .ok()
            .map(|symbol| *symbol)
    };
    let entry = resolve(b"VSTPluginMain\0").or_else(|| resolve(b"main\0"));
    let Some(entry) = entry else {
        return Err(PluginError::Entry(
            "VSTPluginMain (or legacy main) symbol missing".into(),
        ));
    };
    Ok(Vst2Module {
        _module: module,
        entry,
    })
}

/// Everything learned from one opened effect, before activation.
struct OpenedEffect {
    descriptor: PluginDescriptor,
    parameters: Vec<PluginParameterSpec>,
    effect: *mut AEffect,
    context: *mut HostContext,
    module: Arc<SharedModule>,
    has_chunks: bool,
}

// The effect object and its module are used from whichever thread drives
// audio; the VST2 contract allows single-threaded use on any thread, and
// every pointer here belongs to the effect's module, outliving this struct.
unsafe impl Send for OpenedEffect {}

/// Open the effect and collect its properties. `effOpen` through property
/// reads; no activation. `effect` is live and must be closed by the caller.
unsafe fn open_effect(
    module: Vst2Module,
    sample_rate: f64,
    max_block_frames: u32,
) -> Result<OpenedEffect, PluginError> {
    // SAFETY: the module handle keeps the entry point mapped; the effect is
    // created by our call and cleaned up by close_effect on every path.
    let shared = Arc::clone(&module._module);
    let effect = unsafe { (module.entry)(audio_master) };
    if effect.is_null() {
        return Err(PluginError::Init("VSTPluginMain returned null".into()));
    }
    // SAFETY: the effect exists; reading its header fields is the contract.
    if unsafe { (*effect).magic } != VST_MAGIC {
        // The object is plugin-allocated and untrustworthy past its magic
        // word; calling effClose on garbage could crash the process, so the
        // pointer leaks instead (rare: non-VST module or corrupt entry).
        return Err(PluginError::Entry("AEffect magic is not 'VstP'".into()));
    }
    let context = Box::into_raw(Box::new(HostContext {
        sample_rate: sample_rate as isize,
        max_block_frames: max_block_frames.max(1) as isize,
    }));
    // SAFETY: reserved1 is host-owned storage; the box lives until Drop.
    unsafe { (*effect).reserved1 = context as isize };

    let dispatcher = |opcode: i32, index: i32, value: isize, ptr: *mut c_void, opt: f32| -> isize {
        // SAFETY: the effect is open and this backend only dispatches
        // documented control opcodes control-side.
        unsafe { ((*effect).dispatcher)(effect, opcode, index, value, ptr, opt) }
    };
    dispatcher(EFF_OPEN, 0, 0, std::ptr::null_mut(), 0.0);
    dispatcher(
        EFF_SET_SAMPLE_RATE,
        0,
        0,
        std::ptr::null_mut(),
        sample_rate as f32,
    );
    dispatcher(
        EFF_SET_BLOCK_SIZE,
        0,
        max_block_frames.max(1) as isize,
        std::ptr::null_mut(),
        0.0,
    );

    // Header facts (inputs/outputs/latency) are readable once the effect is
    // open; the host keeps a copy because close frees the object.
    let (num_inputs, num_outputs, flags, unique_id) = unsafe {
        (
            (*effect).num_inputs.max(0) as u32,
            (*effect).num_outputs.max(0) as u32,
            (*effect).flags,
            (*effect).unique_id,
        )
    };
    if flags & EFF_FLAGS_CAN_REPLACING == 0 {
        // SAFETY: open but unusable; unwind before returning.
        unsafe { close_effect(effect, context) };
        return Err(PluginError::Init(
            "plugin does not implement processReplacing (VST 2.4 requirement)".into(),
        ));
    }

    // Parameters hold their defaults right after open, before any host set.
    let num_params = unsafe { (*effect).num_params }.clamp(0, 4096);
    let mut parameters = Vec::with_capacity(num_params as usize);
    for index in 0..num_params {
        let mut name = [0u8; 64];
        // The name buffer is 64 writable bytes; the dispatcher writes at
        // most kVstMaxParamStrLen bytes.
        dispatcher(EFF_GET_PARAM_NAME, index, 0, name.as_mut_ptr().cast(), 0.0);
        let name = string_from_cbuf(&name);
        let default = unsafe { ((*effect).get_parameter)(effect, index) } as f64;
        let automatable = {
            // Control-side opcode on the open effect; the answer is a
            // boolean where 1 means automatable.
            let answer = dispatcher(EFF_CAN_BE_AUTOMATED, index, 0, std::ptr::null_mut(), 0.0);
            answer == 1
        };
        parameters.push(PluginParameterSpec {
            id: index as u32,
            name,
            // VST2 automation values are normalized 0..1 floats.
            min: 0.0,
            max: 1.0,
            default: default.clamp(0.0, 1.0),
            automatable,
            stepped: false,
        });
    }

    let mut string_buffer = [0u8; 64];
    let read_string = |opcode: i32, buffer: &mut [u8; 64]| -> String {
        // The buffer is writable memory of kVstMax*StrLen capacity.
        dispatcher(opcode, 0, 0, buffer.as_mut_ptr().cast(), 0.0);
        string_from_cbuf(buffer)
    };
    let name = read_string(EFF_GET_EFFECT_NAME, &mut string_buffer);
    string_buffer.fill(0);
    let vendor = read_string(EFF_GET_VENDOR_STRING, &mut string_buffer);
    let version = dispatcher(EFF_GET_VENDOR_VERSION, 0, 0, std::ptr::null_mut(), 0.0);
    let descriptor = PluginDescriptor {
        format: PluginFormat::Vst2,
        path: Path::new("").to_path_buf(),
        id: format!("{unique_id:08X}"),
        name,
        vendor,
        version: format!(
            "{}.{}.{}.{}",
            (version >> 16) & 0xff,
            (version >> 12) & 0xf,
            (version >> 8) & 0xf,
            version & 0xff
        ),
        input_channels: num_inputs,
        output_channels: num_outputs,
        is_synth: flags & EFF_FLAGS_IS_SYNTH != 0,
        has_editor: flags & EFF_FLAGS_HAS_EDITOR != 0,
    };

    Ok(OpenedEffect {
        descriptor,
        parameters,
        effect,
        context,
        module: shared,
        has_chunks: flags & EFF_FLAGS_PROGRAM_CHUNKS != 0,
    })
}

/// Close an effect whose open sequence failed before any state was built.
/// SAFETY: `effect` was returned by VSTPluginMain and must not be reused.
unsafe fn close_bare_effect(effect: *mut AEffect) {
    // SAFETY: effClose is the release call for a non-null effect.
    unsafe { ((*effect).dispatcher)(effect, EFF_CLOSE, 0, 0, std::ptr::null_mut(), 0.0) };
}

/// Close an effect whose HostContext was attached, releasing both.
/// SAFETY: `context` must not be reused after this call.
unsafe fn close_effect(effect: *mut AEffect, context: *mut HostContext) {
    // SAFETY: the effect is open (or at minimum created with valid magic);
    // the context stays live through the close call, matching audio_master.
    unsafe {
        close_bare_effect(effect);
        drop(Box::from_raw(context));
    }
}

pub(crate) struct Vst2Instance {
    descriptor: PluginDescriptor,
    parameters: Vec<PluginParameterSpec>,
    effect: *mut AEffect,
    context: *mut HostContext,
    /// Keeps the library mapped through teardown; never dereferenced.
    _module: Arc<SharedModule>,
    has_chunks: bool,
    latency: u32,
    input_ptrs: Box<[*const f32]>,
    output_ptrs: Box<[*mut f32]>,
}

// Same single-thread-at-a-time contract as every format backend; see
// ClapInstance.
unsafe impl Send for Vst2Instance {}

pub(crate) fn instantiate(
    source: &PluginDescriptor,
    sample_rate: f64,
    max_block_frames: u32,
) -> Result<Vst2Instance, PluginError> {
    let module = load_module(&source.path)?;
    // SAFETY: the returned effect is fully owned by this function's scope;
    // every failure path below closes it via close_effect.
    let mut opened = unsafe { open_effect(module, sample_rate, max_block_frames)? };
    if opened.descriptor.input_channels == 0
        || opened.descriptor.input_channels != opened.descriptor.output_channels
    {
        let (input, output) = (
            opened.descriptor.input_channels,
            opened.descriptor.output_channels,
        );
        // SAFETY: opened effect with an attached context.
        unsafe { close_effect(opened.effect, opened.context) };
        return Err(PluginError::NoAudioPorts { input, output });
    }
    let channels = opened.descriptor.output_channels as usize;
    let dispatcher = |opcode: i32, index: i32, value: isize, ptr: *mut c_void, opt: f32| -> isize {
        // SAFETY: the open effect only receives documented control
        // opcodes on the control side here.
        unsafe { ((*opened.effect).dispatcher)(opened.effect, opcode, index, value, ptr, opt) }
    };
    // Activation sequence per the 2.4 hosting contract.
    dispatcher(EFF_MAINS_CHANGED, 0, 1, std::ptr::null_mut(), 0.0);
    dispatcher(EFF_START_PROCESS, 0, 0, std::ptr::null_mut(), 0.0);
    // Latency is meaningful in the resumed state.
    // SAFETY: header field of the live effect.
    let latency = unsafe { (*opened.effect).initial_delay }.max(0) as u32;
    opened.descriptor.path = source.path.clone();

    Ok(Vst2Instance {
        descriptor: opened.descriptor,
        parameters: opened.parameters,
        effect: opened.effect,
        context: opened.context,
        _module: opened.module,
        has_chunks: opened.has_chunks,
        latency,
        input_ptrs: vec![std::ptr::null(); channels].into_boxed_slice(),
        output_ptrs: vec![std::ptr::null_mut(); channels].into_boxed_slice(),
    })
}

impl Vst2Instance {
    /// Suspend, apply, resume — the state-transfer window the spec expects.
    fn with_suspended_renderer(
        &mut self,
        run: impl FnOnce(&Self) -> Result<(), PluginError>,
    ) -> Result<(), PluginError> {
        // SAFETY: control-side suspend/resume around the caller's transfer.
        unsafe {
            ((*self.effect).dispatcher)(
                self.effect,
                EFF_STOP_PROCESS,
                0,
                0,
                std::ptr::null_mut(),
                0.0,
            );
            ((*self.effect).dispatcher)(
                self.effect,
                EFF_MAINS_CHANGED,
                0,
                0,
                std::ptr::null_mut(),
                0.0,
            );
        }
        let outcome = run(self);
        // SAFETY: symmetric reactivation.
        unsafe {
            ((*self.effect).dispatcher)(
                self.effect,
                EFF_MAINS_CHANGED,
                0,
                1,
                std::ptr::null_mut(),
                0.0,
            );
            ((*self.effect).dispatcher)(
                self.effect,
                EFF_START_PROCESS,
                0,
                0,
                std::ptr::null_mut(),
                0.0,
            );
        }
        outcome
    }
}

impl PluginInstance for Vst2Instance {
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
        let frames = outputs.first().map_or(0, |c| c.len());
        // Parameter automation at block granularity: setParameter on the
        // audio thread immediately before processReplacing, the classic
        // VST2 hosting pattern for host-driven automation.
        for event in events {
            // SAFETY: direct function call on the live effect; the parameter
            // index came from this instance's scanned table.
            unsafe {
                ((*self.effect).set_parameter)(
                    self.effect,
                    event.parameter as i32,
                    event.value as f32,
                )
            };
        }
        for (slot, channel) in self.input_ptrs.iter_mut().zip(inputs) {
            *slot = channel.as_ptr();
        }
        for (slot, channel) in self.output_ptrs.iter_mut().zip(outputs.iter_mut()) {
            *slot = channel.as_mut_ptr();
        }
        let Some(process) = (unsafe { (*self.effect).process_replacing }) else {
            return Err(PluginError::Process(
                "processReplacing vanished after construction".into(),
            ));
        }; // SAFETY: the pointer tables match the channel counts negotiated at
        // construction; frames do not exceed the block size announced then.
        unsafe {
            process(
                self.effect,
                self.input_ptrs.as_ptr(),
                self.output_ptrs.as_mut_ptr(),
                frames as i32,
            );
        }
        Ok(())
    }

    fn save_state(&self) -> Result<Option<Vec<u8>>, PluginError> {
        if !self.has_chunks {
            return Ok(None);
        }
        let mut data: *mut c_void = std::ptr::null_mut();
        // SAFETY: control-side chunk request; the plugin hands back a buffer
        // it owns until the next dispatcher call.
        let size = unsafe {
            ((*self.effect).dispatcher)(
                self.effect,
                EFF_GET_CHUNK,
                1,
                0,
                std::ptr::from_mut(&mut data).cast(),
                0.0,
            )
        };
        if size <= 0 || data.is_null() {
            return Err(PluginError::State(
                "effGetChunk returned no program chunk".into(),
            ));
        }
        // SAFETY: size bytes readable per the chunk contract.
        let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size as usize) };
        Ok(Some(bytes.to_vec()))
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
        if !self.has_chunks || state.is_empty() {
            return Ok(());
        }
        let outcome = self.with_suspended_renderer(|instance| {
            // SAFETY: the state buffer outlives the dispatcher call; the
            // plugin copies what it needs.
            //
            // The return value carries no success signal: JUCE (and much of
            // the ecosystem) returns 0 here, other plugins return 1, so the
            // dispatcher result is deliberately ignored like mainstream
            // hosts do.
            unsafe {
                ((*instance.effect).dispatcher)(
                    instance.effect,
                    EFF_SET_CHUNK,
                    1,
                    state.len() as isize,
                    state.as_ptr() as *mut c_void,
                    0.0,
                )
            };
            Ok(())
        });
        // Defaults may have moved; refresh the scanned table's starting
        // point is not needed — automation targets stay normalized.
        outcome
    }
}

impl Drop for Vst2Instance {
    fn drop(&mut self) {
        // SAFETY: symmetric teardown of the activated effect; the context is
        // released after close so any callback during close finds it.
        unsafe {
            ((*self.effect).dispatcher)(
                self.effect,
                EFF_STOP_PROCESS,
                0,
                0,
                std::ptr::null_mut(),
                0.0,
            );
            ((*self.effect).dispatcher)(
                self.effect,
                EFF_MAINS_CHANGED,
                0,
                0,
                std::ptr::null_mut(),
                0.0,
            );
            ((*self.effect).dispatcher)(self.effect, EFF_CLOSE, 0, 0, std::ptr::null_mut(), 0.0);
            drop(Box::from_raw(self.context));
        }
        // The module handle field drops last, keeping the library mapped
        // through the whole teardown.
    }
}

/// Probe a VST2 module: open, read properties, close. The descriptor's
/// channel counts are the effect's declared pin counts.
pub(crate) fn scan_descriptor(path: &Path) -> Result<PluginDescriptor, PluginError> {
    let module = load_module(path)?;
    // SAFETY: the effect is closed below on both success and failure paths
    // (Drop of the error value is not involved; open_effect's internal
    // failures close themselves).
    let opened = unsafe { open_effect(module, 48000.0, 512)? };
    let mut descriptor = opened.descriptor;
    descriptor.path = path.to_path_buf();
    // SAFETY: symmetric close of a fully-opened, context-attached effect.
    unsafe { close_effect(opened.effect, opened.context) };
    Ok(descriptor)
}
