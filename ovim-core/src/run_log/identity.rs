use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// An invalid externally supplied run-log identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidId {
    expected_prefix: &'static str,
}

impl fmt::Display for InvalidId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "identifier must start with `{}` and contain a value",
            self.expected_prefix
        )
    }
}

impl std::error::Error for InvalidId {}

fn generated_id(prefix: &str) -> String {
    // The timestamp makes IDs useful in logs while the process ID and atomic
    // counter keep concurrent generation collision-free within a local ovim
    // process. This is an opaque identifier, not a timestamp contract.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!(
        "{prefix}{nanos:032x}{:08x}{counter:016x}",
        std::process::id()
    )
}

macro_rules! identifier {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new() -> Self {
                Self(generated_id($prefix))
            }

            pub fn parse(value: impl Into<String>) -> Result<Self, InvalidId> {
                let value = value.into();
                if value.len() > $prefix.len() && value.starts_with($prefix) {
                    Ok(Self(value))
                } else {
                    Err(InvalidId {
                        expected_prefix: $prefix,
                    })
                }
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
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = InvalidId;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }
    };
}

identifier!(RunId, "run_");
identifier!(AgentId, "agt_");
identifier!(TurnId, "trn_");
identifier!(EventId, "evt_");
identifier!(WorkspaceId, "wsp_");
identifier!(OperationId, "op_");
identifier!(ArtifactId, "art_");
identifier!(ManifestId, "mft_");
identifier!(ConversationId, "cnv_");
identifier!(BranchId, "brn_");
identifier!(RepositoryId, "repo_");
identifier!(BaseManifestId, "bsm_");

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[test]
    fn identifiers_have_distinct_types_and_expected_prefixes() {
        let run = RunId::new();
        let agent = AgentId::new();

        assert!(run.as_str().starts_with("run_"));
        assert!(agent.as_str().starts_with("agt_"));
        assert!(RunId::parse(agent.to_string()).is_err());
    }

    #[test]
    fn concurrent_generation_is_unique() {
        let ids = Arc::new(Mutex::new(Vec::new()));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let ids = Arc::clone(&ids);
                thread::spawn(move || {
                    let generated: Vec<_> = (0..100).map(|_| EventId::new()).collect();
                    ids.lock().unwrap().extend(generated);
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        let ids = ids.lock().unwrap();
        let unique: HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
    }
}
