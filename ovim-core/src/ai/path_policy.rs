use std::path::{Path, PathBuf};

/// Normalize a path without filesystem access (no symlink resolution).
pub fn normalize_path(path: &Path) -> PathBuf {
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

/// Canonicalize if possible, otherwise normalize syntactically.
pub fn canonicalize_or_normalize(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| normalize_path(path))
}

/// Return true when `path` contains explicit `..` traversal components.
pub fn has_parent_traversal(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
}

/// Check if a path is inside an allow-root set.
pub fn is_path_approved(path: &Path, approved_roots: &[PathBuf]) -> bool {
    let normalized = canonicalize_or_normalize(path);
    approved_roots.iter().any(|root| {
        let root = canonicalize_or_normalize(root);
        normalized.starts_with(&root)
    })
}

/// Returns a human-readable reason when a path should be treated as sensitive.
pub fn sensitive_path_reason(path: &Path) -> Option<&'static str> {
    let normalized = canonicalize_or_normalize(path);
    let file_name = normalized
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());

    if let Some(name) = file_name.as_deref() {
        if name == ".env" || name.starts_with(".env.") {
            return Some(".env secrets are blocked by default");
        }
        if matches!(
            name,
            "id_rsa" | "id_dsa" | "id_ecdsa" | "id_ed25519" | "authorized_keys" | "known_hosts"
        ) {
            return Some("SSH key material is blocked by default");
        }
        if name.ends_with(".pem")
            || name.ends_with(".key")
            || name.ends_with(".p12")
            || name.ends_with(".pfx")
            || name.ends_with(".kdbx")
        {
            return Some("certificate/key files are blocked by default");
        }
    }

    for component in normalized.components() {
        let std::path::Component::Normal(c) = component else {
            continue;
        };
        let comp = c.to_string_lossy().to_ascii_lowercase();
        if comp == ".ssh" {
            return Some(".ssh directory is blocked by default");
        }
        if comp == ".aws" {
            return Some(".aws directory is blocked by default");
        }
    }

    None
}

pub fn is_sensitive_path(path: &Path) -> bool {
    sensitive_path_reason(path).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_env_files() {
        assert!(is_sensitive_path(Path::new("/repo/.env")));
        assert!(is_sensitive_path(Path::new("/repo/.env.local")));
        assert!(!is_sensitive_path(Path::new("/repo/.envrc")));
    }

    #[test]
    fn detects_key_material_files() {
        assert!(is_sensitive_path(Path::new("/repo/keys/private.pem")));
        assert!(is_sensitive_path(Path::new("/repo/keys/private.key")));
        assert!(is_sensitive_path(Path::new("/repo/.ssh/id_ed25519")));
    }

    #[test]
    fn detects_sensitive_directories() {
        assert!(is_sensitive_path(Path::new("/repo/.aws/credentials")));
        assert!(is_sensitive_path(Path::new("/repo/.ssh/config")));
    }

    #[test]
    fn approval_checks_prefixes() {
        let approved = vec![PathBuf::from("/repo/allowed")];
        assert!(is_path_approved(
            Path::new("/repo/allowed/a.txt"),
            &approved
        ));
        assert!(!is_path_approved(Path::new("/repo/other/a.txt"), &approved));
    }
}
