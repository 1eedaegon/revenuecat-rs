use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

use atomic_write_file::AtomicWriteFile;
use revenuecat::{Error, ErrorCode, MAX_APP_USER_ID_BYTES};
use serde::{Deserialize, Serialize};

const MAX_IDENTITY_FILE_BYTES: usize = 4_096;
const PERSISTED_IDENTITY_FILE_NAME: &str = "revenuecat-app-user-id.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PersistedIdentity {
    pub(super) current_app_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) pending_app_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) pending_operation: Option<IdentityOperation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum IdentityOperation {
    LogIn,
    LogOut,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PersistedIdentityFile {
    Legacy(String),
    Current(PersistedIdentity),
}

impl PersistedIdentity {
    pub(super) fn settled(app_user_id: impl Into<String>) -> Self {
        Self {
            current_app_user_id: app_user_id.into(),
            pending_app_user_id: None,
            pending_operation: None,
        }
    }

    pub(super) fn transitioning(
        current_app_user_id: &str,
        pending_app_user_id: &str,
        operation: IdentityOperation,
    ) -> Self {
        Self {
            current_app_user_id: current_app_user_id.to_owned(),
            pending_app_user_id: Some(pending_app_user_id.to_owned()),
            pending_operation: Some(operation),
        }
    }

    pub(super) fn pending_operation(&self) -> Option<IdentityOperation> {
        self.pending_app_user_id
            .as_ref()
            .map(|_| self.pending_operation.unwrap_or(IdentityOperation::LogIn))
    }

    fn validate(self) -> Result<Self, Error> {
        if !valid_app_user_id(&self.current_app_user_id)
            || self
                .pending_app_user_id
                .as_deref()
                .is_some_and(|app_user_id| !valid_app_user_id(app_user_id))
            || self.pending_app_user_id.is_some() != self.pending_operation.is_some()
        {
            return Err(identity_error(
                "The persisted RevenueCat identity contains an invalid App User ID or incomplete pending operation.",
            ));
        }
        Ok(self)
    }
}

pub(super) fn identity_file_name() -> &'static str {
    PERSISTED_IDENTITY_FILE_NAME
}

pub(super) fn load(path: &Path) -> Result<Option<PersistedIdentity>, Error> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(identity_error(format!(
                "Could not read the persisted RevenueCat identity: {error}"
            )));
        }
    };
    let mut bytes = Vec::new();
    file.take((MAX_IDENTITY_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            identity_error(format!(
                "Could not read the persisted RevenueCat identity: {error}"
            ))
        })?;
    if bytes.len() > MAX_IDENTITY_FILE_BYTES {
        return Err(identity_error(
            "The persisted RevenueCat identity file is unexpectedly large.",
        ));
    }
    let stored = serde_json::from_slice::<PersistedIdentityFile>(&bytes).map_err(|error| {
        identity_error(format!(
            "The persisted RevenueCat identity is invalid JSON: {error}"
        ))
    })?;
    let identity = match stored {
        PersistedIdentityFile::Legacy(app_user_id) => PersistedIdentity::settled(app_user_id),
        PersistedIdentityFile::Current(identity) => identity,
    };
    identity.validate().map(Some)
}

pub(super) fn persist(path: &Path, identity: &PersistedIdentity) -> Result<(), Error> {
    identity.clone().validate()?;
    let serialized = serde_json::to_vec(identity).map_err(|error| {
        identity_error(format!(
            "Could not serialize the RevenueCat identity: {error}"
        ))
    })?;
    if serialized.len() > MAX_IDENTITY_FILE_BYTES {
        return Err(identity_error(
            "The RevenueCat identity cannot be persisted because it is too large.",
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| identity_error("The RevenueCat identity path has no parent directory."))?;
    fs::create_dir_all(parent).map_err(|error| {
        identity_error(format!(
            "Could not create the RevenueCat identity directory: {error}"
        ))
    })?;
    let result = (|| -> std::io::Result<()> {
        let mut options = AtomicWriteFile::options();
        #[cfg(unix)]
        {
            use atomic_write_file::unix::OpenOptionsExt as AtomicOpenOptionsExt;
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
            options.preserve_mode(false).try_preserve_owner(false);
        }
        let mut file = options.open(path)?;
        file.write_all(&serialized)?;
        file.sync_all()?;
        file.commit()?;
        sync_parent_directory(parent)
    })();
    if let Err(error) = result {
        return Err(identity_error(format!(
            "Could not persist the RevenueCat identity: {error}"
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> std::io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> std::io::Result<()> {
    Ok(())
}

fn valid_app_user_id(app_user_id: &str) -> bool {
    !app_user_id.trim().is_empty() && app_user_id.len() <= MAX_APP_USER_ID_BYTES
}

fn identity_error(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::ConfigurationError, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_identity_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "revenuecat-rs-{name}-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ))
    }

    #[test]
    fn legacy_identity_is_loaded_as_a_settled_record() {
        let path = test_identity_path("identity-legacy");
        let _ = fs::remove_file(&path);
        fs::write(&path, serde_json::to_vec("$RCAnonymousID:legacy").unwrap()).unwrap();

        assert_eq!(
            load(&path).unwrap(),
            Some(PersistedIdentity::settled("$RCAnonymousID:legacy"))
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn identity_journal_round_trips_and_replaces_atomically() {
        let path = test_identity_path("identity-round-trip");
        let _ = fs::remove_file(&path);
        let pending =
            PersistedIdentity::transitioning("user-a", "user-b", IdentityOperation::LogIn);
        persist(&path, &pending).unwrap();
        assert_eq!(load(&path).unwrap(), Some(pending));

        let settled = PersistedIdentity::settled("user-b");
        persist(&path, &settled).unwrap();
        assert_eq!(load(&path).unwrap(), Some(settled));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn corrupt_or_oversized_identity_fails_closed() {
        let path = test_identity_path("identity-corrupt");
        let _ = fs::remove_file(&path);
        fs::write(&path, b"not-json").unwrap();
        assert_eq!(load(&path).unwrap_err().code, ErrorCode::ConfigurationError);

        fs::write(&path, vec![b'x'; MAX_IDENTITY_FILE_BYTES + 1]).unwrap();
        assert_eq!(load(&path).unwrap_err().code, ErrorCode::ConfigurationError);

        fs::write(&path, serde_json::to_vec("").unwrap()).unwrap();
        assert_eq!(load(&path).unwrap_err().code, ErrorCode::ConfigurationError);

        fs::write(
            &path,
            br#"{"currentAppUserId":"user-a","pendingAppUserId":"user-b"}"#,
        )
        .unwrap();
        assert_eq!(load(&path).unwrap_err().code, ErrorCode::ConfigurationError);
        fs::remove_file(path).unwrap();
    }
}
