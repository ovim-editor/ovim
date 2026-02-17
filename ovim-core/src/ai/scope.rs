use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use crate::ai::tools::SideEffect;
use crate::ai::types::FileScope;

/// Capabilities granted to a tool execution context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    pub file_scope: FileScope,
    pub shell: bool,
    pub network: bool,
    pub allow_mutations: bool,
}

impl Capabilities {
    /// Check whether `path` is allowed under the current file scope.
    pub fn validate_path(&self, path: &Path, ctx: &ScopeContext) -> Result<()> {
        match self.file_scope {
            FileScope::Any => Ok(()),
            FileScope::Selection => {
                bail!("file access not allowed in selection scope");
            }
            FileScope::File => {
                let Some(current) = &ctx.current_file else {
                    bail!("no current file to validate against");
                };
                // Reject path traversal
                if has_parent_traversal(path) {
                    bail!("path traversal (..) not allowed");
                }
                let canon_path = normalize_path(path);
                let canon_current = normalize_path(current);
                if canon_path != canon_current {
                    bail!(
                        "file scope restricts access to current file ({}), got {}",
                        canon_current.display(),
                        canon_path.display()
                    );
                }
                Ok(())
            }
            FileScope::Project => {
                let Some(root) = &ctx.project_root else {
                    bail!("no project root to validate against");
                };
                if has_parent_traversal(path) {
                    bail!("path traversal (..) not allowed");
                }
                let canon_path = normalize_path(path);
                let canon_root = normalize_path(root);
                if !canon_path.starts_with(&canon_root) {
                    bail!(
                        "project scope restricts access to {} — path {} is outside",
                        canon_root.display(),
                        canon_path.display()
                    );
                }
                Ok(())
            }
        }
    }

    pub fn check_shell(&self) -> Result<()> {
        if self.shell {
            Ok(())
        } else {
            bail!("shell access not allowed by current scope");
        }
    }

    pub fn check_network(&self) -> Result<()> {
        if self.network {
            Ok(())
        } else {
            bail!("network access not allowed by current scope");
        }
    }

    /// Whether this capability set allows the given side effect.
    pub fn allows_side_effect(&self, effect: SideEffect) -> bool {
        match effect {
            SideEffect::Read | SideEffect::Navigation => true,
            SideEffect::Mutation => self.allow_mutations,
            SideEffect::External => self.shell,
        }
    }

    /// Whether this scope satisfies the given requirement.
    pub fn contains(&self, required: &RequiredScope) -> bool {
        self.file_scope >= required.file_scope
            && (self.shell || !required.shell)
            && (self.network || !required.network)
    }

    /// Compute the intersection (minimum) of two capability sets.
    pub fn intersect(a: &Capabilities, b: &Capabilities) -> Capabilities {
        Capabilities {
            file_scope: std::cmp::min(a.file_scope, b.file_scope),
            shell: a.shell && b.shell,
            network: a.network && b.network,
            allow_mutations: a.allow_mutations && b.allow_mutations,
        }
    }
}

/// Minimum scope a tool requires to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredScope {
    pub file_scope: FileScope,
    pub shell: bool,
    pub network: bool,
}

/// Runtime context for scope validation.
#[derive(Debug, Clone)]
pub struct ScopeContext {
    pub current_file: Option<PathBuf>,
    pub project_root: Option<PathBuf>,
}

/// Check if a path contains `..` components.
fn has_parent_traversal(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
}

