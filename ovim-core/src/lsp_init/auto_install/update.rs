//! Keeping language servers that ovim installed from GitHub releases current.
//!
//! Every GitHub install leaves a receipt next to it naming the release tag on
//! disk. With that, ovim can tell an old server from the latest one:
//!
//! - once a day, when a server ovim installed starts, it asks GitHub for the
//!   latest release. `autoinstall=auto` installs it for the next start;
//!   `autoinstall=prompt` says once that it exists; `autoinstall=off` never
//!   asks.
//! - `:LspUpdate` (and `u` in `:LspManager`) asks right away and installs
//!   whatever is latest.
//!
//! A server found on `PATH` belongs to whoever put it there and is never
//! touched.

use super::github::{install_release, GithubClient, GithubInstall};
use super::{github_install, InstallResult, INSTALL_GATE, MINIMUM_RELEASE_AGE_DAYS};
use crate::editor::AutoInstallMode;
use crate::language_config::{installed_github_binary, AutoInstallConfig};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// How often a started server is checked against GitHub.
const CHECK_INTERVAL_HOURS: i64 = 24;

/// One check at a time: two files of one language opening together must not
/// both download the same release.
static CHECK_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Installs already checked by this process. A failed request leaves no trace
/// in the receipt, so without this an offline editor would ask GitHub again
/// on every file it opens.
static CHECKED_THIS_SESSION: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

/// What ovim knows about one GitHub install. Stored beside the install as
/// `<install path>.ovim-release.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Receipt {
    /// The repository the install came from. A receipt for another
    /// repository (the language was reconfigured) says nothing about this one.
    repo: String,
    /// The release tag on disk. `None` for an install made before receipts
    /// existed.
    #[serde(default)]
    tag: Option<String>,
    /// The newest release the user has been told about or given.
    #[serde(default)]
    latest: Option<String>,
    /// When GitHub last answered a check, in Unix seconds.
    #[serde(default)]
    checked_at: Option<i64>,
}

fn receipt_path(target: &Path) -> PathBuf {
    let mut name = target
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".ovim-release.json");
    target.with_file_name(name)
}

fn read_receipt(target: &Path, repo: &str) -> Receipt {
    std::fs::read(receipt_path(target))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Receipt>(&bytes).ok())
        .filter(|receipt| receipt.repo == repo)
        .unwrap_or_else(|| Receipt {
            repo: repo.to_string(),
            ..Receipt::default()
        })
}

fn write_receipt(target: &Path, receipt: &Receipt) {
    let path = receipt_path(target);
    let result = (|| -> std::io::Result<()> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        // Readers must never see half a receipt.
        let staged = tempfile::NamedTempFile::new_in(parent)?;
        std::fs::write(staged.path(), serde_json::to_vec_pretty(receipt)?)?;
        staged.persist(&path).map_err(|error| error.error)?;
        Ok(())
    })();
    if let Err(error) = result {
        crate::lsp_warn!(
            "AutoInstall",
            "Could not record the installed release at {}: {}",
            path.display(),
            error
        );
    }
}

/// Record that `tag` of `repo` was just installed at `target`.
pub(super) fn record_install(target: &Path, repo: &str, tag: &str) {
    write_receipt(
        target,
        &Receipt {
            repo: repo.to_string(),
            tag: Some(tag.to_string()),
            latest: Some(tag.to_string()),
            checked_at: Some(Utc::now().timestamp()),
        },
    );
}

/// What a background check does about the latest release.
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    Nothing,
    Install,
    Notify,
}

