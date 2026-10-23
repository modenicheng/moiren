//! audio-hack-lab: experiments for hijacking / routing per-app audio on Windows
//! without drivers. Subcommands:
//!   default                     print default render endpoint id
//!   tone --dur S [--freq F]     play a sine to the default render endpoint
//!   loopback --dur S            classic AUDCLNT_STREAMFLAGS_LOOPBACK capture
//!   procloop --pid P --dur S    process-loopback capture ([--exclude])
//!   sessions [--device {g}] [--pid P]   list audio sessions on an endpoint
//!   mute --pid P --on|--off     SetMute all sessions of a pid
//!   epmute --on|--off|--query   endpoint master mute on default render device
//!   pref --pid P --device {g}   per-app output routing via hidden WinRT
//!                               Windows.Media.Internal.AudioPolicyConfig
//!   openproc                    OpenProcess probe against audiodg (PPL check)
//!
//! Timeline lines: TL tag=.. t=1.02 rms_db=.. tone_db=.. frames=.. silent_frames=..

#![allow(non_snake_case)]
use std::f32::consts::TAU;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows::core::{implement, Interface, HRESULT, HSTRING, Result};
use windows::Win32::Foundation::{BOOL, E_FAIL, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    eConsole, eRender, ActivateAudioInterfaceAsync, IAudioCaptureClient, IAudioClient,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioRenderClient, IAudioSessionControl, IAudioSessionControl2, IAudioSessionEnumerator,
    IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    WAVEFORMATEX,
};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{
    CreateEventW, OpenProcess, SetEvent, WaitForSingleObject, PROCESS_CREATE_THREAD,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
};
use windows::Win32::System::Variant::VT_BLOB;
use windows::Win32::System::WinRT::RoGetActivationFactory;

const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

// ------------------------------------------------------------------ util

fn db(x: f32) -> f32 {
    10.0 * (x + 1e-12).log10()
}

#[repr(C)]
struct Wfx {
    tag: u16,
    ch: u16,
    fs: u32,
    rate: u32,
    block: u16,
    bits: u16,
    cb: u16,
}

unsafe fn read_wfx(p: *const WAVEFORMATEX) -> Wfx {
    std::ptr::read_unaligned(p as *const Wfx)
}

struct WindowStats {
    tag: &'static str,
    freq: f32,
    fs: f32,
    samples: Vec<f32>,
    silent_frames: u64,
    frames: u64,
    window_start: Instant,
}

impl WindowStats {
    fn new(tag: &'static str, freq: f32, fs: f32) -> Self {
        Self { tag, freq, fs, samples: Vec::new(), silent_frames: 0, frames: 0, window_start: Instant::now() }
    }

    fn push(&mut self, data: &[u8], frames: u32, flags: u32, channels: u16, bits: u16, fmt_tag: u16) {
        self.frames += frames as u64;
        if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
            self.silent_frames += frames as u64;
            return;
        }
        let samps: Vec<f32> = if fmt_tag == WAVE_FORMAT_IEEE_FLOAT || (fmt_tag == WAVE_FORMAT_EXTENSIBLE && bits == 32) {
            data.chunks(size_of::<f32>()).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
        } else if bits == 16 {
            data.chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect()
        } else {
            vec![]
        };
        let ch = channels.max(1) as usize;
        self.samples.extend(samps.into_iter().step_by(ch));
    }

    fn flush(&mut self) {
        let n = self.samples.len().max(1);
        let mut rms = 0.0f32;
        for &s in &self.samples {
            rms += s * s;
        }
        rms = (rms / n as f32).sqrt();

        let k = (n as f32 * self.freq / self.fs).round();
        let w = TAU * k / n as f32;
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &s in &self.samples {
            let s0 = s + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
        let tone = power / (n * n) as f32 / (w.sin() * w.sin() + 1e-12).max(1e-12);

        let t = self.window_start.elapsed().as_secs_f32();
        println!(
            "TL tag={} t={:.2} rms_db={:.1} tone{}_db={:.1} frames={} silent_frames={}",
            self.tag, t, db(rms * rms), self.freq, db(tone), self.frames, self.silent_frames
        );
        self.samples.clear();
        self.frames = 0;
        self.silent_frames = 0;
    }
}