/// Normalize a path without hitting the filesystem (no symlink resolution).
/// Uses `components()` to collapse `.` and handle prefix canonically.
fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with_file(file: &str, root: &str) -> ScopeContext {
        ScopeContext {
            current_file: Some(PathBuf::from(file)),
            project_root: Some(PathBuf::from(root)),
        }
    }

    #[test]
    fn file_scope_ordering() {
        assert!(FileScope::Selection < FileScope::File);
        assert!(FileScope::File < FileScope::Project);
        assert!(FileScope::Project < FileScope::Any);
    }

    #[test]
    fn validate_path_file_scope_same_file() {
        let caps = Capabilities {
            file_scope: FileScope::File,
            shell: false,
            network: false,
            allow_mutations: true,
        };
        let ctx = ctx_with_file("/src/main.rs", "/src");
        assert!(caps.validate_path(Path::new("/src/main.rs"), &ctx).is_ok());
    }

    #[test]
    fn validate_path_file_scope_different_file() {
        let caps = Capabilities {
            file_scope: FileScope::File,
            shell: false,
            network: false,
            allow_mutations: true,
        };
        let ctx = ctx_with_file("/src/main.rs", "/src");
        assert!(caps.validate_path(Path::new("/src/lib.rs"), &ctx).is_err());
    }

    #[test]
    fn validate_path_project_scope_inside() {
        let caps = Capabilities {
            file_scope: FileScope::Project,
            shell: false,
            network: false,
            allow_mutations: true,
        };
        let ctx = ctx_with_file("/project/src/main.rs", "/project");
        assert!(caps
            .validate_path(Path::new("/project/src/lib.rs"), &ctx)
            .is_ok());
    }

    #[test]
    fn validate_path_project_scope_outside() {
        let caps = Capabilities {
            file_scope: FileScope::Project,
            shell: false,
            network: false,
            allow_mutations: true,
        };
        let ctx = ctx_with_file("/project/src/main.rs", "/project");
        assert!(caps
            .validate_path(Path::new("/other/file.rs"), &ctx)
            .is_err());
    }

    #[test]
    fn validate_path_any_scope() {
        let caps = Capabilities {
            file_scope: FileScope::Any,
            shell: false,
            network: false,
            allow_mutations: true,
        };
        let ctx = ctx_with_file("/src/main.rs", "/src");
        assert!(caps
            .validate_path(Path::new("/anywhere/file.txt"), &ctx)
            .is_ok());
    }

    #[test]
    fn validate_path_rejects_traversal() {
        let caps = Capabilities {
            file_scope: FileScope::Project,
            shell: false,
            network: false,
            allow_mutations: true,
        };
        let ctx = ctx_with_file("/project/src/main.rs", "/project");
        assert!(caps
            .validate_path(Path::new("/project/../etc/passwd"), &ctx)
            .is_err());
    }

    #[test]
    fn validate_path_selection_scope_rejects_all() {
        let caps = Capabilities {
            file_scope: FileScope::Selection,
            shell: false,
            network: false,
            allow_mutations: true,
        };
        let ctx = ctx_with_file("/src/main.rs", "/src");
        assert!(caps.validate_path(Path::new("/src/main.rs"), &ctx).is_err());
    }

    #[test]
    fn contains_checks_all_dimensions() {
        let caps = Capabilities {
            file_scope: FileScope::Project,
            shell: true,
            network: false,
            allow_mutations: true,
        };
        let req_ok = RequiredScope {
            file_scope: FileScope::File,
            shell: true,
            network: false,
        };
        assert!(caps.contains(&req_ok));

        let req_fail_net = RequiredScope {
            file_scope: FileScope::File,
            shell: false,
            network: true,
        };
        assert!(!caps.contains(&req_fail_net));

        let req_fail_scope = RequiredScope {
            file_scope: FileScope::Any,
            shell: false,
            network: false,
        };
        assert!(!caps.contains(&req_fail_scope));
    }

    #[test]
    fn intersect_takes_minimum() {
        let a = Capabilities {
            file_scope: FileScope::Project,
            shell: true,
            network: true,
            allow_mutations: true,
        };
        let b = Capabilities {
            file_scope: FileScope::File,
            shell: false,
            network: true,
            allow_mutations: false,
        };
        let result = Capabilities::intersect(&a, &b);
        assert_eq!(result.file_scope, FileScope::File);
        assert!(!result.shell);
        assert!(result.network);
        assert!(!result.allow_mutations);
    }

    #[test]
    fn check_shell_and_network() {
        let caps = Capabilities {
            file_scope: FileScope::File,
            shell: true,
            network: false,
            allow_mutations: true,
        };
        assert!(caps.check_shell().is_ok());
        assert!(caps.check_network().is_err());
    }

    #[test]
    fn allows_side_effect_read_always_allowed() {
        let caps = Capabilities {
            file_scope: FileScope::File,
            shell: false,
            network: false,
            allow_mutations: false,
        };
        assert!(caps.allows_side_effect(SideEffect::Read));
    }

    #[test]
    fn allows_side_effect_mutation_respects_flag() {
        let mut caps = Capabilities {
            file_scope: FileScope::File,
            shell: false,
            network: false,
            allow_mutations: false,
        };
        assert!(!caps.allows_side_effect(SideEffect::Mutation));
        caps.allow_mutations = true;
        assert!(caps.allows_side_effect(SideEffect::Mutation));
    }

    #[test]
    fn allows_side_effect_external_respects_shell() {
        let mut caps = Capabilities {
            file_scope: FileScope::File,
            shell: false,
            network: false,
            allow_mutations: true,
        };
        assert!(!caps.allows_side_effect(SideEffect::External));
        caps.shell = true;
        assert!(caps.allows_side_effect(SideEffect::External));
    }
}
