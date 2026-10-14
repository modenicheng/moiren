use super::{DeviceCatalog, InputSelection, RunLimit, SessionSpec};

pub(super) fn default_session(catalog: &DeviceCatalog) -> Option<SessionSpec> {
    let input = catalog.default_input_endpoint_id.as_ref()?;
    let output = catalog.default_output_endpoint_id.as_ref()?;
    if !catalog.inputs.iter().any(|row| row.endpoint_id == *input)
        || !catalog.outputs.iter().any(|row| row.endpoint_id == *output)
    {
        return None;
    }
    let spec = SessionSpec {
        source: InputSelection::Physical {
            endpoint_id: input.clone(),
        },
        output_endpoint_id: output.clone(),
        limit: RunLimit::UntilStopped,
        gain: 0.0,
        pan: 0.0,
        max_block_frames: 256,
    };
    spec.validate().ok()?;
    Some(spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::DeviceRow;

    fn catalog() -> DeviceCatalog {
        DeviceCatalog {
            inputs: vec![DeviceRow {
                endpoint_id: "input".into(),
                name: "input".into(),
            }],
            outputs: vec![DeviceRow {
                endpoint_id: "output".into(),
                name: "output".into(),
            }],
            default_input_endpoint_id: Some("input".into()),
            default_output_endpoint_id: Some("output".into()),
            ..Default::default()
        }
    }

    #[test]
    fn default_session_is_muted_and_has_no_demo_time_limit() {
        let session = default_session(&catalog()).unwrap();
        assert_eq!(session.gain, 0.0);
        assert_eq!(session.pan, 0.0);
        assert_eq!(session.limit, RunLimit::UntilStopped);
        assert_eq!(
            session.source,
            InputSelection::Physical {
                endpoint_id: "input".into()
            }
        );
    }

    #[test]
    fn missing_or_stale_defaults_never_choose_an_arbitrary_device() {
        let mut catalog = catalog();
        catalog.default_input_endpoint_id = None;
        assert!(default_session(&catalog).is_none());
        catalog.default_input_endpoint_id = Some("vanished".into());
        assert!(default_session(&catalog).is_none());
        catalog.default_input_endpoint_id = Some("input".into());
        catalog.default_output_endpoint_id = Some("vanished".into());
        assert!(default_session(&catalog).is_none());
        catalog.default_output_endpoint_id = Some("output".into());
        catalog.outputs.clear();
        assert!(default_session(&catalog).is_none());
    }
}