fn float_format(fs: u32, channels: u16) -> WAVEFORMATEX {
    let block = channels * 4;
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
        nChannels: channels,
        nSamplesPerSec: fs,
        wBitsPerSample: 32,
        nBlockAlign: block,
        nAvgBytesPerSec: fs * block as u32,
        cbSize: 0,
    }
}

unsafe fn default_render_device() -> Result<IMMDevice> {
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
    enumerator.GetDefaultAudioEndpoint(eRender, eConsole)
}

unsafe fn device_activate<T: Interface>(device: &IMMDevice) -> Result<T> {
    device.Activate(CLSCTX_ALL, None)
}

// ------------------------------------------------------------------ default

unsafe fn cmd_default() -> Result<()> {
    let device = default_render_device()?;
    let id = device.GetId()?;
    println!("default render endpoint: {}", id.to_string()?);
    Ok(())
}

// ------------------------------------------------------------------ tone

unsafe fn cmd_tone(freq: f32, dur: f32) -> Result<()> {
    println!("PID={} MODE=tone freq={} dur={}", std::process::id(), freq, dur);
    let device = default_render_device()?;
    let id = device.GetId()?;
    println!("render endpoint: {}", id.to_string()?);
    let client: IAudioClient = device_activate(&device)?;
    let fmt_ptr = client.GetMixFormat()?;
    let fmt = read_wfx(fmt_ptr);
    println!("mix format: {}ch {}Hz {}bits tag={:#x}", fmt.ch, fmt.fs, fmt.bits, fmt.tag);
    let fs = fmt.fs as f32;
    let channels = fmt.ch as usize;
    let use_float = fmt.tag == WAVE_FORMAT_IEEE_FLOAT
        || (fmt.tag == WAVE_FORMAT_EXTENSIBLE && fmt.bits == 32);

    client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, 2_000_000, 0, fmt_ptr, None)?;
    let event = CreateEventW(None, true, false, None)?;
    client.SetEventHandle(event)?;
    let render: IAudioRenderClient = client.GetService()?;
    let buf_frames = client.GetBufferSize()?;
    client.Start()?;

    let t0 = Instant::now();
    let mut phase = 0.0f32;
    let amp = 0.3f32;
    while t0.elapsed().as_secs_f32() < dur {
        let w = WaitForSingleObject(event, 2000);
        if w != WAIT_OBJECT_0 {
            break;
        }
        let padding = client.GetCurrentPadding()?;
        let avail = buf_frames - padding;
        if avail == 0 {
            continue;
        }
        let data = render.GetBuffer(avail)?;
        let n = avail as usize * channels;
        if use_float {
            let out = std::slice::from_raw_parts_mut(data as *mut f32, n);
            for frame in out.chunks_mut(channels) {
                phase += TAU * freq / fs;
                if phase > TAU {
                    phase -= TAU;
                }
                let v = amp * phase.sin();
                for ch in frame.iter_mut() {
                    *ch = v;
                }
            }
        } else {
            let out = std::slice::from_raw_parts_mut(data as *mut i16, n);
            for frame in out.chunks_mut(channels) {
                phase += TAU * freq / fs;
                let v = (amp * phase.sin() * 32767.0) as i16;
                for ch in frame.iter_mut() {
                    *ch = v;
                }
            }
        }
        render.ReleaseBuffer(avail, 0)?;
    }
    client.Stop()?;
    println!("tone done");
    Ok(())
}

// ------------------------------------------------------------------ classic loopback

