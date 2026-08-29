use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use training_core::{
    domain::StoredArtifact,
    ports::{ArtifactError, CheckpointSink},
};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct LocalCheckpointStore {
    root: PathBuf,
}

impl LocalCheckpointStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl CheckpointSink for LocalCheckpointStore {
    fn write(
        &self,
        run_id: Uuid,
        epoch: u32,
        model_format: &str,
        artifact: &[u8],
    ) -> Result<StoredArtifact, ArtifactError> {
        let directory = self.root.join(run_id.to_string());
        std::fs::create_dir_all(&directory).map_err(artifact_error)?;
        let format = model_format
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '-' {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let path = directory.join(format!("epoch-{epoch:05}-{format}.artifact"));
        if path.exists() {
            return Err(ArtifactError(format!(
                "checkpoint already exists: {}",
                path.display()
            )));
        }
        let temporary = directory.join(format!(".epoch-{epoch:05}-{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(artifact_error)?;
            file.write_all(artifact).map_err(artifact_error)?;
            file.sync_all().map_err(artifact_error)?;
            drop(file);
            // Publishing a same-directory hard link is atomic and, unlike a
            // Unix rename, can never replace an existing immutable artifact.
            std::fs::hard_link(&temporary, &path).map_err(artifact_error)?;
            let _ = std::fs::remove_file(&temporary);
            Ok(())
        })();
        if result.is_err() && temporary.is_file() {
            let _ = std::fs::remove_file(&temporary);
        }
        result?;
        Ok(StoredArtifact {
            path: path.to_string_lossy().into_owned(),
            checksum: checksum(artifact),
            size_bytes: u64::try_from(artifact.len())
                .map_err(|error| ArtifactError(error.to_string()))?,
        })
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, ArtifactError> {
        std::fs::read(path).map_err(artifact_error)
    }
}

pub fn verify_checksum(bytes: &[u8], expected: &str) -> Result<(), ArtifactError> {
    let actual = checksum(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(ArtifactError(format!(
            "checkpoint checksum mismatch: expected {expected}, calculated {actual}"
        )))
    }
}

pub fn verify_file(
    path: &str,
    expected_checksum: &str,
    expected_size: u64,
) -> Result<(), ArtifactError> {
    let mut file = std::fs::File::open(path).map_err(artifact_error)?;
    let actual_size = file.metadata().map_err(artifact_error)?.len();
    if actual_size != expected_size {
        return Err(ArtifactError(format!(
            "checkpoint size mismatch: expected {expected_size}, read {actual_size}"
        )));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(artifact_error)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let actual = format!("sha256:{:x}", digest.finalize());
    if actual == expected_checksum {
        Ok(())
    } else {
        Err(ArtifactError(format!(
            "checkpoint checksum mismatch: expected {expected_checksum}, calculated {actual}"
        )))
    }
}

fn checksum(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn artifact_error(error: impl std::fmt::Display) -> ArtifactError {
    ArtifactError(error.to_string())
}

#[cfg(test)]
mod tests {
    use training_core::ports::CheckpointSink;
    use uuid::Uuid;

    use super::{LocalCheckpointStore, verify_checksum, verify_file};

    #[test]
    fn writes_immutable_checkpoints_and_reads_them_back() {
        let directory = tempfile::tempdir().expect("temp directory");
        let store = LocalCheckpointStore::new(directory.path());
        let run_id = Uuid::new_v4();
        let artifact = br#"{"model":"fixture"}"#;
        let stored = store
            .write(run_id, 1, "hashing-linear-v1", artifact)
            .expect("write");
        assert_eq!(store.read(&stored.path).expect("read"), artifact);
        assert!(stored.checksum.starts_with("sha256:"));
        assert_eq!(stored.size_bytes, artifact.len() as u64);
        verify_checksum(artifact, &stored.checksum).expect("checksum matches");
        verify_file(&stored.path, &stored.checksum, stored.size_bytes)
            .expect("streamed file verification");
        assert!(verify_checksum(b"modified", &stored.checksum).is_err());
        assert!(
            store
                .write(run_id, 1, "hashing-linear-v1", artifact)
                .is_err()
        );
        let artifact_directory = directory.path().join(run_id.to_string());
        assert!(
            std::fs::read_dir(artifact_directory)
                .expect("artifact directory")
                .all(|entry| !entry
                    .expect("directory entry")
                    .path()
                    .to_string_lossy()
                    .ends_with(".tmp"))
        );
    }
}
