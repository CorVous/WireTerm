//! Adjacent, revisioned named-secret storage.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::Zeroizing;

const STORE_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("named-secret name must contain 1-64 letters, numbers, '.', '-', or '_'")]
    InvalidName,
    #[error("named-secret value must not be empty")]
    EmptyValue,
    #[error("named secret is not available")]
    NotFound,
    #[error("named-secret store is unreadable")]
    Read,
    #[error("named-secret store could not be updated")]
    Write,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SecretRevision {
    version: u32,
    revision: u64,
    entries: BTreeMap<String, String>,
}

impl Default for SecretRevision {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            revision: 0,
            entries: BTreeMap::new(),
        }
    }
}

/// User-facing secret names and local plaintext values in adjacent data.
#[derive(Clone, Debug)]
pub struct SecretStore {
    revisions_dir: PathBuf,
}

impl SecretStore {
    #[must_use]
    pub fn new(data_dir: &Path) -> Self {
        Self {
            revisions_dir: data_dir.join("secret-revisions"),
        }
    }

    #[must_use]
    pub fn revisions_dir(&self) -> &Path {
        &self.revisions_dir
    }

    pub fn names(&self) -> Result<Vec<String>, SecretError> {
        Ok(self.load_latest()?.entries.into_keys().collect())
    }

    pub fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        validate_name(name)?;
        if value.is_empty() {
            return Err(SecretError::EmptyValue);
        }
        let mut revision = self.load_latest()?;
        revision.entries.insert(name.to_owned(), value.to_owned());
        self.save_revision(revision)
    }

    pub fn remove(&self, name: &str) -> Result<bool, SecretError> {
        validate_name(name)?;
        let mut revision = self.load_latest()?;
        let removed = revision.entries.remove(name).is_some();
        if removed {
            self.save_revision(revision)?;
        }
        Ok(removed)
    }

    pub fn resolve(&self, name: &str) -> Result<Zeroizing<String>, SecretError> {
        validate_name(name)?;
        self.load_latest()?
            .entries
            .remove(name)
            .map(Zeroizing::new)
            .ok_or(SecretError::NotFound)
    }

    fn load_latest(&self) -> Result<SecretRevision, SecretError> {
        let entries = match fs::read_dir(&self.revisions_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(SecretRevision::default());
            }
            Err(_) => return Err(SecretError::Read),
        };
        let mut paths = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.starts_with("revision-")
                            && Path::new(name)
                                .extension()
                                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
                    })
            })
            .collect::<Vec<_>>();
        paths.sort_unstable_by(|left, right| right.file_name().cmp(&left.file_name()));
        for path in paths {
            let Ok(bytes) = fs::read(path) else {
                continue;
            };
            let Ok(revision) = serde_json::from_slice::<SecretRevision>(&bytes) else {
                continue;
            };
            if revision.version == STORE_VERSION {
                return Ok(revision);
            }
        }
        Ok(SecretRevision::default())
    }

    fn save_revision(&self, mut revision: SecretRevision) -> Result<(), SecretError> {
        fs::create_dir_all(&self.revisions_dir).map_err(|_| SecretError::Write)?;
        revision.version = STORE_VERSION;
        revision.revision = self
            .load_latest()?
            .revision
            .max(revision.revision)
            .saturating_add(1);
        let bytes = serde_json::to_vec_pretty(&revision).map_err(|_| SecretError::Write)?;
        let stem = format!("revision-{:020}", revision.revision);
        let temporary = self.revisions_dir.join(format!("{stem}.tmp"));
        let final_path = self.revisions_dir.join(format!("{stem}.json"));
        fs::write(&temporary, bytes).map_err(|_| SecretError::Write)?;
        fs::rename(&temporary, final_path).map_err(|_| SecretError::Write)
    }
}

fn validate_name(name: &str) -> Result<(), SecretError> {
    if (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        Ok(())
    } else {
        Err(SecretError::InvalidName)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_nanos();
            Self(
                std::env::temp_dir()
                    .join(format!("wireterm-secrets-{}-{nonce}", std::process::id())),
            )
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn secret_lifecycle_is_local_plaintext_and_revisioned() {
        let directory = TestDirectory::new();
        let store = SecretStore::new(&directory.0);
        store.set("weather-token", "do-not-leak-123").expect("set");
        assert_eq!(store.names().expect("names"), ["weather-token"]);
        assert_eq!(
            &*store.resolve("weather-token").expect("resolve"),
            "do-not-leak-123"
        );

        let first = fs::read_dir(store.revisions_dir())
            .expect("revision directory")
            .next()
            .expect("revision")
            .expect("entry")
            .path();
        let stored = fs::read(first).expect("stored bytes");
        assert!(
            stored
                .windows(15)
                .any(|window| window == b"do-not-leak-123")
        );

        assert!(store.remove("weather-token").expect("remove"));
        assert!(store.names().expect("names after remove").is_empty());
        assert!(matches!(
            store.resolve("weather-token"),
            Err(SecretError::NotFound)
        ));
    }

    #[test]
    fn invalid_names_and_empty_values_are_rejected() {
        let directory = TestDirectory::new();
        let store = SecretStore::new(&directory.0);
        assert!(matches!(
            store.set("space name", "x"),
            Err(SecretError::InvalidName)
        ));
        assert!(matches!(
            store.set("valid", ""),
            Err(SecretError::EmptyValue)
        ));
    }
}
