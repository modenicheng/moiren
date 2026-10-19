//! Read-only capture choices, independent of output/default endpoint presence.
use super::api;
use crate::{capture::CaptureError, catalog, owner::Apartment};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CaptureEndpoint {
    pub endpoint_id: String,
    pub name: Option<String>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub format_supported: Option<bool>,
    /// True means Initialize must still verify the Windows conversion path.
    pub windows_conversion_required: bool,
    pub errors: Vec<String>,
}
impl From<catalog::EndpointRow> for CaptureEndpoint {
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
pub fn list_capture_endpoints() -> Result<Vec<CaptureEndpoint>, CaptureError> {
    let _apartment = api("CoInitializeEx(capture catalog)", Apartment::new())?;
    let snapshot = api("capture endpoint catalog", catalog::capture_snapshot())?;
    // The capture snapshot is already capture-only, so no flow filter applies;
    // mix formats are judged with the capture catalog_support direction.
    Ok(catalog::endpoint_rows(snapshot, None, true)
        .into_iter()
        .map(CaptureEndpoint::from)
        .collect())
}
