//! Owned, query-only metadata. No native interfaces escape the query owner.

use super::{AsioDriver, BackendError};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AsioCapabilities {
    /// Canonical registry CLSID and registration description.
    pub driver: AsioDriver,
    /// The initialized driver's own bounded name, not its registry description.
    pub driver_name: String,
    pub driver_version: i32,
    pub inputs: Vec<AsioChannel>,
    pub outputs: Vec<AsioChannel>,
    pub buffer: AsioBufferCapabilities,
    pub sample_rates: [AsioRateSupport; 4],
    /// None means the driver cannot currently report its rate (no clock or
    /// optional query not present), not that the rate is zero.
    pub current_sample_rate: Option<f64>,
    /// Driver-reported frames only: not bridge loss or measured round-trip IO.
    /// Some drivers cannot report latency before buffer creation.
    pub latencies: Option<AsioLatencies>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AsioChannel {
    pub index: u32,
    pub name: String,
    pub sample_type: AsioSampleType,
    pub is_active: bool,
    pub group: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AsioBufferCapabilities {
    pub min_frames: u32,
    pub max_frames: u32,
    pub preferred_frames: u32,
    /// Native ASIO rule: -1 doubles from minimum; positive values are steps.
    /// For zero granularity, this host accepts only the in-range reported
    /// preferred size, even when the driver reports a variable min/max range.
    pub granularity: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AsioRateSupport {
    pub sample_rate: u32,
    pub supported: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AsioLatencies {
    pub input_frames: u32,
    pub output_frames: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AsioSampleType {
    Int16Msb,
    Int24Msb,
    Int32Msb,
    Float32Msb,
    Float64Msb,
    Int32Msb16,
    Int32Msb18,
    Int32Msb20,
    Int32Msb24,
    Int16Lsb,
    Int24Lsb,
    Int32Lsb,
    Float32Lsb,
    Float64Lsb,
    Int32Lsb16,
    Int32Lsb18,
    Int32Lsb20,
    Int32Lsb24,
    DsdInt8Lsb1,
    DsdInt8Msb1,
    DsdInt8Ner8,
    /// Preserve unfamiliar native types; discovery does not imply that the
    /// stereo PCM stream implementation can transfer this channel's samples.
    Unknown(i32),
}

impl From<i32> for AsioSampleType {
    fn from(value: i32) -> Self {
        match value {
            0 => Self::Int16Msb,
            1 => Self::Int24Msb,
            2 => Self::Int32Msb,
            3 => Self::Float32Msb,
            4 => Self::Float64Msb,
            8 => Self::Int32Msb16,
            9 => Self::Int32Msb18,
            10 => Self::Int32Msb20,
            11 => Self::Int32Msb24,
            16 => Self::Int16Lsb,
            17 => Self::Int24Lsb,
            18 => Self::Int32Lsb,
            19 => Self::Float32Lsb,
            20 => Self::Float64Lsb,
            24 => Self::Int32Lsb16,
            25 => Self::Int32Lsb18,
            26 => Self::Int32Lsb20,
            27 => Self::Int32Lsb24,
            32 => Self::DsdInt8Lsb1,
            33 => Self::DsdInt8Msb1,
            40 => Self::DsdInt8Ner8,
            other => Self::Unknown(other),
        }
    }
}

/// Initialize, query and release the exact registered 64-bit driver on a
/// dedicated COM owner thread, sharing the stream's process-wide lease.
///
/// This is an opt-in native driver activation, not registry-only enumeration.
/// It never sets a rate, creates buffers, starts IO or publishes callbacks.
/// Enumeration rejects negative counts or more than 256 channels per direction.
/// All native resources are gone before this function returns an owned snapshot.
/// Driver entry points are synchronous third-party code; they cannot be safely
/// cancelled or given a hard timeout by this in-process host.
pub fn query_capabilities(driver_id: &str) -> Result<AsioCapabilities, BackendError> {
    #[cfg(all(windows, target_pointer_width = "64"))]
    {
        let driver_id = driver_id.to_owned();
        std::thread::Builder::new()
            .name("moiren-asio-capabilities".into())
            .spawn(move || {
                query_with_driver(|| {
                    let clsid = super::registry::parse_id(&driver_id)?;
                    let id = super::registry::canonical_id(&clsid);
                    log::debug!(target: "moiren::audio::asio::capabilities", "query stage=registry.list enter driver={id}");
                    let identity = super::registry::list()?
                        .into_iter()
                        .find(|entry| entry.id == id)
                        .ok_or_else(|| BackendError::ResourceUnavailable(format!(
                            "ASIO driver {id} is not registered in the 64-bit SOFTWARE\\ASIO registry"
                        )))?;
                    log::debug!(target: "moiren::audio::asio::capabilities", "query stage=registry.list exit");
                    log::debug!(target: "moiren::audio::asio::capabilities", "query stage=apartment.new enter");
                    let apartment = Apartment::new()?;
                    log::debug!(target: "moiren::audio::asio::capabilities", "query stage=apartment.new exit");
                    log::debug!(target: "moiren::audio::asio::capabilities", "query stage=CoCreateInstance enter");
                    let driver = Driver::open(&clsid)?;
                    log::debug!(target: "moiren::audio::asio::capabilities", "query stage=CoCreateInstance exit");
                    Ok(QueryDriver {
                        driver,
                        _apartment: Some(apartment),
                        identity,
                    })
                })
            })
            .map_err(|error| BackendError::Failed(format!("ASIO query owner spawn failed: {error}")))?
            .join()
            .map_err(|_| BackendError::Failed("ASIO query owner panicked".into()))?
    }
    #[cfg(not(all(windows, target_pointer_width = "64")))]
    {
        let _ = driver_id;
        Err(super::unsupported_platform())
    }
}

#[cfg(all(windows, target_pointer_width = "64"))]
use super::driver::{Apartment, Driver};

#[cfg(all(windows, target_pointer_width = "64"))]
struct QueryDriver {
    // Fields drop in declaration order: Release before CoUninitialize. The
    // surrounding query scope keeps the lease alive through both, even on Err.
    driver: Driver,
    _apartment: Option<Apartment>,
    identity: AsioDriver,
}

#[cfg(all(windows, target_pointer_width = "64"))]
fn query_with_driver(
    open: impl FnOnce() -> Result<QueryDriver, BackendError>,
) -> Result<AsioCapabilities, BackendError> {
    log::debug!(target: "moiren::audio::asio::capabilities", "query stage=lease.acquire enter");
    let lease = super::callback::Exclusive::acquire()?;
    log::debug!(target: "moiren::audio::asio::capabilities", "query stage=lease.acquire exit");
    log::debug!(target: "moiren::audio::asio::capabilities", "query stage=driver.open enter");
    let session = open()?;
    log::debug!(target: "moiren::audio::asio::capabilities", "query stage=driver.open exit");
    let result = (|| {
        log::debug!(target: "moiren::audio::asio::capabilities", "query stage=driver.init enter");
        let result = session.driver.initialize();
        log::debug!(target: "moiren::audio::asio::capabilities", "query stage=driver.init exit ok={}", result.is_ok());
        result?;
        session.snapshot()
    })();
    // Explicitly bracket the original field drop order without changing its
    // owner-thread or lease contract. Errors still run every teardown stage.
    let QueryDriver {
        driver,
        _apartment: apartment,
        ..
    } = session;
    log::debug!(target: "moiren::audio::asio::capabilities", "query stage=driver.Release enter");
    drop(driver);
    log::debug!(target: "moiren::audio::asio::capabilities", "query stage=driver.Release exit");
    log::debug!(target: "moiren::audio::asio::capabilities", "query stage=apartment.drop enter");
    drop(apartment);
    log::debug!(target: "moiren::audio::asio::capabilities", "query stage=apartment.drop exit");
    log::debug!(target: "moiren::audio::asio::capabilities", "query stage=lease.drop enter");
    drop(lease);
    log::debug!(target: "moiren::audio::asio::capabilities", "query stage=lease.drop exit ok={}", result.is_ok());
    result
}

#[cfg(all(windows, target_pointer_width = "64"))]
impl QueryDriver {
    fn snapshot(&self) -> Result<AsioCapabilities, BackendError> {
        use super::{abi, buffer::BufferSizes};

        let driver = &self.driver;
        let raw = driver.raw();
        let vt = driver.vtable();
        let mut name = [0u8; 32];
        let mut inputs = 0;
        let mut outputs = 0;
        let mut buffer = BufferSizes {
            min: 0,
            max: 0,
            preferred: 0,
            granularity: 0,
        };
        let mut current_rate = 0.0;
        let mut input_latency = 0;
        let mut output_latency = 0;

        // SAFETY: initialized IASIO reference and all output storage live only
        // on this owner thread. Fixed strings follow the existing private ABI;
        // every return code/count is checked before using variable-sized data.
        let (version, current_code, latency_code) = unsafe {
            log::debug!(target: "moiren::audio::asio::capabilities", "query stage=getDriverName enter");
            (vt.get_driver_name)(raw, name.as_mut_ptr());
            log::debug!(target: "moiren::audio::asio::capabilities", "query stage=getDriverName exit");
            log::debug!(target: "moiren::audio::asio::capabilities", "query stage=getDriverVersion enter");
            let version = (vt.get_driver_version)(raw);
            log::debug!(target: "moiren::audio::asio::capabilities", "query stage=getDriverVersion exit version={version}");
            log::debug!(target: "moiren::audio::asio::capabilities", "query stage=getChannels enter");
            let code = (vt.get_channels)(raw, &mut inputs, &mut outputs);
            log::debug!(target: "moiren::audio::asio::capabilities", "query stage=getChannels exit code={code} inputs={inputs} outputs={outputs}");
            driver.check("getChannels", code)?;
            const MAX_CHANNELS_PER_DIRECTION: i32 = 256;
            if !(0..=MAX_CHANNELS_PER_DIRECTION).contains(&inputs)
                || !(0..=MAX_CHANNELS_PER_DIRECTION).contains(&outputs)
            {
                return Err(BackendError::Failed(format!(
                    "ASIO getChannels returned invalid counts: inputs={inputs}, outputs={outputs}; limit=256 per direction"
                )));
            }
            log::debug!(target: "moiren::audio::asio::capabilities", "query stage=getBufferSize enter");
            let code = (vt.get_buffer_size)(
                raw,
                &mut buffer.min,
                &mut buffer.max,
                &mut buffer.preferred,
                &mut buffer.granularity,
            );
            log::debug!(target: "moiren::audio::asio::capabilities", "query stage=getBufferSize exit code={code}");
            driver.check("getBufferSize", code)?;
            // Validate the driver's preferred size without imposing the stream
            // menu's 64..=4096/power-of-two policy on metadata discovery.
            let preferred = u32::try_from(buffer.preferred).unwrap_or(0);
            buffer.validate(preferred)?;
            log::debug!(target: "moiren::audio::asio::capabilities", "query stage=getSampleRate enter");
            let current_code = (vt.get_sample_rate)(raw, &mut current_rate);
            log::debug!(target: "moiren::audio::asio::capabilities", "query stage=getSampleRate exit code={current_code} rate={current_rate}");
            log::debug!(target: "moiren::audio::asio::capabilities", "query stage=getLatencies enter");
            let latency_code = (vt.get_latencies)(raw, &mut input_latency, &mut output_latency);
            log::debug!(target: "moiren::audio::asio::capabilities", "query stage=getLatencies exit code={latency_code} input={input_latency} output={output_latency}");
            (version, current_code, latency_code)
        };

        // ASE_NotPresent and ASE_NoClock make the current rate unavailable.
        let current_sample_rate = match current_code {
            -997 | -995 => None,
            code => {
                driver.check("getSampleRate", code)?;
                if !current_rate.is_finite() || current_rate <= 0.0 {
                    return Err(BackendError::Failed(format!(
                        "ASIO getSampleRate returned invalid rate {current_rate}"
                    )));
                }
                Some(current_rate)
            }
        };
        let latencies = match latency_code {
            -997 => None, // ASE_NotPresent, often until buffers exist.
            code => {
                driver.check("getLatencies", code)?;
                if input_latency < 0 || output_latency < 0 {
                    return Err(BackendError::Failed(format!(
                        "ASIO getLatencies returned negative frames: input={input_latency}, output={output_latency}"
                    )));
                }
                Some(AsioLatencies {
                    input_frames: input_latency as u32,
                    output_frames: output_latency as u32,
                })
            }
        };

        let mut sample_rates = [44100, 48000, 88200, 96000].map(|sample_rate| AsioRateSupport {
            sample_rate,
            supported: false,
        });
        for candidate in &mut sample_rates {
            // ASE_NoClock denotes an unsupported candidate, not a failed
            // activation. All other errors retain the driver's typed failure.
            log::debug!(target: "moiren::audio::asio::capabilities", "query stage=canSampleRate enter rate={}", candidate.sample_rate);
            let code = unsafe { (vt.can_sample_rate)(raw, f64::from(candidate.sample_rate)) };
            log::debug!(target: "moiren::audio::asio::capabilities", "query stage=canSampleRate exit code={code}");
            candidate.supported = match code {
                abi::OK => true,
                -995 => false,
                code => {
                    driver.check("canSampleRate", code)?;
                    unreachable!()
                }
            };
        }
        Ok(AsioCapabilities {
            driver: self.identity.clone(),
            driver_name: bounded_name(&name),
            driver_version: version,
            inputs: self.channels(inputs, true)?,
            outputs: self.channels(outputs, false)?,
            buffer: AsioBufferCapabilities {
                min_frames: buffer.min as u32,
                max_frames: buffer.max as u32,
                preferred_frames: buffer.preferred as u32,
                granularity: buffer.granularity,
            },
            sample_rates,
            current_sample_rate,
            latencies,
        })
    }

    fn channels(&self, count: i32, input: bool) -> Result<Vec<AsioChannel>, BackendError> {
        let mut channels = Vec::with_capacity(count as usize);
        for index in 0..count {
            let mut info = super::abi::ChannelInfo {
                channel: index,
                is_input: i32::from(input),
                ..Default::default()
            };
            // SAFETY: count was bounded before enumeration; each packed ABI
            // descriptor is initialized, owner-local and read by value below.
            log::debug!(target: "moiren::audio::asio::capabilities", "query stage=getChannelInfo enter input={input} index={index}");
            let code =
                unsafe { (self.driver.vtable().get_channel_info)(self.driver.raw(), &mut info) };
            log::debug!(target: "moiren::audio::asio::capabilities", "query stage=getChannelInfo exit code={code}");
            self.driver.check("getChannelInfo", code)?;
            channels.push(AsioChannel {
                index: index as u32,
                name: bounded_name(&info.name),
                sample_type: info.sample_type.into(),
                is_active: info.is_active != 0,
                group: info.channel_group,
            });
        }
        Ok(channels)
    }
}

#[cfg(all(windows, target_pointer_width = "64"))]
fn bounded_name(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

#[cfg(all(test, windows, target_pointer_width = "64"))]
#[path = "capability_tests.rs"]
mod tests;
