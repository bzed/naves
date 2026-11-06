// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The editable project settings: validated on every change and saved to `naves.toml` when valid.

use std::path::{Path, PathBuf};

use naves_core::project::{Project, ProjectError};

/// The project being edited.
pub struct SettingsForm {
    dir: PathBuf,
    /// The values shown in the form; may be invalid while the user is typing.
    pub project: Project,
    /// Why the current values were not saved, in the user's words.
    pub error: Option<String>,
}

impl SettingsForm {
    /// Loads `<dir>/naves.toml`.
    ///
    /// # Errors
    ///
    /// Whatever [`Project::load`] reports.
    pub fn load(dir: &Path) -> Result<Self, ProjectError> {
        Ok(Self {
            dir: dir.to_path_buf(),
            project: Project::load(dir)?,
            error: None,
        })
    }

    /// The project folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Call after a widget changed a value: saves the project if it is valid, otherwise keeps the file as
    /// it is and records the reason in [`SettingsForm::error`]. Returns whether the file was written.
    pub fn commit(&mut self) -> bool {
        match self.project.save(&self.dir) {
            Ok(()) => {
                self.error = None;
                true
            }
            Err(error) => {
                self.error = Some(error.to_string());
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> (tempfile::TempDir, SettingsForm) {
        let dir = tempfile::tempdir().unwrap();
        Project::new("T", 47.5, 12.9, 2000.0)
            .save(dir.path())
            .unwrap();
        let form = SettingsForm::load(dir.path()).unwrap();
        (dir, form)
    }

    #[test]
    fn a_valid_edit_is_saved() {
        let (dir, mut form) = form();
        form.project.scale.height = 0.75;
        assert!(form.commit());
        assert!(form.error.is_none());
        assert!((Project::load(dir.path()).unwrap().scale.height - 0.75).abs() < 1e-9);
    }

    #[test]
    fn an_invalid_edit_keeps_the_file_and_explains() {
        let (dir, mut form) = form();
        let before = std::fs::read(dir.path().join("naves.toml")).unwrap();
        form.project.scale.horizontal = 3.0;
        assert!(!form.commit());
        assert!(form.error.as_deref().unwrap().contains("horizontal"));
        assert_eq!(
            std::fs::read(dir.path().join("naves.toml")).unwrap(),
            before
        );
        // fixing the value clears the message and saves
        form.project.scale.horizontal = 0.5;
        assert!(form.commit() && form.error.is_none());
    }

    #[test]
    fn loading_a_missing_project_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(SettingsForm::load(dir.path()).is_err());
    }
}
