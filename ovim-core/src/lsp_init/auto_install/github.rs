use super::InstallResult;
use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Install via GitHub release (download binary or archive).
pub(super) async fn install_via_github(
    language_name: &str,
    repo: &str,
    asset_pattern: &str,
    install_path: &str,
    binary_name: Option<&str>,
    strip_components: usize,
    default_binary_name: &str,
) -> InstallResult {
    #[derive(Debug, Deserialize)]
    struct GitHubRelease {
        assets: Vec<GitHubAsset>,
    }

    #[derive(Debug, Deserialize)]
    struct GitHubAsset {
        name: String,
        browser_download_url: String,
    }

    if repo.split('/').count() != 2 {
        return InstallResult::Failed(format!(
            "Invalid GitHub repo '{repo}'. Expected format: owner/repo"
        ));
    }

    let release_url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let client = match reqwest::Client::builder()
        .user_agent("ovim-auto-install")
        .build()
    {
        Ok(client) => client,
        Err(e) => {
            return InstallResult::Failed(format!("Failed to initialize HTTP client: {e}"));
        }
    };

    let release_res = match client
        .get(&release_url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
    {
        Ok(res) => res,
        Err(e) => {
            return InstallResult::Failed(format!(
                "Failed to query GitHub releases for {repo}: {e}"
            ));
        }
    };

    if !release_res.status().is_success() {
        return InstallResult::Failed(format!(
            "GitHub API request failed for {repo}: HTTP {}",
            release_res.status()
        ));
    }

    let release = match release_res.json::<GitHubRelease>().await {
        Ok(release) => release,
        Err(e) => {
            return InstallResult::Failed(format!("Failed to parse GitHub release metadata: {e}"));
        }
    };

    let asset = select_asset(
        &release.assets,
        |asset| &asset.name,
        asset_pattern,
        std::env::consts::OS,
        std::env::consts::ARCH,
    );

    let Some(asset) = asset else {
        return InstallResult::Failed(format!(
            "No release asset matched pattern '{asset_pattern}' (expanded for {}/{}) in {repo}",
            std::env::consts::OS,
            std::env::consts::ARCH,
        ));
    };

    crate::lsp_info!(
        "AutoInstall",
        "Installing {} via GitHub release: {}/{}",
        language_name,
        repo,
        asset.name
    );

    let download_res = match client
        .get(&asset.browser_download_url)
        .header("Accept", "application/octet-stream")
        .send()
        .await
    {
        Ok(res) => res,
        Err(e) => {
            return InstallResult::Failed(format!(
                "Failed to download GitHub asset '{}': {e}",
                asset.name
            ));
        }
    };

    if !download_res.status().is_success() {
        return InstallResult::Failed(format!(
            "GitHub asset download failed for '{}': HTTP {}",
            asset.name,
            download_res.status()
        ));
    }

    let bytes = match download_res.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => {
            return InstallResult::Failed(format!(
                "Failed to read downloaded bytes for '{}': {e}",
                asset.name
            ));
        }
    };

    match install_downloaded_asset(
        &bytes,
        &asset.name,
        &expand_install_path(install_path),
        binary_name,
        strip_components,
        default_binary_name,
    )
    .await
    {
        Ok(path) => InstallResult::Success(path),
        Err(error) => InstallResult::Failed(format!(
            "Failed to install GitHub asset '{}': {error}",
            asset.name
        )),
    }
}

fn expand_install_path(raw: &str) -> PathBuf {
    let expanded = shellexpand::tilde(raw).into_owned();
    PathBuf::from(expanded)
}

/// Pick the release asset for one platform. The expanded patterns are tried
/// in order, so the first spelling of the platform that matches wins.
fn select_asset<'a, T>(
    assets: &'a [T],
    name: impl Fn(&T) -> &str,
    pattern: &str,
    os: &str,
    arch: &str,
) -> Option<&'a T> {
    expand_platform_patterns_for(pattern, os, arch)
        .iter()
        .find_map(|pattern| {
            assets
                .iter()
                .find(|asset| asset_matches_pattern(name(asset), pattern))
        })
}

