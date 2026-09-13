use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use project_workspace_core::{FileIdentity, validate_relative};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

pub(crate) fn plain(path: &Path) -> Result<fs::Metadata> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("Cannot read {}", path.display()))?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "Symbolic links are not allowed: {}",
        path.display()
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "Reparse points are not allowed: {}",
            path.display()
        );
    }
    ensure!(
        metadata.is_file() || metadata.is_dir(),
        "Unsupported file type: {}",
        path.display()
    );
    Ok(metadata)
}

/// Check every existing ancestor, not just the leaf (junctions included).
pub(crate) fn canonical_plain(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    for ancestor in absolute.ancestors() {
        plain(ancestor)?;
    }
    Ok(fs::canonicalize(absolute)?)
}

pub(crate) fn contained(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_relative(relative)?;
    let mut path = root.to_path_buf();
    for part in relative.split('/') {
        path.push(part);
        plain(&path)?;
    }
    Ok(path)
}

pub(crate) fn json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    ensure!(
        plain(path)?.len() <= 16 * 1_048_576,
        "JSON metadata exceeds 16 MiB: {}",
        path.display()
    );
    serde_json::from_reader(File::open(path)?)
        .with_context(|| format!("Invalid JSON metadata: {}", path.display()))
}

pub(crate) fn hash(path: &Path, relative: &str) -> Result<FileIdentity> {
    ensure!(
        plain(path)?.is_file(),
        "Expected a regular file: {}",
        path.display()
    );
    let mut reader = File::open(path)?;
    let total = reader.metadata()?.len();
    crate::progress::file_progress(path, 0, total);
    let mut hasher = Sha256::new();
    let mut bytes = 0;
    let mut buffer = [0_u8; 65_536];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        bytes += n as u64;
        if bytes % (8 * 1024 * 1024) == 0 {
            crate::progress::file_progress(path, bytes, total);
        }
    }
    crate::progress::file_progress(path, bytes, total);
    Ok(FileIdentity {
        path: relative.into(),
        bytes,
        fingerprint: format!("sha256:{:x}", hasher.finalize()),
    })
}

pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

pub(crate) fn copy_verified(
    source: &Path,
    destination: &Path,
    identity: &FileIdentity,
) -> Result<()> {
    ensure!(plain(source)?.is_file(), "Source must be a regular file.");
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    ensure!(
        hash(destination, &identity.path)? == *identity
            && hash(source, &identity.path)? == *identity,
        "Source changed during copying, or copy is corrupt: {}",
        source.display()
    );
    Ok(())
}
