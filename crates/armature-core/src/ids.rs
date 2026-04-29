use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ulid::Ulid;

use crate::error::{ArmatureError, ArmatureResult};

fn validate_prefixed_ulid(value: &str, prefix: &str) -> ArmatureResult<()> {
    let ulid = value
        .strip_prefix(prefix)
        .ok_or_else(|| ArmatureError::invalid_input(format!("expected {prefix}<ulid>")))?;
    Ulid::from_string(ulid)
        .map(|_| ())
        .map_err(|_| ArmatureError::invalid_input(format!("invalid ULID in {value}")))
}

macro_rules! prefixed_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new() -> Self {
                Self(format!("{}{}", $prefix, Ulid::new()))
            }

            pub fn parse(value: impl Into<String>) -> ArmatureResult<Self> {
                let value = value.into();
                validate_prefixed_ulid(&value, $prefix)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

prefixed_id!(RunId, "run_");
prefixed_id!(EventId, "evt_");
prefixed_id!(TriggerId, "trg_");

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceId(String);

impl WorkspaceId {
    pub fn from_canonical_path(path: &Path) -> ArmatureResult<Self> {
        let canonical = path.canonicalize()?;
        let mut hasher = Sha256::new();
        hasher.update(canonical.to_string_lossy().as_bytes());
        let digest = hasher.finalize();
        Ok(Self(format!("ws_{:x}", digest)[..19].to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{EventId, RunId, TriggerId, WorkspaceId};

    #[test]
    fn generated_ids_use_expected_prefixes() {
        assert!(RunId::new().as_str().starts_with("run_"));
        assert!(EventId::new().as_str().starts_with("evt_"));
        assert!(TriggerId::new().as_str().starts_with("trg_"));
    }

    #[test]
    fn parse_rejects_wrong_prefix() {
        let error = RunId::parse("evt_01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap_err();
        assert_eq!(error.to_string(), "invalid_input: expected run_<ulid>");
    }

    #[test]
    fn workspace_ids_are_stable_for_same_path() {
        let one = WorkspaceId::from_canonical_path(Path::new(".")).unwrap();
        let two = WorkspaceId::from_canonical_path(Path::new(".")).unwrap();
        assert_eq!(one, two);
        assert!(one.as_str().starts_with("ws_"));
    }
}