unsafe fn cmd_loopback(freq: f32, dur: f32) -> Result<()> {
    let device = default_render_device()?;
    let client: IAudioClient = device_activate(&device)?;
    let fmt_ptr = client.GetMixFormat()?;
    let fmt = read_wfx(fmt_ptr);
    println!("CAPTURE loopback fmt: {}ch {}Hz {}bits", fmt.ch, fmt.fs, fmt.bits);
    client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, 1_000_000, 0, fmt_ptr, None)?;
    let capture: IAudioCaptureClient = client.GetService()?;
    client.Start()?;

    let mut stats = WindowStats::new("loopback", freq, fmt.fs as f32);
    let t0 = Instant::now();
    let mut next_flush = Instant::now() + Duration::from_millis(250);
    while t0.elapsed().as_secs_f32() < dur {
        let mut packet = capture.GetNextPacketSize()?;
        while packet != 0 {
            let mut data: *mut u8 = null_mut();
            let mut frames = 0u32;
            let mut flags = 0u32;
            capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
            if !data.is_null() {
                let bytes = frames as usize * fmt.block as usize;
                let buf = std::slice::from_raw_parts(data, bytes);
                stats.push(buf, frames, flags, fmt.ch, fmt.bits, fmt.tag);
            }
            capture.ReleaseBuffer(frames)?;
            packet = capture.GetNextPacketSize()?;
        }
        if Instant::now() >= next_flush {
            stats.flush();
            next_flush += Duration::from_millis(250);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    stats.flush();
    client.Stop()?;
    Ok(())
}

// ------------------------------------------------------------------ process loopback

#[implement(IActivateAudioInterfaceCompletionHandler, windows::Win32::System::Com::IAgileObject)]
struct ActivationHandler {
    done: HANDLE,
    outcome: std::sync::Arc<ActivationOutcome>,
}

impl windows::Win32::System::Com::IAgileObject_Impl for ActivationHandler_Impl {}

impl IActivateAudioInterfaceCompletionHandler_Impl for ActivationHandler_Impl {
    fn ActivateCompleted(&self, operation: Option<&windows::Win32::Media::Audio::IActivateAudioInterfaceAsyncOperation>) -> windows::core::Result<()> {
        let op = operation.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
        unsafe {
            let mut hr = HRESULT(0);
            let mut unk: Option<windows::core::IUnknown> = None;
            op.GetActivateResult(&mut hr, &mut unk)?;
            *self.outcome.hr.lock().unwrap() = hr;
            if hr.is_ok() {
                if let Some(unknown) = unk {
                    *self.outcome.client.lock().unwrap() = unknown.cast::<IAudioClient>().ok();
                }
            }
            let _ = SetEvent(self.done);
        }
        Ok(())
    }
}

// PROPVARIANT hand-rolled to reach the VT_BLOB union member without windows-core wrapper friction.
#[repr(C)]
struct BlobPv {
    vt: u16,
    reserved: [u16; 3],
    cb: u32,
    pad: u32,
    ptr: *mut u8,
}

#[derive(Default)]
struct ActivationOutcome {
    client: std::sync::Mutex<Option<IAudioClient>>,
    hr: std::sync::Mutex<HRESULT>,
}

unsafe fn cmd_procloop(pid: u32, freq: f32, dur: f32, exclude: bool) -> Result<()> {
    println!("CAPTURE procloop pid={} mode={} freq={}", pid, if exclude { "exclude" } else { "include" }, freq);
    let done = CreateEventW(None, true, false, None)?;
    let outcome = std::sync::Arc::new(ActivationOutcome {
        client: std::sync::Mutex::new(None),
        hr: std::sync::Mutex::new(HRESULT(0)),
    });
    let handler = ActivationHandler {
        done,
        outcome: outcome.clone(),
    };
    let handler_iface: IActivateAudioInterfaceCompletionHandler = handler.into();

    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: if exclude {
                    PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE
                } else {
                    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE
                },
            },
        },
    };
    let mut pv = BlobPv {
        vt: VT_BLOB.0,
        reserved: [0; 3],
        cb: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
        pad: 0,
        ptr: &mut params as *mut _ as *mut u8,
    };

    let op = ActivateAudioInterfaceAsync(
        VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
        &IAudioClient::IID,
        Some(&mut pv as *mut BlobPv as *const windows::core::PROPVARIANT),
        &handler_iface,
    );
    match &op {
        Ok(_) => println!("ACTIVATE call -> OK"),
        Err(e) => println!("ACTIVATE call -> ERR {:#x}", e.code().0),
    }
    let _op = op?;
    pv.ptr = null_mut();

    let w = WaitForSingleObject(done, 10_000);
    println!("WAIT -> {} callback_hr={:#x}", w.0, outcome.hr.lock().unwrap().0);
    if w != WAIT_OBJECT_0 {
        return Err(windows::core::Error::from(E_FAIL));
    }
    let hr = *outcome.hr.lock().unwrap();
    if !hr.is_ok() {
        return Err(windows::core::Error::from(hr));
    }
    let client = outcome
        .client
        .lock()
        .unwrap()
        .take()
        .ok_or_else(|| windows::core::Error::from(E_FAIL))?;

    let fs = 48_000u32;
    let mut fmt = float_format(fs, 2);
    let init = client.Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_LOOPBACK
            | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
            | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
            | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
        0,
        0,
        &mut fmt,
        None,
    );
    match &init {
        Ok(_) => println!("INITIALIZE -> OK"),
        Err(e) => println!("INITIALIZE -> ERR {:#x}", e.code().0),
    }
    init?;
    let event = CreateEventW(None, true, false, None)?;
    client.SetEventHandle(event)?;
    let capture: IAudioCaptureClient = client.GetService()?;
    client.Start()?;

    let mut stats = WindowStats::new("procloop", freq, fs as f32);
    let t0 = Instant::now();
    let mut next_flush = Instant::now() + Duration::from_millis(250);
    while t0.elapsed().as_secs_f32() < dur {
        let w = WaitForSingleObject(event, 200);
        if w == WAIT_OBJECT_0 {
            let mut packet = capture.GetNextPacketSize()?;
            while packet != 0 {
                let mut data: *mut u8 = null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                if !data.is_null() {
                    let bytes = frames as usize * fmt.nBlockAlign as usize;
                    let buf = std::slice::from_raw_parts(data, bytes);
                    stats.push(buf, frames, flags, fmt.nChannels, fmt.wBitsPerSample, fmt.wFormatTag);
                }
                capture.ReleaseBuffer(frames)?;
                packet = capture.GetNextPacketSize()?;
            }
        }
        if Instant::now() >= next_flush {
            stats.flush();
            next_flush += Duration::from_millis(250);
        }
    }
    stats.flush();
    client.Stop()?;
    println!("procloop done");
    Ok(())
}

