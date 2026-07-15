//! Shared, deterministic helpers for integration tests.

use std::path::Path;

use tempfile::TempDir;
use thiserror::Error;

/// Isolated test directory that is removed on drop.
#[derive(Debug)]
pub struct TestWorkspace {
    root: TempDir,
}

impl TestWorkspace {
    /// Creates an isolated workspace.
    pub fn new() -> Result<Self, TestSupportError> {
        TempDir::new()
            .map(|root| Self { root })
            .map_err(TestSupportError::Io)
    }

    /// Returns the isolated root path.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.root.path()
    }
}

/// Test support setup failure.
#[derive(Debug, Error)]
pub enum TestSupportError {
    /// Temporary filesystem setup failed.
    #[error("failed to create test workspace: {0}")]
    Io(std::io::Error),
}