fn expand_platform_patterns_for(pattern: &str, os: &str, arch: &str) -> Vec<String> {
    if !["{os}", "{arch}", "{ext}"]
        .iter()
        .any(|placeholder| pattern.contains(placeholder))
    {
        return vec![pattern.to_string()];
    }

    let os_variants: Vec<&str> = match os {
        "macos" => vec!["darwin", "macos"],
        "linux" => vec!["linux"],
        "windows" => vec!["windows", "win64"],
        other => vec![other],
    };

    let arch_variants: Vec<&str> = match arch {
        "x86_64" => vec!["x86_64", "amd64", "x64"],
        "aarch64" => vec!["aarch64", "arm64"],
        other => vec![other],
    };

    // Most projects ship Unix standalone binaries with gzip and Windows
    // binaries in zip archives. Configs that do not use {ext} are unaffected.
    let extension = if os == "windows" { "zip" } else { "gz" };
    let mut patterns = Vec::new();
    for os in os_variants {
        for arch in &arch_variants {
            patterns.push(
                pattern
                    .replace("{os}", os)
                    .replace("{arch}", arch)
                    .replace("{ext}", extension),
            );
        }
    }
    patterns
}

async fn install_downloaded_asset(
    bytes: &[u8],
    asset_name: &str,
    install_path: &Path,
    binary_name: Option<&str>,
    strip_components: usize,
    default_binary_name: &str,
) -> Result<PathBuf, String> {
    match asset_kind(asset_name) {
        AssetKind::Archive => {
            install_archive(
                bytes,
                asset_name,
                install_path,
                binary_name,
                strip_components,
                default_binary_name,
            )
            .await
        }
        AssetKind::GzipBinary => {
            let output_path = binary_name
                .map(|name| install_path.join(platform_executable_name(name)))
                .unwrap_or_else(|| install_path.to_path_buf());
            let decompressed = decompress_gzip_binary(bytes)?;
            write_executable(&output_path, &decompressed).await?;
            Ok(output_path)
        }
        AssetKind::Binary => {
            write_executable(install_path, bytes).await?;
            Ok(install_path.to_path_buf())
        }
    }
}

async fn install_archive(
    bytes: &[u8],
    asset_name: &str,
    target_dir: &Path,
    binary_name: Option<&str>,
    strip_components: usize,
    default_binary_name: &str,
) -> Result<PathBuf, String> {
    tokio::fs::create_dir_all(target_dir)
        .await
        .map_err(|error| {
            format!(
                "failed to create install directory '{}': {error}",
                target_dir.display()
            )
        })?;

    if strip_components == 0 {
        extract_archive(bytes, asset_name, target_dir)?;
    } else {
        extract_archive_stripped(bytes, asset_name, target_dir, strip_components)?;
    }

    let binary_name = binary_name.unwrap_or(default_binary_name);
    let binary_path = target_dir.join(platform_executable_name(binary_name));
    if !binary_path.is_file() {
        return Err(format!(
            "archive extracted but binary '{binary_name}' was not found at '{}'",
            binary_path.display()
        ));
    }

    set_executable_permissions(&binary_path).await?;
    Ok(binary_path)
}

fn decompress_gzip_binary(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = flate2::read::GzDecoder::new(bytes);
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|error| format!("gzip decompression failed: {error}"))?;
    Ok(decompressed)
}

async fn write_executable(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    tokio::fs::create_dir_all(parent).await.map_err(|error| {
        format!(
            "failed to create install directory '{}': {error}",
            parent.display()
        )
    })?;

    // Publish only the complete executable, with its final permissions. A
    // same-directory rename also replaces a running binary without truncating
    // its inode, and never follows a destination symlink into another file.
    let staged = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("failed to stage binary for '{}': {error}", path.display()))?;
    tokio::fs::write(staged.path(), bytes)
        .await
        .map_err(|error| format!("failed to write binary to '{}': {error}", path.display()))?;
    set_executable_permissions(staged.path()).await?;
    staged
        .as_file()
        .sync_all()
        .map_err(|error| format!("failed to sync binary for '{}': {error}", path.display()))?;
    staged
        .persist(path)
        .map_err(|error| format!("failed to publish binary to '{}': {error}", path.display()))?;
    Ok(())
}

#[cfg(unix)]
async fn set_executable_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .await
        .map_err(|error| {
            format!(
                "failed to set executable permissions on '{}': {error}",
                path.display()
            )
        })
}