// ------------------------------------------------------------------ sessions / mute

unsafe fn resolve_device(device_hint: Option<String>) -> Result<IMMDevice> {
    match device_hint {
        Some(g) => {
            let inner = g.trim_start_matches('{').trim_end_matches('}');
            let full = format!("{{0.0.0.00000000}}.{{{}}}", inner);
            let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            enumerator.GetDevice(&HSTRING::from(full))
        }
        None => default_render_device(),
    }
}

unsafe fn each_session(device_hint: Option<String>, mut f: impl FnMut(u32, IAudioSessionControl) -> Result<()>) -> Result<()> {
    let device = resolve_device(device_hint)?;
    let mgr: IAudioSessionManager2 = device_activate(&device)?;
    let sessions: IAudioSessionEnumerator = mgr.GetSessionEnumerator()?;
    let count = sessions.GetCount()?;
    for i in 0..count {
        let ctl: IAudioSessionControl = sessions.GetSession(i)?;
        let ctl2: IAudioSessionControl2 = ctl.cast()?;
        let pid = ctl2.GetProcessId()?;
        f(pid, ctl)?;
    }
    Ok(())
}

unsafe fn cmd_sessions(device_hint: Option<String>, filter_pid: Option<u32>) -> Result<()> {
    let device = resolve_device(device_hint.clone())?;
    let id = device.GetId()?;
    println!("sessions on endpoint: {}", id.to_string()?);
    each_session(device_hint, |pid, ctl| {
        if let Some(want) = filter_pid {
            if pid != want {
                return Ok(());
            }
        }
        let ctl2: IAudioSessionControl2 = ctl.cast()?;
        let ident = match ctl2.GetSessionIdentifier() {
            Ok(p) => p.to_string().unwrap_or_else(|_| "?".into()),
            Err(_) => "?".into(),
        };
        let simple: ISimpleAudioVolume = ctl.cast()?;
        let vol = simple.GetMasterVolume().unwrap_or(-1.0);
        let mute = simple.GetMute().unwrap_or(BOOL(0)).as_bool();
        println!("SESSION pid={} vol={:.2} mute={} id={}", pid, vol, mute, ident);
        Ok(())
    })
}

