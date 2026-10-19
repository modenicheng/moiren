//! Endpoint discovery is control-side work; it never opens a render stream.
use serde::Serialize;

use super::api;
use crate::{catalog, owner::Apartment, render::RenderError};

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
impl From<catalog::EndpointRow> for RenderEndpoint {
    fn from(row: catalog::EndpointRow) -> Self {
        Self {
            endpoint_id: row.endpoint_id,
            name: row.name,
            sample_rate: row.sample_rate,
            channels: row.channels,
            format_supported: row.format_supported,
            windows_conversion_required: row.windows_conversion_required,
            errors: row.errors,
        }
    }
}
/// Read-only catalog; does not choose a default endpoint or modify sessions.
pub fn list_render_endpoints() -> Result<Vec<RenderEndpoint>, RenderError> {
    let _apartment = api("CoInitializeEx(catalog)", Apartment::new())?;
    let snapshot = api("render endpoint catalog", catalog::render_snapshot())?;
    Ok(project_render_endpoints(snapshot))
}
pub(super) fn project_render_endpoints(snapshot: catalog::CatalogSnapshot) -> Vec<RenderEndpoint> {
    // The render snapshot can carry foreign-flow entries: keep only render
    // endpoints and judge mix formats with the render support direction.
    catalog::endpoint_rows(snapshot, Some("render"), false)
        .into_iter()
        .map(RenderEndpoint::from)
        .collect()
}
