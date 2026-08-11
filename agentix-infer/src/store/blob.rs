use crate::error::InferError;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub fn blob_path(models_dir: &Path, hash: &str) -> PathBuf {
    models_dir.join("blobs").join(format!("sha256-{}", hash))
}

/// Write a blob from a reader. Returns (sha256_hex, size_bytes).
/// Writes to a temp file first, then renames atomically.
pub fn write_blob(models_dir: &Path, mut reader: impl Read) -> Result<(String, u64), InferError> {
    let blobs_dir = models_dir.join("blobs");
    std::fs::create_dir_all(&blobs_dir)?;

    let tmp = tempfile::NamedTempFile::new_in(&blobs_dir)?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    {
        let file = tmp.as_file();
        let mut file_ref = file;
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file_ref.write_all(&buf[..n])?;
            total += n as u64;
        }
    }

    let hash = hex::encode(hasher.finalize());
    let dest = blob_path(models_dir, &hash);

    if dest.exists() {
        // Already stored — discard the tmp file and return the existing size
        return Ok((hash, dest.metadata()?.len()));
    }

    tmp.persist(&dest).map_err(|e| InferError::Io(e.error))?;
    Ok((hash, total))
}

pub fn verify_blob(models_dir: &Path, hash: &str) -> Result<(), InferError> {
    let path = blob_path(models_dir, hash);
    let mut file = std::fs::File::open(&path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = hex::encode(hasher.finalize());
    if actual != hash {
        return Err(InferError::ChecksumMismatch {
            path,
            expected: hash.to_string(),
            actual,
        });
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn write_and_verify() {
        let dir = tempfile::tempdir().unwrap();
        let data = b"hello world";
        let (hash, size) = write_blob(dir.path(), Cursor::new(data)).unwrap();
        assert_eq!(size, 11);
        assert!(!hash.is_empty());
        verify_blob(dir.path(), &hash).unwrap();
        assert!(blob_path(dir.path(), &hash).exists());
    }

    #[test]
    fn checksum_mismatch_detected() {
        let dir = tempfile::tempdir().unwrap();
        let data = b"hello world";
        let (hash, _) = write_blob(dir.path(), Cursor::new(data)).unwrap();
        // corrupt the blob
        std::fs::write(blob_path(dir.path(), &hash), b"corrupted").unwrap();
        assert!(verify_blob(dir.path(), &hash).is_err());
    }

    #[test]
    fn idempotent_write() {
        let dir = tempfile::tempdir().unwrap();
        let data = b"idempotent";
        let (h1, _) = write_blob(dir.path(), Cursor::new(data)).unwrap();
        let (h2, _) = write_blob(dir.path(), Cursor::new(data)).unwrap();
        assert_eq!(h1, h2);
    }
}
