//! Where a file's project starts: the one home of every root rule.
//!
//! - [`find_project_root_with_outermost`]: the JVM / language-server root.
//!   Language servers are started with it (`LspConfig::find_root`), the
//!   launch flow uses it when no server owns the file, and the local test
//!   plan builds its build-tool commands against the same directory, so a
//!   run, a test and the server never disagree about the root. Maven's rule
//!   is the reactor rule (OV-00440): a parent pom only counts when it lists
//!   the module.
//! - [`vcs_root`]: the repository a file lives in, where pickers, grep,
//!   replace-in-files and recent-file history are scoped.

use std::path::{Path, PathBuf};

/// Root of the repository containing `path`: the nearest ancestor holding a
/// `.git` directory or file (worktrees and submodules use a file). A
/// relative `path` is taken relative to the working directory.
pub fn vcs_root(path: &Path) -> Option<PathBuf> {
    let path = absolute(path);
    path.ancestors()
        .skip(1)
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
}

/// [`vcs_root`], falling back to the directory holding `path` when it is not
/// in a repository.
pub fn vcs_root_or_dir(path: &Path) -> PathBuf {
    vcs_root(path).unwrap_or_else(|| dir_of(&absolute(path)))
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

/// Find project root by walking up and checking markers
///
/// Walks up the directory tree from the file, checking for marker files
/// like Cargo.toml, package.json, etc. Returns the first directory that
/// contains any of the specified markers.
///
/// Educational Note: Why walk up instead of down?
/// - Project roots are above files in the directory tree
/// - Walking down would be exponentially slower (must check all subdirs)
/// - Walking up is O(depth) where depth is typically <10
pub fn find_project_root(file_path: &Path, markers: &[String]) -> PathBuf {
    marker_root(file_path, markers).unwrap_or_else(|| dir_of(file_path))
}

/// The nearest ancestor directory of `file_path` holding one of `markers`.
fn marker_root(file_path: &Path, markers: &[String]) -> Option<PathBuf> {
    file_path
        .ancestors()
        .skip(1)
        .find(|dir| markers.iter().any(|marker| dir.join(marker).exists()))
        .map(Path::to_path_buf)
}

/// The directory holding `file_path` (the fallback root).
fn dir_of(file_path: &Path) -> PathBuf {
    file_path
        .parent()
        .unwrap_or_else(|| Path::new("/"))
        .to_path_buf()
}

/// Like [`find_project_root`], but a directory holding one of
/// `outermost_markers` (searched from the top down) wins over a nearer
/// `markers` match.
///
/// `pom.xml` is special in `outermost_markers`: an ancestor pom is not a
/// root just for existing (an unrelated `~/pom.xml` must not capture every
/// project below it). It only counts when it is the Maven reactor
/// aggregator of the module below it, i.e. its `<modules>` list the module
/// directory (or its pom file); the walk continues upwards so a nested
/// aggregator chain resolves to the outermost reactor.
pub fn find_project_root_with_outermost(
    file_path: &Path,
    markers: &[String],
    outermost_markers: &[String],
) -> PathBuf {
    marker_root_with_outermost(file_path, markers, outermost_markers)
        .unwrap_or_else(|| dir_of(file_path))
}

/// [`find_project_root_with_outermost`] without the directory fallback: `None`
/// when no marker identifies a project around `file_path`.
pub fn marker_root_with_outermost(
    file_path: &Path,
    markers: &[String],
    outermost_markers: &[String],
) -> Option<PathBuf> {
    if !outermost_markers.is_empty() {
        let plain: Vec<&String> = outermost_markers
            .iter()
            .filter(|m| *m != "pom.xml")
            .collect();
        let outermost = file_path
            .ancestors()
            .skip(1)
            .filter(|dir| plain.iter().any(|m| dir.join(m).exists()))
            .last();
        if let Some(dir) = outermost {
            return Some(dir.to_path_buf());
        }
        if outermost_markers.iter().any(|m| m == "pom.xml") {
            if let Some(root) = maven_reactor_root(file_path) {
                return Some(root);
            }
        }
    }
    marker_root(file_path, markers)
}

/// Whether `path` is too broad to watch for changes: the filesystem root, the
/// home directory, or a directory above it. A "project" rooted there is no
/// project, and watching it would claim a watch for every directory below.
pub fn is_too_broad_to_watch(path: &Path) -> bool {
    path.parent().is_none() || dirs::home_dir().is_some_and(|home| home.starts_with(path))
}

/// The outermost Maven reactor aggregator above `file_path`: start at the
/// nearest module pom and climb while the parent directory's pom lists the
/// current directory in `<modules>`. `None` when no pom encloses the file.
fn maven_reactor_root(file_path: &Path) -> Option<PathBuf> {
    let mut module = file_path
        .ancestors()
        .skip(1)
        .find(|dir| dir.join("pom.xml").is_file())?
        .to_path_buf();
    while let Some(parent) = module.parent() {
        let pom = parent.join("pom.xml");
        let Ok(text) = std::fs::read_to_string(&pom) else {
            break;
        };
        let lists_module = pom_modules(&text).iter().any(|entry| {
            let entry = entry.trim_end_matches("pom.xml").trim_end_matches('/');
            normalize_relative(&parent.join(entry)) == module
        });
        if !lists_module {
            break;
        }
        module = parent.to_path_buf();
    }
    Some(module)
}

/// Lexically resolve `.` and `..` (module paths may be `../sibling`).
fn normalize_relative(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The `<module>` entries of a pom's `<modules>` sections (also inside
/// profiles), ignoring XML comments.
fn pom_modules(pom: &str) -> Vec<String> {
    let mut text = String::with_capacity(pom.len());
    let mut rest = pom;
    while let Some(start) = rest.find("<!--") {
        text.push_str(&rest[..start]);
        rest = match rest[start..].find("-->") {
            Some(end) => &rest[start + end + 3..],
            None => "",
        };
    }
    text.push_str(rest);
    let mut modules = Vec::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find("<module>") {
        let after = &rest[start + "<module>".len()..];
        let Some(end) = after.find("</module>") else {
            break;
        };
        modules.push(after[..end].trim().to_string());
        rest = &after[end..];
    }
    modules
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_and_everything_above_it_is_too_broad_to_watch() {
        assert!(is_too_broad_to_watch(Path::new("/")));
        if let Some(home) = dirs::home_dir() {
            assert!(is_too_broad_to_watch(&home));
            assert!(is_too_broad_to_watch(home.parent().unwrap_or(&home)));
            assert!(!is_too_broad_to_watch(&home.join("projects/app")));
        }
        assert!(!is_too_broad_to_watch(&std::env::temp_dir().join("app")));
    }

    #[test]
    fn marker_root_is_none_without_a_project_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a/b/c.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let markers = vec!["no-such-marker-file".to_string()];
        assert_eq!(marker_root_with_outermost(&file, &markers, &[]), None);
        std::fs::write(tmp.path().join("no-such-marker-file"), "").unwrap();
        assert_eq!(
            marker_root_with_outermost(&file, &markers, &[]),
            Some(tmp.path().to_path_buf())
        );
    }

    #[test]
    fn outermost_root_marker_beats_nearest_submodule() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        let sub = root.join("app/src/main/java");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(root.join("settings.gradle"), "").unwrap();
        std::fs::write(root.join("app/build.gradle"), "").unwrap();
        let file = sub.join("A.java");
        let markers = vec!["build.gradle".to_string()];
        let outer = vec!["settings.gradle".to_string()];
        assert_eq!(find_project_root(&file, &markers), root.join("app"));
        assert_eq!(
            find_project_root_with_outermost(&file, &markers, &outer),
            root
        );
        assert_eq!(
            find_project_root_with_outermost(&file, &markers, &["nope".to_string()]),
            root.join("app")
        );
    }

    #[test]
    fn maven_root_is_the_outermost_reactor_aggregator() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("multi");
        let file = root.join("app/src/main/java/demo/Main.java");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::create_dir_all(root.join("core")).unwrap();
        std::fs::write(
            root.join("pom.xml"),
            "<project><modules><!-- <module>ghost</module> --><module>core</module><module>app/</module></modules></project>",
        )
        .unwrap();
        std::fs::write(root.join("app/pom.xml"), "<project/>").unwrap();
        let markers: Vec<String> = ["pom.xml"].map(String::from).into();
        let outer: Vec<String> = ["settings.gradle", "pom.xml"].map(String::from).into();
        assert_eq!(find_project_root(&file, &markers), root.join("app"));
        assert_eq!(
            find_project_root_with_outermost(&file, &markers, &outer),
            root
        );
        // A pom that does not list the module is not its reactor.
        std::fs::write(
            root.join("pom.xml"),
            "<project><modules><module>core</module></modules></project>",
        )
        .unwrap();
        assert_eq!(
            find_project_root_with_outermost(&file, &markers, &outer),
            root.join("app")
        );
        // Nested aggregators and `../` module paths.
        std::fs::write(
            root.join("pom.xml"),
            "<project><modules><module>../multi/app</module></modules></project>",
        )
        .unwrap();
        assert_eq!(
            find_project_root_with_outermost(&file, &markers, &outer),
            root
        );
    }

    #[test]
    fn test_find_project_root() {
        use std::fs;
        use tempfile::tempdir;

        // Create temp directory structure:
        // temp/
        //   Cargo.toml
        //   src/
        //     lib.rs
        //     subdir/
        //       mod.rs

        let temp = tempdir().unwrap();
        let root = temp.path();

        fs::write(root.join("Cargo.toml"), "").unwrap();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "").unwrap();
        fs::create_dir(root.join("src/subdir")).unwrap();
        let file = root.join("src/subdir/mod.rs");
        fs::write(&file, "").unwrap();

        // Find Cargo.toml from nested file
        let markers = vec!["Cargo.toml".to_string()];
        let found_root = find_project_root(&file, &markers);

        assert_eq!(found_root, root);
    }

    #[test]
    fn vcs_root_is_the_nearest_git_dir_or_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let nested = root.join("repo/sub/deeper");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir(root.join("repo/.git")).unwrap();
        let file = nested.join("A.java");
        assert_eq!(vcs_root(&file), Some(root.join("repo")));
        // A worktree / submodule has a `.git` file; the nearest one wins.
        std::fs::write(root.join("repo/sub/.git"), "gitdir: elsewhere").unwrap();
        assert_eq!(vcs_root(&file), Some(root.join("repo/sub")));
        // Outside a repository the file's directory is the fallback.
        let lone = tmp.path().join("lone/B.java");
        std::fs::create_dir_all(lone.parent().unwrap()).unwrap();
        assert_eq!(vcs_root(&lone), None);
        assert_eq!(vcs_root_or_dir(&lone), lone.parent().unwrap());
    }
}
