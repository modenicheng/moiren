use std::{fs, io, path::Path};

use serde::{Deserialize, Serialize};

use super::{PersistenceError, WorkspaceDocument, preferences_path, storage};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WindowState {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
    pub workspace: WorkspaceDocument,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            x: 80.0,
            y: 80.0,
            width: 1440.0,
            height: 900.0,
            maximized: false,
            workspace: WorkspaceDocument::Graph,
        }
    }
}

impl WindowState {
    pub fn validate(&self) -> Result<(), PersistenceError> {
        if ![self.x, self.y, self.width, self.height]
            .into_iter()
            .all(f32::is_finite)
            || self.width < 320.0
            || self.height < 240.0
            || self.width > 32768.0
            || self.height > 32768.0
            || self.x.abs() > 131072.0
            || self.y.abs() > 131072.0
        {
            return Err(PersistenceError::invalid(
                "window.bounds",
                "invalid window geometry",
            ));
        }
        Ok(())
    }
}

pub fn load_window_state_at(path: impl AsRef<Path>) -> Result<WindowState, PersistenceError> {
    match storage::read_json::<WindowState>(path.as_ref()) {
        Ok(state) => {
            state.validate()?;
            Ok(state)
        }
        Err(PersistenceError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(WindowState::default())
        }
        Err(error) => Err(error),
    }
}

pub fn save_window_state_at(
    path: impl AsRef<Path>,
    state: &WindowState,
) -> Result<(), PersistenceError> {
    state.validate()?;
    let path = path.as_ref();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| PersistenceError::Io {
            operation: "create window state directory",
            path: parent.to_owned(),
            source,
        })?;
    }
    storage::write_json(path, state)
}

pub fn window_state_path() -> Result<std::path::PathBuf, PersistenceError> {
    Ok(preferences_path()?.with_file_name("window.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_state_accepts_negative_monitor_coordinates_and_maximized_restore_bounds() {
        let state = WindowState {
            x: -1440.0,
            y: -100.0,
            width: 1200.0,
            height: 800.0,
            maximized: true,
            workspace: WorkspaceDocument::Mixer,
        };
        state.validate().unwrap();
        let restored: WindowState =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(restored, state);
    }

    #[test]
    fn window_state_rejects_invalid_geometry() {
        for state in [
            WindowState {
                x: f32::NAN,
                ..Default::default()
            },
            WindowState {
                width: 0.0,
                ..Default::default()
            },
            WindowState {
                height: f32::INFINITY,
                ..Default::default()
            },
        ] {
            assert!(state.validate().is_err());
        }
    }

    #[test]
    fn window_state_storage_is_independent_of_preferences() {
        let directory = std::env::temp_dir().join(format!(
            "moiren-window-state-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("window.json");
        assert_eq!(load_window_state_at(&path).unwrap(), WindowState::default());
        let state = WindowState {
            maximized: true,
            ..Default::default()
        };
        save_window_state_at(&path, &state).unwrap();
        assert_eq!(load_window_state_at(&path).unwrap(), state);
        std::fs::write(&path, b"not json").unwrap();
        assert!(load_window_state_at(&path).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
