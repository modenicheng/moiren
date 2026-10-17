//! Read-only capture choices, independent of output/default endpoint presence.
use super::api;
use crate::{
    capture::CaptureError, catalog, owner::Apartment, shared_stream::format::catalog_support,
};
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
pub fn list_capture_endpoints() -> Result<Vec<CaptureEndpoint>, CaptureError> {
    let _apartment = api("CoInitializeEx(capture catalog)", Apartment::new())?;
    let snapshot = api("capture endpoint catalog", catalog::capture_snapshot())?;
    let mut endpoints: Vec<_> = snapshot
        .endpoints
        .into_iter()
        .map(|e| {
            let (supported, windows_conversion_required) = e
                .mix_format
                .as_ref()
                .map_or((None, false), |format| catalog_support(format, true));
            CaptureEndpoint {
                endpoint_id: e.id,
                name: e.name,
                sample_rate: e.mix_format.as_ref().map(|f| f.sample_rate),
                channels: e.mix_format.as_ref().map(|f| f.channels),
                format_supported: supported,
                windows_conversion_required,
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
            windows_conversion_required: false,
            errors: vec![format!("{}: {}", e.stage, e.hresult)],
        });
    }
    Ok(endpoints)
}
