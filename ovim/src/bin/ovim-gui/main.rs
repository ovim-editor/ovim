#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{Context, Result};
use ovim::cli::FileArg;
use ovim::gui::ssh::RemoteOptions;
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
    let mut file: Option<String> = None;
    let mut resume = false;
    let mut remote = RemoteOptions::default();
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let (flag, inline) = match argument.split_once('=') {
            Some((flag, value)) => (flag.to_string(), Some(value.to_string())),
            None => (argument.clone(), None),
        };
        match flag.as_str() {
            "--resume" => resume = true,
            // Only the destination is named here. A port, an identity, or a
            // proxy belongs in the user's SSH config, which ssh reads anyway.
            "--remote" => {
                remote.destination = Some(flag_value(inline, &mut arguments, "--remote")?)
            }
            "--fresh" => remote.fresh = true,
            "--allow-version-mismatch" => remote.allow_version_mismatch = true,
            // The capability travels in the session descriptor rather than in
            // an argument, so it never reaches the process table.
            "--remote-session" => {
                remote.session_file = Some(PathBuf::from(flag_value(
                    inline,
                    &mut arguments,
                    "--remote-session",
                )?))
            }
            "--remote-endpoint" => {
                remote.endpoint = Some(flag_value(inline, &mut arguments, "--remote-endpoint")?)
            }
            _ if argument.starts_with('-') => anyhow::bail!("Unknown GUI option: {argument}"),
            _ if file.is_none() => file = Some(argument),
            _ => anyhow::bail!("Only one file or directory can be opened at startup"),
        }
    }

    // Under a remote launch the path belongs to the other host: it goes to the
    // bootstrap and is never parsed or opened here.
    let is_remote = remote.destination.is_some() || remote.session_file.is_some();
    remote.path = file.clone();
    let local_file = (!is_remote)
        .then(|| file.as_deref().map(FileArg::parse))
        .flatten();

    let _ = ovim::log::init();
    if let Err(error) = ovim::language_config::LanguageRegistry::init() {
        ovim_core::log_warn!("gui", "Language registry initialization: {}", error);
    }
    ovim::gui::app::run(local_file, resume, remote.resolve()?)
}
