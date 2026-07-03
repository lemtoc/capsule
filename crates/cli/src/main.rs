//! CLI entry point for capsule, a macOS zsh prompt engine.

#![warn(clippy::pedantic, clippy::nursery, clippy::cargo)]

mod build_id;
mod cli;
mod connect;
mod daemon;
mod preset;

use clap::Parser;

use crate::cli::{Cli, Command, DaemonAction, Shell};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Daemon { action } => match action {
            None => daemon::run(),
            Some(DaemonAction::Status { json }) => daemon::status(json),
            Some(DaemonAction::Install | DaemonAction::Uninstall) => {
                use daemon::{InstallOutcome, ServiceManager as _};

                let home = daemon::home_dir()?;
                let socket_path = daemon::socket_path()?;

                #[cfg(target_os = "macos")]
                let sm = daemon::Launchd::new(&socket_path)?;
                #[cfg(target_os = "linux")]
                let sm = daemon::Systemd::new(&socket_path);
                #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                anyhow::bail!("service management is not supported on this platform");

                match action {
                    Some(DaemonAction::Install) => {
                        let outcome = sm.install(&home, &socket_path)?;
                        match outcome {
                            InstallOutcome::Installed => {
                                println!("capsule daemon installed and loaded");
                            }
                            InstallOutcome::Restarted => {
                                println!("daemon restarted (binary updated)");
                            }
                            InstallOutcome::AlreadyCurrent => {
                                println!("service is already current, no reload needed");
                            }
                        }
                        Ok(())
                    }
                    Some(DaemonAction::Uninstall) => sm.uninstall(&home),
                    _ => unreachable!(),
                }
            }
        },
        Command::Connect { local } => connect::run(local),
        Command::Init { shell, local } => {
            match shell {
                Shell::Zsh => {
                    if local {
                        let current_exe = std::env::current_exe()?;
                        let current_exe = current_exe.to_string_lossy();
                        let connect_command = zsh_single_quote(&current_exe);
                        print!(
                            "{}",
                            capsule_core::init::zsh::generate_local_with_connect_command(
                                &connect_command
                            )
                        );
                    } else {
                        print!("{}", capsule_core::init::zsh::generate());
                    }
                }
            }
            Ok(())
        }
        Command::Preset => preset::run(),
    }
}

fn zsh_single_quote(value: &str) -> String {
    let escaped = value.replace('\'', "'\\''");
    format!("'{escaped}'")
}