unsafe fn cmd_mute(pid: u32, on: bool) -> Result<()> {
    let mut found = 0;
    each_session(None, |spid, ctl| {
        if spid == pid {
            let simple: ISimpleAudioVolume = ctl.cast()?;
            simple.SetMute(BOOL(on as i32), null())?;
            found += 1;
        }
        Ok(())
    })?;
    println!("mute pid={} -> {} (sessions touched: {})", pid, on, found);
    Ok(())
}

unsafe fn cmd_epmute(arg: &str, vol: f32) -> Result<()> {
    let device = default_render_device()?;
    let epv: IAudioEndpointVolume = device_activate(&device)?;
    match &*arg {
        "--query" => {
            let m = epv.GetMute()?;
            println!("endpoint mute: {}", m.as_bool());
        }
        "--vol" => {
            let v = vol;
            epv.SetMasterVolumeLevelScalar(v, null())?;
            println!("endpoint volume set: {}", v);
        },
        s => {
            let on = s == "--on";
            epv.SetMute(BOOL(on as i32), null())?;
            println!("endpoint mute set: {}", on);
        },
        _ => println!("unknown epmute arg"),
    }
    Ok(())
}

// ------------------------------------------------------------------ hidden per-app routing

// Windows.Media.Internal.AudioPolicyConfig (cloaked WinRT static class, the
// engine behind Settings' per-app device preference). Build 26300 exposes a NEW
// IID variant ({AB3D4630-...}) unknown to EarTrumpet/SoundSwitch, so locate the
// target via dynamic GetIids and probe vtable slots in crash-isolated children.
unsafe fn policy_target() -> Result<(*mut std::ffi::c_void, *mut *mut std::ffi::c_void)> {
    use std::ffi::c_void;
    type QiFn = unsafe extern "system" fn(*mut c_void, *const windows::core::GUID, *mut *mut c_void) -> HRESULT;
    type GetIidsFn = unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut windows::core::GUID) -> HRESULT;

    let name = HSTRING::from("Windows.Media.Internal.AudioPolicyConfig");
    let factory: windows::core::IInspectable = RoGetActivationFactory(&name)?;
    let fptr = factory.as_raw();
    let fvtbl = *(fptr as *mut *mut *mut c_void);
    let get_iids: GetIidsFn = std::mem::transmute(*fvtbl.add(3));
    let mut count = 0u32;
    let mut iids: *mut windows::core::GUID = null_mut();
    let hr = get_iids(fptr, &mut count, &mut iids);
    if !hr.is_ok() || count == 0 {
        return Err(windows::core::Error::from(hr));
    }
    let last = *iids.offset(count as isize - 1);
    let bytes: [u8; 16] = std::mem::transmute(last);
    println!("interface count={count} last IID bytes={bytes:02x?}");
    CoTaskMemFree(Some(iids as *const c_void));

    let qi: QiFn = std::mem::transmute(*fvtbl);
    let mut target: *mut c_void = null_mut();
    let hr = qi(fptr, &last, &mut target);
    if !hr.is_ok() {
        return Err(windows::core::Error::from(hr));
    }
    let tvtbl = *(target as *mut *mut *mut c_void);
    Ok((target, tvtbl))
}

