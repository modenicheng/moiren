//! Endpoint discovery is control-side work; it never opens a render stream.
use serde::Serialize;

use super::api;
use crate::{
    catalog,
    owner::Apartment,
    render::{CHANNELS, RenderError, SAMPLE_RATE},
};
use windows::Win32::Media::Multimedia::WAVE_FORMAT_IEEE_FLOAT;

#[derive(Debug, Serialize)]
pub struct RenderEndpoint {
    pub endpoint_id: String,
    pub name: Option<String>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub format_supported: Option<bool>,
    pub errors: Vec<String>,
}
/// Read-only catalog; does not choose a default endpoint or modify sessions.
pub fn list_render_endpoints() -> Result<Vec<RenderEndpoint>, RenderError> {
    let _apartment = api("CoInitializeEx(catalog)", Apartment::new())?;
    let snapshot = api("render endpoint catalog", catalog::render_snapshot())?;
    Ok(project_render_endpoints(snapshot))
}
pub(super) fn project_render_endpoints(snapshot: catalog::CatalogSnapshot) -> Vec<RenderEndpoint> {
    let mut endpoints: Vec<_> = snapshot
        .endpoints
        .into_iter()
        .filter(|e| e.flow == "render")
        .map(|e| {
            let supported = e.mix_format.as_ref().map(|f| {
                let float = f.format_tag == WAVE_FORMAT_IEEE_FLOAT as u16
                    || f.sub_format.as_ref().is_some_and(|g| {
                        g.eq_ignore_ascii_case("00000003-0000-0010-8000-00aa00389b71")
                    });
                float
                    && f.sample_rate == SAMPLE_RATE
                    && f.channels == CHANNELS as u16
                    && f.container_bits == 32
                    && f.block_align == 8
                    && f.valid_bits.is_none_or(|bits| bits == 32)
                    && f.channel_mask.is_none_or(|mask| mask == 0 || mask == 3)
            });
            RenderEndpoint {
                endpoint_id: e.id,
                name: e.name,
                sample_rate: e.mix_format.as_ref().map(|f| f.sample_rate),
                channels: e.mix_format.as_ref().map(|f| f.channels),
                format_supported: supported,
                errors: e
                    .errors
                    .into_iter()
                    .map(|error| format!("{}: {}", error.stage, error.hresult))
                    .collect(),
            }
        })
        .collect();
    // A device that failed before its ID was read cannot be selected, but its
    // original failure must remain visible alongside usable endpoints.
    for error in snapshot.errors {
        endpoints.push(RenderEndpoint {
            endpoint_id: String::new(),
            name: None,
            sample_rate: None,
            channels: None,
            format_supported: None,
            errors: vec![format!("{}: {}", error.stage, error.hresult)],
        });
    }
    endpoints
}