#[cfg(not(unix))]
async fn set_executable_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn platform_executable_name(binary_name: &str) -> String {
    if std::env::consts::EXE_SUFFIX.is_empty()
        || binary_name.ends_with(std::env::consts::EXE_SUFFIX)
        || Path::new(binary_name).extension().is_some()
    {
        binary_name.to_string()
    } else {
        format!("{binary_name}{}", std::env::consts::EXE_SUFFIX)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssetKind {
    Archive,
    GzipBinary,
    Binary,
}

fn asset_kind(name: &str) -> AssetKind {
    let lower = name.to_ascii_lowercase();
    if is_archive_asset(&lower) {
        AssetKind::Archive
    } else if lower.ends_with(".gz") {
        AssetKind::GzipBinary
    } else {
        AssetKind::Binary
    }
}

/// Extract an archive (.tar.gz, .tgz, .zip) to a target directory.
fn extract_archive(
    bytes: &[u8],
    asset_name: &str,
    target_dir: &std::path::Path,
) -> Result<(), String> {
    let lower = asset_name.to_ascii_lowercase();

    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        extract_tar_gz(bytes, target_dir)
    } else if lower.ends_with(".tar.xz") || lower.ends_with(".txz") {
        extract_via_shell_tar(bytes, asset_name, target_dir)
    } else if lower.ends_with(".zip") {
        extract_zip(bytes, target_dir)
    } else {
        Err(format!("Unsupported archive format: {asset_name}"))
    }
}

/// Extract into a staging folder inside `target_dir`, then move the contents
/// of the folder `strip_components` levels down into `target_dir`.
fn extract_archive_stripped(
    bytes: &[u8],
    asset_name: &str,
    target_dir: &Path,
    strip_components: usize,
) -> Result<(), String> {
    let staging = tempfile::tempdir_in(target_dir)
        .map_err(|error| format!("failed to create a staging folder: {error}"))?;
    extract_archive(bytes, asset_name, staging.path())?;

    let mut root = staging.path().to_path_buf();
    for _ in 0..strip_components {
        root = only_folder_in(&root).ok_or_else(|| {
            format!("cannot strip a folder from '{asset_name}': it needs one top-level folder")
        })?;
    }

    let entries = std::fs::read_dir(&root)
        .map_err(|error| format!("failed to read '{}': {error}", root.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("failed to read '{}': {error}", root.display()))?;
        let destination = target_dir.join(entry.file_name());
        // A rename replaces a file, but not a folder that has contents.
        if std::fs::symlink_metadata(&destination).is_ok_and(|meta| meta.is_dir()) {
            std::fs::remove_dir_all(&destination).map_err(|error| {
                format!("failed to replace '{}': {error}", destination.display())
            })?;
        }
        std::fs::rename(entry.path(), &destination).map_err(|error| {
            format!(
                "failed to move '{}' into place: {error}",
                destination.display()
            )
        })?;
    }
    Ok(())
}

/// The path of the single entry in `dir`, if that entry is a folder.
fn only_folder_in(dir: &Path) -> Option<PathBuf> {
    let mut entries = std::fs::read_dir(dir).ok()?;
    let only = entries.next()?.ok()?;
    if entries.next().is_some() || !only.file_type().ok()?.is_dir() {
        return None;
    }
    Some(only.path())
}

fn extract_tar_gz(bytes: &[u8], target_dir: &std::path::Path) -> Result<(), String> {
    use flate2::read::GzDecoder;
    use std::io::Cursor;

    let cursor = Cursor::new(bytes);
    let decoder = GzDecoder::new(cursor);
    let mut archive = tar::Archive::new(decoder);

    archive
        .unpack(target_dir)
        .map_err(|e| format!("tar.gz extraction failed: {e}"))
}

fn extract_zip(bytes: &[u8], target_dir: &std::path::Path) -> Result<(), String> {
    use std::io::Cursor;

    let cursor = Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("Failed to read zip archive: {e}"))?;

    archive
        .extract(target_dir)
        .map_err(|e| format!("zip extraction failed: {e}"))
}

