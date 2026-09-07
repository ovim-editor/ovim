#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{Context, Result};
use ovim::cli::FileArg;
use ovim::gui::RemoteEndpoint;
use std::path::PathBuf;

/// The value of a flag, written either `--flag value` or `--flag=value`.
fn flag_value(
    inline: Option<String>,
    rest: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<String> {
    inline
        .or_else(|| rest.next())
        .with_context(|| format!("{flag} needs a value"))
}

fn main() -> Result<()> {
    let mut file = None;
    let mut resume = false;
    let mut remote_session: Option<PathBuf> = None;
    let mut remote_endpoint: Option<String> = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let (flag, inline) = match argument.split_once('=') {
            Some((flag, value)) => (flag.to_string(), Some(value.to_string())),
            None => (argument.clone(), None),
        };
        match flag.as_str() {
            "--resume" => resume = true,
            // The capability travels in the session descriptor rather than in
            // an argument, so it never reaches the process table.
            "--remote-session" => {
                remote_session = Some(PathBuf::from(flag_value(
                    inline,
                    &mut arguments,
                    "--remote-session",
                )?))
            }
            "--remote-endpoint" => {
                remote_endpoint = Some(flag_value(inline, &mut arguments, "--remote-endpoint")?)
            }
            _ if argument.starts_with('-') => anyhow::bail!("Unknown GUI option: {argument}"),
            _ if file.is_none() => file = Some(FileArg::parse(&argument)),
            _ => anyhow::bail!("Only one file or directory can be opened at startup"),
        }
    }

    anyhow::ensure!(
        remote_session.is_some() || remote_endpoint.is_none(),
        "--remote-endpoint needs --remote-session: without a descriptor there is no capability to \
         authenticate with"
    );
    let remote = remote_session
        .map(|path| RemoteEndpoint::from_session_file(&path, remote_endpoint.as_deref()))
        .transpose()?;

    let _ = ovim::log::init();
    if let Err(error) = ovim::language_config::LanguageRegistry::init() {
        ovim_core::log_warn!("gui", "Language registry initialization: {}", error);
    }
    ovim::gui::app::run(file, resume, remote)
}
