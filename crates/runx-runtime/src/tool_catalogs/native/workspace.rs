use std::path::{Path, PathBuf};

use crate::RuntimeError;

use super::invalid_input;

pub(crate) fn resolve_repo_root_for(
    tool: &str,
    requested: &str,
    env: &std::collections::BTreeMap<String, String>,
    skill_directory: &Path,
) -> Result<PathBuf, RuntimeError> {
    crate::services::resolve_scoped_root(requested, "workspace", env, skill_directory)
        .map_err(|error| invalid_input(tool, error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_filesystem_containment_rejects_absolute_roots()
    -> Result<(), Box<dyn std::error::Error>> {
        let workspace = tempfile::tempdir()?;
        let env = std::collections::BTreeMap::from([(
            crate::receipts::paths::RUNX_CWD_ENV.to_owned(),
            workspace.path().to_string_lossy().into_owned(),
        )]);

        assert_eq!(
            resolve_repo_root_for("fs.read", ".", &env, workspace.path())?,
            workspace.path().canonicalize()?
        );
        let error = resolve_repo_root_for("fs.read", "/tmp", &env, workspace.path())
            .err()
            .ok_or_else(|| std::io::Error::other("absolute root must be rejected"))?;
        assert!(error.to_string().contains("relative"));
        Ok(())
    }
}
