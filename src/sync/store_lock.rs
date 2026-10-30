//! Cross-process exclusion for SQL updates and paired index files.

use anyhow::Context;

/// One store's exclusive lock, released when its file handle closes.
pub(crate) struct StoreLock {
    paths: crate::paths::Paths,
    _file: std::fs::File,
}

impl StoreLock {
    /// Wait for all writers and paired index loads to leave this store.
    pub(crate) fn acquire(paths: &crate::paths::Paths) -> anyhow::Result<Self> {
        let path = paths.root().join("store.lock");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("opening store lock {}", path.display()))?;
        fs2::FileExt::lock_exclusive(&file)
            .with_context(|| format!("locking store {}", paths.root().display()))?;
        Ok(Self {
            paths: paths.clone(),
            _file: file,
        })
    }

    /// Root paths protected by this guard.
    pub(crate) fn paths(&self) -> &crate::paths::Paths {
        &self.paths
    }
}
