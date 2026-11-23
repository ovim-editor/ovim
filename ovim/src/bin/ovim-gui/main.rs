#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Result;
use clap::Parser;
use ovim::cli::{FileArg, GuiCli};

fn main() -> Result<()> {
    let cli = GuiCli::parse();

    // Under a remote launch the path belongs to the other host: it goes to the
    // bootstrap and is never parsed or opened here.
    let is_remote = cli.remote_args.is_remote();
    let local_file = (!is_remote)
        .then(|| cli.file.as_deref().map(FileArg::parse))
        .flatten();
    let remote = ovim::gui::ssh::RemoteOptions::from_args(cli.remote_args, cli.file);

    let _ = ovim::log::init();
    if let Err(error) = ovim::language_config::LanguageRegistry::init() {
        ovim_core::log_warn!("gui", "Language registry initialization: {}", error);
    }
    ovim::gui::app::run(local_file, cli.resume, remote.resolve()?)
}
