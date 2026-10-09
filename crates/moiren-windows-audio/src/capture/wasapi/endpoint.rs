//! Read-only capture choices, independent of output/default endpoint presence.
use super::{api, format::validate_format};
use crate::{capture::CaptureError, catalog, owner::Apartment};
use serde::Serialize;
use windows::Win32::Media::Audio::WAVEFORMATEX;

#[derive(Debug, Serialize)]
pub struct CaptureEndpoint {
    pub endpoint_id: String,
    pub name: Option<String>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub format_supported: Option<bool>,
    pub errors: Vec<String>,
}
pub fn list_capture_endpoints() -> Result<Vec<CaptureEndpoint>, CaptureError> {
    let _apartment = api("CoInitializeEx(capture catalog)", Apartment::new())?;
    let snapshot = api("capture endpoint catalog", catalog::capture_snapshot())?;
    let mut endpoints: Vec<_> = snapshot
        .endpoints
        .into_iter()
        .map(|e| {
            let supported = e.mix_format.as_ref().map(|f| {
                let float = f.format_tag == 3
                    || (f.format_tag == 0xfffe
                        && f.extra_bytes >= 22
                        && f.sub_format.as_ref().is_some_and(|g| {
                            g.eq_ignore_ascii_case("00000003-0000-0010-8000-00aa00389b71")
                        }));
                validate_format(
                    WAVEFORMATEX {
                        nSamplesPerSec: f.sample_rate,
                        nChannels: f.channels,
                        wBitsPerSample: f.container_bits,
                        nBlockAlign: f.block_align,
                        nAvgBytesPerSec: f.sample_rate.saturating_mul(u32::from(f.block_align)),
                        ..Default::default()
                    },
                    float,
                    f.valid_bits,
                    f.channel_mask,
                )
                .is_ok()
            });
            CaptureEndpoint {
                endpoint_id: e.id,
                name: e.name,
                sample_rate: e.mix_format.as_ref().map(|f| f.sample_rate),
                channels: e.mix_format.as_ref().map(|f| f.channels),
                format_supported: supported,
                errors: e
                    .errors
                    .into_iter()
                    .map(|e| format!("{}: {}", e.stage, e.hresult))
                    .collect(),
            }
        })
        .collect();
    for e in snapshot.errors {
        endpoints.push(CaptureEndpoint {
            endpoint_id: String::new(),
            name: None,
            sample_rate: None,
            channels: None,
            format_supported: None,
            errors: vec![format!("{}: {}", e.stage, e.hresult)],
        });
    }
    Ok(endpoints)
}