fn plan(
    receipt: &Receipt,
    latest_tag: &str,
    published_at: Option<DateTime<Utc>>,
    mode: AutoInstallMode,
    now: DateTime<Utc>,
) -> Plan {
    if receipt.tag.as_deref() == Some(latest_tag) {
        return Plan::Nothing;
    }
    // The same quarantine as for npm packages: a release that was tampered
    // with is usually pulled within hours, so nothing this fresh is installed
    // or advertised unasked. A release without a date cannot be shown to be
    // old enough.
    let old_enough =
        published_at.is_some_and(|at| now - at >= Duration::days(MINIMUM_RELEASE_AGE_DAYS as i64));
    if !old_enough {
        return Plan::Nothing;
    }
    match mode {
        AutoInstallMode::Auto => Plan::Install,
        AutoInstallMode::Prompt if receipt.latest.as_deref() != Some(latest_tag) => Plan::Notify,
        AutoInstallMode::Prompt | AutoInstallMode::Off => Plan::Nothing,
    }
}

fn check_is_due(receipt: &Receipt, now: DateTime<Utc>) -> bool {
    let Some(checked_at) = receipt.checked_at else {
        return true;
    };
    let elapsed = now.timestamp() - checked_at;
    // A check dated in the future is a clock that moved; do not wait for it.
    !(0..CHECK_INTERVAL_HOURS * 3600).contains(&elapsed)
}

/// Whether `server_command`, as a server was started, is the file `managed`.
fn runs_from(server_command: &str, managed: &Path) -> bool {
    let Ok(running) = which::which(server_command) else {
        return false;
    };
    match (running.canonicalize(), managed.canonicalize()) {
        (Ok(running), Ok(managed)) => running == managed,
        _ => false,
    }
}

fn describe(tag: Option<&str>) -> &str {
    tag.unwrap_or("an unknown version")
}

/// After `server_command` started for `language_name`: if it is the copy ovim
/// installed from GitHub and a check is due, look for a newer release and act
/// on it as `mode` allows. Returns a line for the status bar when there is
/// something to say.
pub(crate) async fn check_for_update(
    language_name: &str,
    command: &str,
    server_command: &str,
    config: &AutoInstallConfig,
    mode: AutoInstallMode,
) -> Option<String> {
    if mode == AutoInstallMode::Off {
        return None;
    }
    let install = github_install(language_name, command, &config.method)?;
    let first_this_session = CHECKED_THIS_SESSION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get_or_insert_with(HashSet::new)
        .insert(install.target());
    if !first_this_session {
        return None;
    }
    let client = GithubClient::new().ok()?;
    check_with(
        &client,
        &install,
        command,
        server_command,
        config,
        mode,
        Utc::now(),
    )
    .await
}

async fn check_with(
    client: &GithubClient,
    install: &GithubInstall<'_>,
    command: &str,
    server_command: &str,
    config: &AutoInstallConfig,
    mode: AutoInstallMode,
    now: DateTime<Utc>,
) -> Option<String> {
    let managed = installed_github_binary(command, Some(config))?;
    if !runs_from(server_command, &managed) {
        return None;
    }

    let _check = CHECK_GATE.lock().await;
    let target = install.target();
    let mut receipt = read_receipt(&target, install.repo);
    if !check_is_due(&receipt, now) {
        return None;
    }

    let release = match client.latest_release(install.repo).await {
        Ok(release) => release,
        Err(error) => {
            crate::lsp_info!("AutoInstall", "Update check skipped: {}", error);
            return None;
        }
    };

    let installed = receipt.tag.clone();
    let decision = plan(&receipt, &release.tag, release.published_at, mode, now);
    receipt.checked_at = Some(now.timestamp());
    if decision == Plan::Notify || installed.as_deref() == Some(release.tag.as_str()) {
        receipt.latest = Some(release.tag.clone());
    }
    write_receipt(&target, &receipt);

    match decision {
        Plan::Nothing => None,
        Plan::Notify => Some(format!(
            "LSP: {command} {} is available (installed: {}); :LspUpdate installs it",
            release.tag,
            describe(installed.as_deref()),
        )),
        Plan::Install => {
            let _install = INSTALL_GATE.lock().await;
            Some(match install_release(client, install, &release).await {
                InstallResult::Success(_) => format!(
                    "LSP: {}",
                    UpdateResult::Updated {
                        from: installed,
                        to: Some(release.tag),
                    }
                    .summary(command)
                ),
                InstallResult::Failed(error) | InstallResult::PrerequisitesMissing(error) => {
                    crate::lsp_warn!("AutoInstall", "Updating {} failed: {}", command, error);
                    format!("LSP: {}", UpdateResult::Failed(error).summary(command))
                }
            })
        }
    }
}

