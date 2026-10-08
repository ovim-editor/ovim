//! On-demand install of the Claude Agent SDK. The SDK is not open source, so
//! ovim does not redistribute it: the pinned version is fetched from npm into
//! the user's cache the first time a Claude Agent chat runs.
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Stdio;

const PACKAGE: &str = "@anthropic-ai/claude-agent-sdk";
/// SHA-256 of the pinned version's `sdk.mjs`, the module the runtime is tested against.
const SDK_SHA256: &str = "d768bb75542ea853a66c570bd82b010a7b843f4949ab3ebb20e9d1f2a27af881";

fn module_path(prefix: &Path) -> PathBuf {
    prefix.join("node_modules/@anthropic-ai/claude-agent-sdk/sdk.mjs")
}

fn verified(module: &Path) -> bool {
    std::fs::read(module).is_ok_and(|bytes| format!("{:x}", Sha256::digest(bytes)) == SDK_SHA256)
}

/// Return the pinned SDK module, installing it with npm on first use.
pub(crate) async fn ensure_sdk() -> Result<PathBuf> {
    let root = dirs::cache_dir()
        .context("Failed to get cache directory")?
        .join("ovim/claude-agent-sdk");
    let install = root.join(super::SDK_VERSION);
    let module = module_path(&install);
    if verified(&module) {
        return Ok(module);
    }
    std::fs::create_dir_all(&root)?;
    let staging = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(&root)?;
    // Only the SDK module is needed: optional dependencies are bundled Claude
    // executables (ovim uses the installed `claude`), and peer dependencies are
    // only needed by SDK features the runtime does not import.
    let output = tokio::process::Command::new(if cfg!(windows) { "npm.cmd" } else { "npm" })
        .args([
            "install",
            "--no-save",
            "--no-package-lock",
            "--omit=optional",
            "--ignore-scripts",
            "--legacy-peer-deps",
            "--no-audit",
            "--no-fund",
            "--loglevel=error",
            "--prefix",
        ])
        .arg(staging.path())
        .arg(format!("{PACKAGE}@{}", super::SDK_VERSION))
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .context("Could not run npm to install the Claude Agent SDK. Install Node.js 18.18 or newer, which includes npm")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "npm could not install {PACKAGE}@{}: {}",
            super::SDK_VERSION,
            stderr.trim()
        );
    }
    if !verified(&module_path(staging.path())) {
        bail!(
            "The downloaded {PACKAGE}@{} does not match the module ovim was tested with",
            super::SDK_VERSION
        );
    }
    // Another chat may have finished the same install meanwhile.
    if verified(&module) {
        return Ok(module);
    }
    let _ = std::fs::remove_dir_all(&install);
    let staged = staging.keep();
    if let Err(error) = std::fs::rename(&staged, &install) {
        let _ = std::fs::remove_dir_all(&staged);
        if !verified(&module) {
            return Err(error).context("Could not move the Claude Agent SDK into the cache");
        }
    }
    Ok(module)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_pinned_module_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let module = module_path(dir.path());
        assert!(!verified(&module));
        std::fs::create_dir_all(module.parent().unwrap()).unwrap();
        std::fs::write(&module, "export const query = () => {};").unwrap();
        assert!(!verified(&module));
    }

    /// Network: run after bumping `SDK_VERSION` to confirm the pin and hash.
    #[tokio::test]
    #[ignore]
    async fn installs_the_pinned_sdk_from_npm() {
        assert!(verified(&ensure_sdk().await.unwrap()));
    }
}
