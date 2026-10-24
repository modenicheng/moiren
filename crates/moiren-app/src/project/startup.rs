use std::path::Path;

use super::{
    AppPreferences, PersistenceError, ProjectDocument, load_preferences_at, load_project,
    preferences_path,
};

/// All persisted intent is loaded and checked before any service/backend exists.
/// Callers may inspect errors without creating, enumerating or opening resources.
#[derive(Debug, Clone)]
pub struct StartupDocuments {
    pub preferences: AppPreferences,
    pub project: Option<ProjectDocument>,
}

impl StartupDocuments {
    pub fn load() -> Result<Self, PersistenceError> {
        Self::load_at(preferences_path()?)
    }

    pub fn load_at(path: impl AsRef<Path>) -> Result<Self, PersistenceError> {
        let preferences = load_preferences_at(path)?;
        let project = if preferences.restore_last_project {
            preferences
                .last_project_path
                .as_ref()
                .map(load_project)
                .transpose()?
        } else {
            None
        };
        Ok(Self {
            preferences,
            project,
        })
    }

    pub fn into_context(self) -> Result<crate::service::ServiceContext, String> {
        self.preferences
            .validate()
            .map_err(|error| error.to_string())?;
        let mut context = crate::service::ServiceContext::with_settings(
            self.preferences.audio,
            self.preferences.latency_profile,
        );
        context
            .exports
            .set_authorizations(&self.preferences.export_authorizations)?;
        if let Some(project) = self.project {
            context.load_project(project)?;
        }
        Ok(context)
    }
}