unsafe fn cmd_probeget(pid: u32, slot: usize) -> Result<()> {
    use std::ffi::c_void;
    type GetFn = unsafe extern "system" fn(*mut c_void, u32, i32, i32, *mut windows::core::HSTRING) -> HRESULT;
    let (target, tvtbl) = policy_target()?;
    let get_fn: GetFn = std::mem::transmute(*tvtbl.add(slot));
    let mut out: windows::core::HSTRING = windows::core::HSTRING::default();
    let hr = get_fn(target, pid, 0, 0, &mut out);
    println!("PROBE slot={slot} hr={:#x}", hr.0);
    if hr.is_ok() {
        use windows::Win32::System::WinRT::WindowsGetStringRawBuffer;
        let mut len = 0u32;
        let p = WindowsGetStringRawBuffer(&out, Some(&mut len));
        let s = p.to_string().unwrap_or_default();
        println!("GOT: {}", s);
    }
    Ok(())
}

unsafe fn cmd_pref(pid: u32, device_guid: &str, slot: usize) -> Result<()> {
    use std::ffi::c_void;
    type SetFn = unsafe extern "system" fn(*mut c_void, u32, i32, i32, *const c_void, u64) -> HRESULT;
    let (target, tvtbl) = policy_target()?;
    let set_fn: SetFn = std::mem::transmute(*tvtbl.add(slot));

    let inner = device_guid.trim_start_matches('{').trim_end_matches('}');
    let id_str = if std::env::args().any(|a| a == "--long") {
        let bs = "\\";
        format!("{0}{0}?{0}SWD#MMDEVAPI#{{0.0.0.00000000}}.{{{1}}}#{{e6327cad-dcec-4949-ae8a-991e976a79d2}}", bs, inner)
    } else {
        format!("{{0.0.0.00000000}}.{{{}}}", inner)
    };
    let device_id = HSTRING::from(id_str);
    // HSTRING is #[repr(transparent)] over the handle: transmute the VALUE, not &.
    let dev_raw: *const c_void = std::mem::transmute::<HSTRING, *const c_void>(device_id.clone());
    let extra: u64 = std::env::args().any(|a| a == "--extra-hstring") as u64;
    let extra = if extra == 1 { dev_raw as u64 } else { 0 };
    for role in [0i32, 1i32] {
        let hr = set_fn(target, pid, 0, role, dev_raw, extra);
        println!("SetPersistedDefaultAudioEndpoint(pid={pid}, render, role={role}) -> {:#x}", hr.0);
        if !hr.is_ok() {
            return Err(windows::core::Error::from(hr));
        }
    }
    println!("OK");
    Ok(())
}

// Dump policy object vtable slots with module RVA + prologue bytes for disasm matching.
unsafe fn cmd_vtdump() -> Result<()> {
    use std::ffi::c_void;
    let (target, tvtbl) = policy_target()?;
    for slot in 18..=30 {
        let f = *tvtbl.add(slot);
        if f.is_null() {
            println!("VT slot={slot} null");
            continue;
        }
        let mut base = 0usize;
        let k32 = windows::Win32::System::LibraryLoader::GetModuleHandleExW(
            windows::Win32::System::LibraryLoader::GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                | windows::Win32::System::LibraryLoader::GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            windows::core::PCWSTR(f as *const u16),
            &mut base as *mut usize as *mut windows::Win32::Foundation::HMODULE,
        );
        let mbase = if k32.is_ok() { base } else { 0 };
        let bytes = std::slice::from_raw_parts(f as *const u8, 16);
        println!("VT slot={slot} addr={:p} rva={:#x} bytes={:02x?}", f, (f as usize) - mbase, bytes);
    }
    Ok(())
}