/// How an update the user asked for ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateResult {
    /// A new copy is on disk. The versions are known for GitHub installs.
    Updated {
        from: Option<String>,
        to: Option<String>,
    },
    /// The latest release was already installed.
    UpToDate(String),
    Failed(String),
}

impl UpdateResult {
    /// One line describing the outcome for `command`.
    pub fn summary(&self, command: &str) -> String {
        const RESTART: &str = ":LspRestart to use it";
        match self {
            Self::Updated {
                from: Some(from),
                to: Some(to),
            } => format!("{command} updated from {from} to {to}; {RESTART}"),
            Self::Updated { to: Some(to), .. } => {
                format!("{command} updated to {to}; {RESTART}")
            }
            Self::Updated { to: None, .. } => format!("{command} reinstalled; {RESTART}"),
            Self::UpToDate(tag) => format!("{command} is already at {tag}, the latest release"),
            Self::Failed(error) => format!("updating {command} failed: {error}"),
        }
    }
}

/// Bring the server ovim installed for `language_name` up to date, now.
///
/// GitHub installs are replaced only when the latest release differs from the
/// one on disk. The other install methods have no version to compare, so
/// their installer simply runs again.
pub async fn update_server(
    language_name: &str,
    command: &str,
    config: &AutoInstallConfig,
) -> UpdateResult {
    let Some(install) = github_install(language_name, command, &config.method) else {
        return match super::attempt_auto_install(language_name, command, config).await {
            InstallResult::Success(_) => UpdateResult::Updated {
                from: None,
                to: None,
            },
            InstallResult::Failed(error) | InstallResult::PrerequisitesMissing(error) => {
                UpdateResult::Failed(error)
            }
        };
    };
    match GithubClient::new() {
        Ok(client) => update_with(&client, &install, command, config).await,
        Err(error) => UpdateResult::Failed(error),
    }
}

async fn update_with(
    client: &GithubClient,
    install: &GithubInstall<'_>,
    command: &str,
    config: &AutoInstallConfig,
) -> UpdateResult {
    let managed = installed_github_binary(command, Some(config));
    // Command resolution prefers PATH. Installing a second copy that is never
    // run would report an update that changed nothing.
    if let Ok(on_path) = which::which(command) {
        let is_managed = managed
            .as_deref()
            .is_some_and(|managed| runs_from(command, managed));
        if !is_managed {
            return UpdateResult::Failed(format!(
                "{} was not installed by ovim; update it with the tool that installed it",
                on_path.display()
            ));
        }
    }

    let _install = INSTALL_GATE.lock().await;
    let release = match client.latest_release(install.repo).await {
        Ok(release) => release,
        Err(error) => return UpdateResult::Failed(error),
    };
    let target = install.target();
    let mut receipt = read_receipt(&target, install.repo);
    if managed.is_some() && receipt.tag.as_deref() == Some(release.tag.as_str()) {
        receipt.latest = Some(release.tag.clone());
        receipt.checked_at = Some(Utc::now().timestamp());
        write_receipt(&target, &receipt);
        return UpdateResult::UpToDate(release.tag);
    }
    match install_release(client, install, &release).await {
        InstallResult::Success(_) => UpdateResult::Updated {
            from: receipt.tag,
            to: Some(release.tag),
        },
        InstallResult::Failed(error) | InstallResult::PrerequisitesMissing(error) => {
            UpdateResult::Failed(error)
        }
    }
}

