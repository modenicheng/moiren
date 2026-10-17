//! Endpoint discovery is control-side work; it never opens a render stream.
use serde::Serialize;

use super::api;
use crate::{
    catalog, owner::Apartment, render::RenderError, shared_stream::format::catalog_support,
};

#[derive(Debug, Serialize)]
pub struct RenderEndpoint {
    pub endpoint_id: String,
    pub name: Option<String>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub format_supported: Option<bool>,
    /// Requires an actual Shared Initialize; catalog inspection is not proof.
    pub windows_conversion_required: bool,
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
            let (supported, windows_conversion_required) = e
                .mix_format
                .as_ref()
                .map_or((None, false), |format| catalog_support(format, false));
            RenderEndpoint {
                endpoint_id: e.id,
                name: e.name,
                sample_rate: e.mix_format.as_ref().map(|f| f.sample_rate),
                channels: e.mix_format.as_ref().map(|f| f.channels),
                format_supported: supported,
                windows_conversion_required,
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
            windows_conversion_required: false,
            errors: vec![format!("{}: {}", error.stage, error.hresult)],
        });
    }
    endpoints
}