// ------------------------------------------------------------------ openproc

unsafe fn cmd_openproc() -> Result<()> {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?;
    let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..zeroed() };
    let mut audiodg: Option<u32> = None;
    if Process32FirstW(snap, &mut entry).is_ok() {
        loop {
            let name = String::from_utf16_lossy(&entry.szExeFile).trim_end_matches('\0').to_string();
            if name.eq_ignore_ascii_case("audiodg.exe") {
                audiodg = Some(entry.th32ProcessID);
                break;
            }
            if !Process32NextW(snap, &mut entry).is_ok() {
                break;
            }
        }
    }
    let Some(pid) = audiodg else {
        println!("audiodg not found");
        return Ok(());
    };
    println!("audiodg pid={}", pid);
    let rights = [
        ("QUERY_LIMITED", PROCESS_QUERY_LIMITED_INFORMATION),
        ("VM_READ", PROCESS_VM_READ),
        ("VM_WRITE|VM_OPERATION|CREATE_THREAD (inject)", PROCESS_VM_WRITE | PROCESS_VM_OPERATION | PROCESS_CREATE_THREAD),
    ];
    for (name, right) in rights {
        match OpenProcess(right, false, pid) {
            Ok(_) => println!("OpenProcess {} -> OK", name),
            Err(e) => println!("OpenProcess {} -> DENIED ({:#x})", name, e.code().0),
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ main

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("help");
    let flag = |name: &str| -> Option<String> {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
    };

    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok().expect("CoInitializeEx");
        let result: Result<()> = (|| match cmd {
            "default" => cmd_default(),
            "tone" => cmd_tone(
                flag("--freq").and_then(|v| v.parse().ok()).unwrap_or(997.0),
                flag("--dur").and_then(|v| v.parse().ok()).unwrap_or(5.0),
            ),
            "loopback" => cmd_loopback(
                flag("--freq").and_then(|v| v.parse().ok()).unwrap_or(997.0),
                flag("--dur").and_then(|v| v.parse().ok()).unwrap_or(5.0),
            ),
            "procloop" => cmd_procloop(
                flag("--pid").and_then(|v| v.parse().ok()).expect("--pid"),
                flag("--freq").and_then(|v| v.parse().ok()).unwrap_or(997.0),
                flag("--dur").and_then(|v| v.parse().ok()).unwrap_or(5.0),
                args.iter().any(|a| a == "--exclude"),
            ),
            "sessions" => cmd_sessions(flag("--device"), flag("--pid").and_then(|v| v.parse().ok())),
            "mute" => cmd_mute(
                flag("--pid").and_then(|v| v.parse().ok()).expect("--pid"),
                args.iter().any(|a| a == "--on"),
            ),
            "epmute" => cmd_epmute(
                args.get(2).map(String::as_str).unwrap_or("--query"),
                args.get(3).and_then(|x| x.parse().ok()).unwrap_or(0.0),
            ),
            "pref" => cmd_pref(
                flag("--pid").and_then(|v| v.parse().ok()).expect("--pid"),
                flag("--device").expect("--device").as_str(),
                flag("--slot").and_then(|v| v.parse().ok()).unwrap_or(25),
            ),
            "vtdump" => cmd_vtdump(),
            "probeget" => cmd_probeget(
                flag("--pid").and_then(|v| v.parse().ok()).expect("--pid"),
                flag("--slot").and_then(|v| v.parse().ok()).expect("--slot"),
            ),
            "openproc" => cmd_openproc(),
            _ => {
                println!("usage: lab <default|tone|loopback|procloop|sessions|mute|epmute|pref|openproc> [flags]");
                Ok(())
            }
        })();
        if let Err(e) = result {
            eprintln!("ERROR: {} ({:#x})", e.message(), e.code().0);
            std::process::exit(1);
        }
        CoUninitialize();
    }
}