/// The release of a server ovim installed from GitHub, as far as it is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledRelease {
    /// The tag on disk; `None` for an install made before ovim recorded it.
    pub version: Option<String>,
    /// A newer release ovim has seen.
    pub available: Option<String>,
}

impl InstalledRelease {
    /// For display: `v1.0.0`, `v1.0.0 (v1.1.0 available)`, `unknown`.
    pub fn describe(&self) -> String {
        let version = self.version.as_deref().unwrap_or("unknown");
        match &self.available {
            Some(available) => format!("{version} ({available} available)"),
            None => version.to_string(),
        }
    }
}

/// What is installed for `command`, if ovim installed it from GitHub.
pub fn installed_release(command: &str, config: &AutoInstallConfig) -> Option<InstalledRelease> {
    let install = github_install("", command, &config.method)?;
    let managed = installed_github_binary(command, Some(config))?;
    // A copy on PATH is the one that runs; the release of a second copy in
    // ovim's own folder would describe the wrong server.
    if which::which(command).is_ok() && !runs_from(command, &managed) {
        return None;
    }
    let receipt = read_receipt(&install.target(), install.repo);
    let available = receipt
        .latest
        .filter(|latest| receipt.tag.as_ref() != Some(latest));
    Some(InstalledRelease {
        version: receipt.tag,
        available,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::language_config::InstallMethod;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A stand-in for GitHub that serves one repository's latest release,
    /// with a single asset named `server`.
    struct FakeGithub {
        url: String,
        release: Arc<Mutex<(String, DateTime<Utc>)>>,
        requests: Arc<AtomicUsize>,
    }

    impl FakeGithub {
        async fn publishing(tag: &str, published_at: DateTime<Utc>) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let release = Arc::new(Mutex::new((tag.to_string(), published_at)));
            let requests = Arc::new(AtomicUsize::new(0));
            let (served, counter, base) = (release.clone(), requests.clone(), url.clone());
            tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        return;
                    };
                    let mut request = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut chunk).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => request.extend_from_slice(&chunk[..n]),
                        }
                    }
                    counter.fetch_add(1, Ordering::SeqCst);
                    let (tag, published_at) = served.lock().unwrap().clone();
                    let request = String::from_utf8_lossy(&request);
                    let body = if request.starts_with("GET /repos/owner/server/releases/latest ") {
                        serde_json::json!({
                            "tag_name": tag,
                            "published_at": published_at.to_rfc3339(),
                            "assets": [{
                                "name": "server",
                                "browser_download_url": format!("{base}/download/server"),
                            }],
                        })
                        .to_string()
                    } else {
                        format!("#!/bin/sh\necho {tag}\n")
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                }
            });
            Self {
                url,
                release,
                requests,
            }
        }

        fn publish(&self, tag: &str, published_at: DateTime<Utc>) {
            *self.release.lock().unwrap() = (tag.to_string(), published_at);
        }

        fn requests(&self) -> usize {
            self.requests.load(Ordering::SeqCst)
        }

        fn client(&self) -> GithubClient {
            GithubClient::at(&self.url).unwrap()
        }
    }

    /// A server installed as the single file `<dir>/server`, under a command
    /// name that is not on PATH.
    struct Fixture {
        _dir: tempfile::TempDir,
        binary: PathBuf,
        config: AutoInstallConfig,
    }

    const COMMAND: &str = "ovim-fixture-server-not-on-path";

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let binary = dir.path().join("server");
            let config = AutoInstallConfig {
                method: InstallMethod::Github {
                    repo: "owner/server".to_string(),
                    asset_pattern: "server".to_string(),
                    install_path: binary.to_string_lossy().into_owned(),
                    binary_name: None,
                    strip_components: 0,
                },
                version: None,
                policy: Default::default(),
                allow_headless: false,
            };
            Self {
                _dir: dir,
                binary,
                config,
            }
        }

        fn install(&self) -> GithubInstall<'_> {
            github_install("Fixture", COMMAND, &self.config.method).unwrap()
        }

        async fn install_latest(&self, github: &FakeGithub) {
            let client = github.client();
            let release = client.latest_release("owner/server").await.unwrap();
            let result = install_release(&client, &self.install(), &release).await;
            assert!(matches!(result, InstallResult::Success(_)), "{result:?}");
        }

        async fn check(
            &self,
            github: &FakeGithub,
            mode: AutoInstallMode,
            now: DateTime<Utc>,
        ) -> Option<String> {
            check_with(
                &github.client(),
                &self.install(),
                COMMAND,
                &self.binary.to_string_lossy(),
                &self.config,
                mode,
                now,
            )
            .await
        }

        async fn update(&self, github: &FakeGithub) -> UpdateResult {
            update_with(&github.client(), &self.install(), COMMAND, &self.config).await
        }

        fn on_disk(&self) -> String {
            std::fs::read_to_string(&self.binary).unwrap()
        }

        fn release(&self) -> InstalledRelease {
            installed_release(COMMAND, &self.config).unwrap()
        }
    }

    fn days_ago(days: i64) -> DateTime<Utc> {
        Utc::now() - Duration::days(days)
    }

    fn in_days(days: i64) -> DateTime<Utc> {
        Utc::now() + Duration::days(days)
    }

    #[tokio::test]
    async fn an_install_records_its_release_and_is_not_checked_again_that_day() {
        let github = FakeGithub::publishing("v1.0.0", days_ago(30)).await;
        let server = Fixture::new();
        assert_eq!(installed_release(COMMAND, &server.config), None);

        server.install_latest(&github).await;
        assert_eq!(
            server.release(),
            InstalledRelease {
                version: Some("v1.0.0".to_string()),
                available: None,
            }
        );

        github.publish("v1.1.0", days_ago(10));
        let requests = github.requests();
        let hour_later = Utc::now() + Duration::hours(1);
        assert_eq!(
            server
                .check(&github, AutoInstallMode::Auto, hour_later)
                .await,
            None
        );
        assert_eq!(github.requests(), requests, "no request within a day");
    }

    #[tokio::test]
    async fn auto_mode_installs_a_release_once_it_is_old_enough() {
        let github = FakeGithub::publishing("v1.0.0", days_ago(30)).await;
        let server = Fixture::new();
        server.install_latest(&github).await;

        // Published an hour before the check: still in quarantine.
        github.publish("v1.1.0", in_days(2) - Duration::hours(1));
        assert_eq!(
            server
                .check(&github, AutoInstallMode::Auto, in_days(2))
                .await,
            None
        );
        assert_eq!(server.on_disk(), "#!/bin/sh\necho v1.0.0\n");
        assert_eq!(server.release().available, None);

        let message = server
            .check(&github, AutoInstallMode::Auto, in_days(6))
            .await
            .expect("the update is announced");
        assert_eq!(
            message,
            format!("LSP: {COMMAND} updated from v1.0.0 to v1.1.0; :LspRestart to use it")
        );
        assert_eq!(server.on_disk(), "#!/bin/sh\necho v1.1.0\n");
        assert_eq!(
            server.release(),
            InstalledRelease {
                version: Some("v1.1.0".to_string()),
                available: None,
            }
        );
    }

    #[tokio::test]
    async fn prompt_mode_announces_each_release_once_and_installs_nothing() {
        let github = FakeGithub::publishing("v1.0.0", days_ago(30)).await;
        let server = Fixture::new();
        server.install_latest(&github).await;
        github.publish("v1.1.0", days_ago(10));

        let message = server
            .check(&github, AutoInstallMode::Prompt, in_days(2))
            .await
            .expect("the release is announced");
        assert_eq!(
            message,
            format!(
                "LSP: {COMMAND} v1.1.0 is available (installed: v1.0.0); :LspUpdate installs it"
            )
        );
        assert_eq!(server.on_disk(), "#!/bin/sh\necho v1.0.0\n");
        assert_eq!(server.release().describe(), "v1.0.0 (v1.1.0 available)");

        let requests = github.requests();
        assert_eq!(
            server
                .check(&github, AutoInstallMode::Prompt, in_days(4))
                .await,
            None
        );
        assert_eq!(github.requests(), requests + 1, "it did look again");
    }

    #[tokio::test]
    async fn an_install_from_before_receipts_counts_as_out_of_date() {
        let github = FakeGithub::publishing("v1.0.0", days_ago(30)).await;
        let server = Fixture::new();
        server.install_latest(&github).await;
        std::fs::remove_file(receipt_path(&server.binary)).unwrap();
        assert_eq!(
            server.release(),
            InstalledRelease {
                version: None,
                available: None,
            }
        );

        let message = server
            .check(&github, AutoInstallMode::Prompt, Utc::now())
            .await
            .expect("the release is announced");
        assert!(
            message.contains("(installed: an unknown version)"),
            "{message}"
        );

        let message = server
            .check(&github, AutoInstallMode::Auto, in_days(2))
            .await
            .expect("the reinstall is announced");
        assert_eq!(
            message,
            format!("LSP: {COMMAND} updated to v1.0.0; :LspRestart to use it")
        );
        assert_eq!(server.release().version.as_deref(), Some("v1.0.0"));
    }

    #[tokio::test]
    async fn a_server_started_from_elsewhere_is_left_alone() {
        let github = FakeGithub::publishing("v1.0.0", days_ago(30)).await;
        let server = Fixture::new();
        server.install_latest(&github).await;
        github.publish("v1.1.0", days_ago(10));
        let requests = github.requests();

        let message = check_with(
            &github.client(),
            &server.install(),
            COMMAND,
            "sh",
            &server.config,
            AutoInstallMode::Auto,
            in_days(2),
        )
        .await;

        assert_eq!(message, None);
        assert_eq!(github.requests(), requests);
        assert_eq!(server.on_disk(), "#!/bin/sh\necho v1.0.0\n");
    }

    #[tokio::test]
    async fn an_update_on_request_skips_the_download_when_already_current() {
        let github = FakeGithub::publishing("v1.0.0", days_ago(30)).await;
        let server = Fixture::new();
        server.install_latest(&github).await;

        let requests = github.requests();
        assert_eq!(
            server.update(&github).await,
            UpdateResult::UpToDate("v1.0.0".to_string())
        );
        assert_eq!(github.requests(), requests + 1, "metadata only");

        // Asking for it installs even a release that is minutes old.
        github.publish("v1.1.0", Utc::now());
        assert_eq!(
            server.update(&github).await,
            UpdateResult::Updated {
                from: Some("v1.0.0".to_string()),
                to: Some("v1.1.0".to_string()),
            }
        );
        assert_eq!(server.on_disk(), "#!/bin/sh\necho v1.1.0\n");
    }

    #[tokio::test]
    async fn an_update_on_request_refuses_a_server_that_is_on_path() {
        let github = FakeGithub::publishing("v1.0.0", days_ago(30)).await;
        let server = Fixture::new();

        let result = update_with(&github.client(), &server.install(), "sh", &server.config).await;

        let UpdateResult::Failed(error) = result else {
            panic!("expected a refusal, got {result:?}");
        };
        assert!(error.contains("was not installed by ovim"), "{error}");
        assert!(!server.binary.exists());
        assert_eq!(github.requests(), 0);
    }

    #[test]
    fn a_receipt_for_another_repository_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("server");
        record_install(&target, "owner/server", "v1.0.0");

        assert_eq!(
            read_receipt(&target, "owner/server").tag.as_deref(),
            Some("v1.0.0")
        );
        assert_eq!(
            read_receipt(&target, "fork/server"),
            Receipt {
                repo: "fork/server".to_string(),
                ..Receipt::default()
            }
        );
        assert_eq!(
            receipt_path(&target),
            dir.path().join("server.ovim-release.json")
        );
    }
}