fn extract_via_shell_tar(
    bytes: &[u8],
    asset_name: &str,
    target_dir: &std::path::Path,
) -> Result<(), String> {
    let temp_file = target_dir.join(asset_name);
    std::fs::write(&temp_file, bytes).map_err(|e| format!("Failed to write temp archive: {e}"))?;

    let output = Command::new("tar")
        .args([
            "xf",
            &temp_file.to_string_lossy(),
            "-C",
            &target_dir.to_string_lossy(),
        ])
        .output()
        .map_err(|e| format!("Failed to run tar: {e}"))?;

    let _ = std::fs::remove_file(&temp_file);

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("tar extraction failed: {stderr}"))
    }
}

fn is_archive_asset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".zip")
        || lower.ends_with(".tar")
        || lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
        || lower.ends_with(".tar.xz")
        || lower.ends_with(".txz")
}

fn asset_matches_pattern(asset_name: &str, pattern: &str) -> bool {
    if pattern.is_empty() || pattern == "*" {
        return true;
    }

    if !pattern.contains('*') {
        return asset_name == pattern;
    }

    let parts: Vec<&str> = pattern.split('*').collect();
    let starts_anchored = !pattern.starts_with('*');
    let ends_anchored = !pattern.ends_with('*');
    let mut cursor = 0usize;

    for (idx, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }

        if idx == 0 && starts_anchored {
            if !asset_name[cursor..].starts_with(part) {
                return false;
            }
            cursor += part.len();
            continue;
        }

        if idx == parts.len() - 1 && ends_anchored {
            let remaining = &asset_name[cursor..];
            if !remaining.ends_with(part) {
                return false;
            }
            if let Some(pos) = remaining.rfind(part) {
                cursor += pos + part.len();
            }
            continue;
        }

        if let Some(found_at) = asset_name[cursor..].find(part) {
            cursor += found_at + part.len();
        } else {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    #[test]
    fn asset_patterns_support_wildcards() {
        assert!(asset_matches_pattern(
            "typescript-language-server-linux-x64",
            "typescript-language-server-*"
        ));
        assert!(asset_matches_pattern("gopls", "gopls"));
        assert!(!asset_matches_pattern(
            "clangd-arm64.zip",
            "clangd-*-linux*"
        ));
    }

    #[test]
    fn rust_analyzer_pattern_matches_supported_platform_assets() {
        let pattern = "rust-analyzer-{arch}-*{os}*.{ext}";
        for (os, arch, asset) in [
            (
                "linux",
                "x86_64",
                "rust-analyzer-x86_64-unknown-linux-gnu.gz",
            ),
            ("macos", "aarch64", "rust-analyzer-aarch64-apple-darwin.gz"),
            (
                "windows",
                "x86_64",
                "rust-analyzer-x86_64-pc-windows-msvc.zip",
            ),
        ] {
            let patterns = expand_platform_patterns_for(pattern, os, arch);
            assert!(
                patterns
                    .iter()
                    .any(|pattern| asset_matches_pattern(asset, pattern)),
                "{asset} did not match any of {patterns:?}"
            );
        }
    }

    /// Release assets of hyperion-lsp, from the `dist plan` manifest of its
    /// release workflow (cargo-dist 0.34). cargo-dist also uploads the
    /// manifest itself as `dist-manifest.json`.
    const HYPERION_RELEASE_ASSETS: &[&str] = &[
        "dist-manifest.json",
        "hyperion-lsp-aarch64-apple-darwin.tar.xz",
        "hyperion-lsp-aarch64-apple-darwin.tar.xz.sha256",
        "hyperion-lsp-aarch64-unknown-linux-gnu.tar.xz",
        "hyperion-lsp-aarch64-unknown-linux-gnu.tar.xz.sha256",
        "hyperion-lsp-installer.sh",
        "hyperion-lsp-x86_64-apple-darwin.tar.xz",
        "hyperion-lsp-x86_64-apple-darwin.tar.xz.sha256",
        "hyperion-lsp-x86_64-unknown-linux-gnu.tar.xz",
        "hyperion-lsp-x86_64-unknown-linux-gnu.tar.xz.sha256",
        "hyperion-lsp.rb",
        "sha256.sum",
        "source.tar.gz",
        "source.tar.gz.sha256",
    ];

    /// The GitHub install shared by every language that runs hyperion-lsp.
    fn embedded_hyperion_install() -> crate::language_config::InstallMethod {
        let (languages, _) = crate::language_config::LanguageRegistry::parse_configs(
            include_str!("../../../languages.toml"),
            None,
        )
        .unwrap();
        let methods: Vec<_> = languages
            .iter()
            .filter_map(|language| language.lsp.as_ref())
            .filter(|lsp| lsp.command == "hyperion-lsp")
            .map(|lsp| {
                lsp.auto_install
                    .as_ref()
                    .expect("every hyperion-lsp language has an auto-install")
                    .method
                    .clone()
            })
            .collect();
        assert!(methods.len() >= 4, "java, kotlin, groovy and scala");
        let first = serde_json::to_value(&methods[0]).unwrap();
        for method in &methods {
            assert_eq!(serde_json::to_value(method).unwrap(), first);
        }
        methods[0].clone()
    }

    #[test]
    fn hyperion_pattern_picks_the_one_archive_for_each_platform() {
        let crate::language_config::InstallMethod::Github { asset_pattern, .. } =
            embedded_hyperion_install()
        else {
            panic!("hyperion-lsp installs from GitHub");
        };
        for (os, arch, archive) in [
            (
                "macos",
                "aarch64",
                "hyperion-lsp-aarch64-apple-darwin.tar.xz",
            ),
            ("macos", "x86_64", "hyperion-lsp-x86_64-apple-darwin.tar.xz"),
            (
                "linux",
                "aarch64",
                "hyperion-lsp-aarch64-unknown-linux-gnu.tar.xz",
            ),
            (
                "linux",
                "x86_64",
                "hyperion-lsp-x86_64-unknown-linux-gnu.tar.xz",
            ),
        ] {
            assert_eq!(
                select_asset(
                    HYPERION_RELEASE_ASSETS,
                    |name| name,
                    &asset_pattern,
                    os,
                    arch
                ),
                Some(&archive),
                "{os}/{arch}"
            );
            let patterns = expand_platform_patterns_for(&asset_pattern, os, arch);
            let matching: Vec<_> = HYPERION_RELEASE_ASSETS
                .iter()
                .filter(|name| patterns.iter().any(|p| asset_matches_pattern(name, p)))
                .collect();
            assert_eq!(matching, [&archive], "{os}/{arch}");
        }
        // hyperion-lsp has no Windows build.
        assert_eq!(
            select_asset(
                HYPERION_RELEASE_ASSETS,
                |name| name,
                &asset_pattern,
                "windows",
                "x86_64"
            ),
            None
        );
    }

    /// Pack `files` under one top-level folder, like a cargo-dist Unix archive.
    #[cfg(unix)]
    fn cargo_dist_tar_xz(folder: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
        let source = tempfile::tempdir().unwrap();
        std::fs::create_dir(source.path().join(folder)).unwrap();
        for (name, contents) in files {
            std::fs::write(source.path().join(folder).join(name), contents).unwrap();
        }
        let archive = source.path().join("archive.tar.xz");
        let status = Command::new("tar")
            .arg("-cJf")
            .arg(&archive)
            .arg("-C")
            .arg(source.path())
            .arg(folder)
            .status()
            .expect("run tar");
        assert!(status.success(), "tar -cJf failed");
        std::fs::read(archive).unwrap()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn hyperion_archive_installs_its_binary_where_ovim_finds_it_again() {
        let crate::language_config::InstallMethod::Github {
            binary_name,
            strip_components,
            ..
        } = embedded_hyperion_install()
        else {
            panic!("hyperion-lsp installs from GitHub");
        };
        let install_dir = tempfile::tempdir().unwrap();
        let folder = "hyperion-lsp-aarch64-apple-darwin";
        let asset = format!("{folder}.tar.xz");
        let install = |server: &'static [u8]| {
            let bytes = cargo_dist_tar_xz(
                folder,
                &[
                    ("hyperion-lsp", server),
                    ("LICENSE", b"license"),
                    ("README.md", b"readme"),
                    ("NOTICE", b"notice"),
                    ("THIRD_PARTY_LICENSES", b"third party"),
                ],
            );
            let asset = asset.clone();
            let binary_name = binary_name.clone();
            let install_dir = install_dir.path().to_path_buf();
            async move {
                install_downloaded_asset(
                    &bytes,
                    &asset,
                    &install_dir,
                    binary_name.as_deref(),
                    strip_components,
                    "hyperion-lsp",
                )
                .await
            }
        };

        let installed = install(b"#!/bin/sh\necho 0.1.0\n").await.unwrap();
        assert_eq!(installed, install_dir.path().join("hyperion-lsp"));

        // An update replaces the binary and leaves no staging folder behind.
        let updated = install(b"#!/bin/sh\necho 0.2.0\n").await.unwrap();
        assert_eq!(updated, installed);
        assert_eq!(
            std::fs::read(&installed).unwrap(),
            b"#!/bin/sh\necho 0.2.0\n"
        );
        let mut entries: Vec<_> = std::fs::read_dir(install_dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        entries.sort();
        assert_eq!(
            entries,
            [
                "LICENSE",
                "NOTICE",
                "README.md",
                "THIRD_PARTY_LICENSES",
                "hyperion-lsp"
            ]
        );

        // After a restart, the install contract finds the binary again. The
        // command is one that is not on PATH, so only the contract can find it.
        let mut method = embedded_hyperion_install();
        let crate::language_config::InstallMethod::Github { install_path, .. } = &mut method else {
            unreachable!()
        };
        *install_path = install_dir.path().to_string_lossy().into_owned();
        let config = crate::language_config::AutoInstallConfig {
            method,
            version: None,
            policy: Default::default(),
            allow_headless: false,
        };
        assert_eq!(
            crate::language_config::find_configured_command(
                "ovim-fixture-command-not-on-path",
                &[],
                Some(&config)
            ),
            Some(installed.to_string_lossy().into_owned())
        );
    }

    #[tokio::test]
    async fn stripping_a_folder_needs_exactly_one_top_level_folder() {
        let temp = tempfile::tempdir().unwrap();
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut archive = tar::Builder::new(encoder);
        for name in ["server", "README.md"] {
            let mut header = tar::Header::new_gnu();
            header.set_size(1);
            header.set_mode(0o755);
            header.set_cksum();
            archive
                .append_data(&mut header, name, b"x".as_slice())
                .unwrap();
        }
        let compressed = archive.into_inner().unwrap().finish().unwrap();

        let error = install_downloaded_asset(
            &compressed,
            "flat.tar.gz",
            temp.path(),
            Some("server"),
            1,
            "server",
        )
        .await
        .expect_err("a flat archive has no folder to strip");

        assert!(error.contains("one top-level folder"), "{error}");
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[test]
    fn distinguishes_gzip_binaries_from_tar_archives() {
        assert_eq!(
            asset_kind("rust-analyzer-x86_64-unknown-linux-gnu.gz"),
            AssetKind::GzipBinary
        );
        assert_eq!(
            asset_kind("lua-language-server-linux-x64.tar.gz"),
            AssetKind::Archive
        );
        assert_eq!(asset_kind("zls"), AssetKind::Binary);
    }

    #[tokio::test]
    async fn installs_a_gzip_compressed_executable() {
        let temp_dir = tempfile::tempdir().expect("temporary directory");
        let install_dir = temp_dir.path().join("rust-analyzer/bin");
        let expected_contents = b"standalone executable";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(expected_contents)
            .expect("compress fixture");
        let compressed = encoder.finish().expect("finish fixture");

        let installed = install_downloaded_asset(
            &compressed,
            "rust-analyzer-x86_64-unknown-linux-gnu.gz",
            &install_dir,
            Some("rust-analyzer"),
            0,
            "Rust",
        )
        .await
        .expect("install gzip binary");

        assert_eq!(
            std::fs::read(&installed).expect("read installed binary"),
            expected_contents
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = installed
                .metadata()
                .expect("binary metadata")
                .permissions()
                .mode();
            assert_ne!(mode & 0o111, 0, "installed binary must be executable");
        }
    }

    #[tokio::test]
    async fn archive_default_launcher_uses_the_server_command_and_is_rediscovered() {
        let temp = tempfile::tempdir().unwrap();
        let name = platform_executable_name("ovim-fixture-lsp-command");
        let bytes = b"#!/bin/sh\nexit 0\n";
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut archive = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        archive
            .append_data(&mut header, &name, bytes.as_slice())
            .unwrap();
        let compressed = archive.into_inner().unwrap().finish().unwrap();
        let installed = install_downloaded_asset(
            &compressed,
            "fixture.tar.gz",
            temp.path(),
            None,
            0,
            "ovim-fixture-lsp-command",
        )
        .await
        .unwrap();
        assert_eq!(installed, temp.path().join(name));
        let config = serde_json::from_value(serde_json::json!({
            "method": {
                "type": "github", "repo": "example/server", "asset_pattern": "fixture.tar.gz",
                "install_path": temp.path()
            }
        }))
        .unwrap();
        assert_eq!(
            crate::language_config::find_configured_command(
                "ovim-fixture-lsp-command",
                &[],
                Some(&config)
            ),
            Some(installed.to_string_lossy().into_owned())
        );
    }

    #[tokio::test]
    async fn invalid_gzip_does_not_create_a_binary() {
        let temp_dir = tempfile::tempdir().expect("temporary directory");
        let install_dir = temp_dir.path().join("rust-analyzer/bin");

        let error = install_downloaded_asset(
            b"not gzip data",
            "rust-analyzer-x86_64-unknown-linux-gnu.gz",
            &install_dir,
            Some("rust-analyzer"),
            0,
            "Rust",
        )
        .await
        .expect_err("invalid gzip must fail");

        assert!(error.contains("gzip decompression failed"));
        assert!(!install_dir.join("rust-analyzer").exists());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn replacing_a_running_executable_does_not_truncate_its_inode() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let binary = directory.path().join("server");
        let sleep = which::which("sleep").expect("sleep fixture executable");
        std::fs::copy(sleep, &binary).expect("copy fixture executable");
        let mut running = Command::new(&binary)
            .arg("60")
            .spawn()
            .expect("run fixture");

        let result = write_executable(&binary, b"replacement executable").await;
        let still_running = running
            .try_wait()
            .expect("inspect running fixture")
            .is_none();
        let _ = running.kill();
        running.wait().expect("reap fixture process");

        result.expect("publish replacement while old executable is running");
        assert!(
            still_running,
            "publishing must leave the old process running"
        );
        assert_eq!(
            std::fs::read(binary).expect("read replacement"),
            b"replacement executable"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn executable_publication_replaces_a_symlink_without_overwriting_its_target() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let protected = directory.path().join("other-file");
        std::fs::write(&protected, b"preserve this file").expect("write target fixture");
        let binary = directory.path().join("server");
        std::os::unix::fs::symlink(&protected, &binary).expect("symlink fixture");

        write_executable(&binary, b"replacement executable")
            .await
            .expect("publish executable");

        assert_eq!(
            std::fs::read(protected).expect("read target"),
            b"preserve this file"
        );
        assert!(!std::fs::symlink_metadata(&binary)
            .expect("published metadata")
            .file_type()
            .is_symlink());
        assert_eq!(
            std::fs::read(binary).expect("read replacement"),
            b"replacement executable"
        );
    }

    #[tokio::test]
    async fn failed_publication_preserves_the_destination_and_cleans_staging() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let destination = directory.path().join("existing-directory");
        std::fs::create_dir(&destination).expect("destination directory");
        std::fs::write(destination.join("preserved"), b"keep this").expect("existing contents");

        let error = write_executable(&destination, b"replacement executable")
            .await
            .expect_err("a non-empty directory cannot be replaced by a binary");

        assert!(error.contains("failed to publish binary"));
        assert_eq!(
            std::fs::read(destination.join("preserved")).expect("preserved contents"),
            b"keep this"
        );
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("install directory")
                .count(),
            1
        );
    }

    #[tokio::test]
    #[ignore = "downloads the latest Rust Analyzer release from GitHub"]
    async fn installs_rust_analyzer_release() {
        let temp_dir = tempfile::tempdir().expect("temporary directory");
        let result = install_via_github(
            "Rust",
            "rust-lang/rust-analyzer",
            "rust-analyzer-{arch}-*{os}*.{ext}",
            temp_dir.path().to_str().expect("UTF-8 temporary path"),
            Some("rust-analyzer"),
            0,
            "rust-analyzer",
        )
        .await;
        let InstallResult::Success(binary) = result else {
            panic!("Rust Analyzer install failed: {result:?}");
        };

        let version = Command::new(binary)
            .arg("--version")
            .output()
            .expect("run installed Rust Analyzer");
        assert!(version.status.success());
        assert!(String::from_utf8_lossy(&version.stdout).contains("rust-analyzer"));
    }
}
