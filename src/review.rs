use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::git::Repository;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewStatus {
    #[default]
    Unreviewed,
    Viewed,
    Reviewed,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ReviewState {
    pub files: BTreeMap<PathBuf, ReviewStatus>,
}

pub struct ReviewStore {
    path: PathBuf,
}

impl ReviewStore {
    pub fn for_repository(repo: &Repository) -> Result<Self> {
        let dirs = ProjectDirs::from("dev", "igtm", "grspr")
            .context("could not determine state directory")?;
        let mut hash = Sha256::new();
        hash.update(repo.common_dir.to_string_lossy().as_bytes());
        hash.update([0]);
        hash.update(repo.merge_base_oid.as_bytes());
        hash.update([0]);
        hash.update(repo.head_oid.as_bytes());
        let id = format!("{:x}", hash.finalize());
        Ok(Self {
            path: dirs
                .data_local_dir()
                .join("reviews")
                .join(format!("{id}.json")),
        })
    }

    pub fn load(&self) -> Result<ReviewState> {
        if !self.path.exists() {
            return Ok(ReviewState::default());
        }
        let bytes = fs::read(&self.path)
            .with_context(|| format!("failed to read {}", self.path.display()))?;
        serde_json::from_slice(&bytes).context("invalid review state")
    }

    pub fn save(&self, state: &ReviewState) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(state)?)?;
        fs::rename(&temporary, &self.path)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn state_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let store = ReviewStore {
            path: directory.path().join("state.json"),
        };
        let mut state = ReviewState::default();
        state
            .files
            .insert("src/lib.rs".into(), ReviewStatus::Reviewed);
        store.save(&state).unwrap();
        assert_eq!(
            store.load().unwrap().files[Path::new("src/lib.rs")],
            ReviewStatus::Reviewed
        );
    }
}
