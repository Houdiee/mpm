use anyhow::{Result, bail};
use clap::{Args, CommandFactory, Parser, Subcommand};
use std::process::ExitCode;

use crate::commands::{ApplyOpts, Ctx};
use crate::manifest::Layer;

mod commands;
mod exec;
mod manager;
mod manifest;
mod reconcile;
mod report;

#[derive(Parser)]
#[command(
    name = "mpm",
    version,
    about = "Declare the packages your machine should have, and converge to it"
)]
struct Cli {
    /// Package managers to act on, comma-separated. Defaults to every managed one
    #[arg(value_name = "MANAGERS")]
    managers: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

impl Cli {
    fn managers(&self) -> Vec<String> {
        self.managers
            .iter()
            .flat_map(|spec| spec.split(','))
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect()
    }
}

#[derive(Subcommand)]
enum Command {
    /// Show how this machine differs from its manifests
    Status,

    /// Install and remove packages so this machine matches its manifests
    Apply(ApplyArgs),

    /// Put installed packages under management, merging with what the manifest says
    Inherit(LayerArgs),

    /// Declare packages, to be installed by `mpm apply`
    Add(AddArgs),

    /// Lock packages to the version installed right now
    Pin(PackageArgs),

    /// Let packages track whatever version is current
    Unpin(PackageArgs),

    /// Undeclare packages and uninstall them now
    Remove(PackageArgs),

    /// Report declared packages that have a newer version available
    Outdated,

    /// Bring declared packages up to date, leaving pinned ones alone
    Upgrade(ApplyArgs),

    /// Search each manager's own index and show what it says
    Search(SearchArgs),

    /// Show supported package managers and how this machine is configured
    Managers,
}

#[derive(Args)]
struct SearchArgs {
    /// What to look for
    #[arg(required = true, value_name = "QUERY")]
    query: String,
}

#[derive(Args)]
struct ApplyArgs {
    /// Show the plan and exit without changing anything
    #[arg(long)]
    dry_run: bool,

    /// Do not ask for confirmation
    #[arg(short = 'y', long)]
    yes: bool,
}

#[derive(Args)]
struct AddArgs {
    #[command(flatten)]
    packages: PackageArgs,

    /// Record the version installed right now, instead of any version
    #[arg(long)]
    pin: bool,
}

#[derive(Args)]
struct PackageArgs {
    /// Packages. Quote one to carry a version: `'ripgrep 14.1.0'`
    #[arg(required = true, value_name = "PACKAGE")]
    packages: Vec<String>,

    #[command(flatten)]
    layer: LayerArgs,
}

/// Which layer an edit targets. Defaults to the shared manifest.
#[derive(Args)]
struct LayerArgs {
    /// Target this machine's host layer instead of the shared manifest
    #[arg(long)]
    host: bool,
}

impl LayerArgs {
    fn resolve(&self, host: &str) -> Layer {
        if self.host {
            Layer::Host(host.to_string())
        } else {
            Layer::Common
        }
    }
}

/// Rust ignores `SIGPIPE`, so `mpm status | head` panics on the first write
/// after the reader exits. Restoring the default lets the process die quietly,
/// the way a command-line tool is expected to.
#[cfg(unix)]
fn restore_sigpipe() {
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
}

#[cfg(not(unix))]
fn restore_sigpipe() {}

fn main() -> ExitCode {
    restore_sigpipe();

    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{} {error:#}", report::bold("error:"));
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let managers = cli.managers();

    // Bare `mpm` is someone asking what this does, not asking to run anything.
    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        return Ok(ExitCode::SUCCESS);
    };

    let ctx = Ctx::discover()?;

    match command {
        Command::Status => {
            // Non-zero on drift, so mpm is usable in CI and shell prompts.
            let drifted = commands::status::status(&ctx, &managers)?;
            return Ok(if drifted {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            });
        }

        Command::Apply(args) => commands::apply::apply(
            &ctx,
            &managers,
            &ApplyOpts {
                dry_run: args.dry_run,
                yes: args.yes,
            },
        )?,

        Command::Inherit(layer) => {
            commands::inherit::inherit(&ctx, &managers, &layer.resolve(&ctx.host))?;
        }

        Command::Add(args) => {
            let layer = args.packages.layer.resolve(&ctx.host);
            let managers = require_managers(&managers, "add")?;
            commands::edit::add(&ctx, managers, &args.packages.packages, &layer, args.pin)?;
        }

        Command::Pin(args) => {
            let layer = args.layer.resolve(&ctx.host);
            let managers = require_managers(&managers, "pin")?;
            commands::edit::pin(&ctx, managers, &args.packages, &layer)?;
        }

        Command::Unpin(args) => {
            let layer = args.layer.resolve(&ctx.host);
            let managers = require_managers(&managers, "unpin")?;
            commands::edit::unpin(&ctx, managers, &args.packages, &layer)?;
        }

        Command::Remove(args) => {
            let layer = args.layer.resolve(&ctx.host);
            let managers = require_managers(&managers, "remove")?;
            commands::edit::remove(&ctx, managers, &args.packages, &layer)?;
        }

        Command::Outdated => commands::outdated::outdated(&ctx, &managers)?,

        Command::Upgrade(args) => commands::upgrade::upgrade(
            &ctx,
            &managers,
            &ApplyOpts {
                dry_run: args.dry_run,
                yes: args.yes,
            },
        )?,

        Command::Search(args) => commands::search::search(&ctx, &managers, &args.query)?,

        Command::Managers => {
            if !managers.is_empty() {
                bail!("`managers` lists what this machine has; it takes no manager of its own");
            }
            commands::managers(&ctx)?;
        }
    }

    Ok(ExitCode::SUCCESS)
}

/// Editing commands write to a named manifest, so they cannot default to "all".
fn require_managers<'a>(managers: &'a [String], command: &str) -> Result<&'a [String]> {
    if managers.is_empty() {
        bail!("name a package manager first, as in `mpm cargo {command} ...`");
    }
    Ok(managers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(argv: &[&str]) -> Cli {
        Cli::try_parse_from(argv).expect("parses")
    }

    #[test]
    fn the_cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn no_manager_name_collides_with_a_command_name() {
        // A leading token is read as a manager list unless it names a command,
        // so the two vocabularies must stay disjoint.
        let commands: Vec<String> = Cli::command()
            .get_subcommands()
            .map(|sub| sub.get_name().to_string())
            .collect();
        for id in manager::ALL {
            assert!(
                !commands.iter().any(|name| name == id),
                "`{id}` is both a manager and a command"
            );
        }
    }

    #[test]
    fn managers_lead_and_are_comma_separated() {
        let cli = parse(&["mpm", "cargo,npm", "add", "ripgrep"]);
        assert_eq!(cli.managers(), vec!["cargo", "npm"]);
        let Some(Command::Add(args)) = cli.command else {
            panic!("expected add")
        };
        assert_eq!(args.packages.packages, vec!["ripgrep"]);
    }

    #[test]
    fn packages_are_space_separated() {
        let cli = parse(&["mpm", "cargo", "add", "ripgrep", "bat", "fd"]);
        let Some(Command::Add(args)) = cli.command else {
            panic!("expected add")
        };
        assert_eq!(args.packages.packages, vec!["ripgrep", "bat", "fd"]);
    }

    #[test]
    fn managers_lead_commands_that_take_no_packages_too() {
        let cli = parse(&["mpm", "pacman,brew", "inherit"]);
        assert_eq!(cli.managers(), vec!["pacman", "brew"]);
        assert!(matches!(cli.command, Some(Command::Inherit(_))));
    }

    #[test]
    fn a_command_name_is_not_read_as_a_manager() {
        let cli = parse(&["mpm", "status"]);
        assert!(cli.managers().is_empty());
        assert!(matches!(cli.command, Some(Command::Status)));
    }

    #[test]
    fn bare_invocation_asks_for_help_rather_than_acting() {
        let cli = parse(&["mpm"]);
        assert!(cli.managers().is_empty());
        assert!(cli.command.is_none());
    }

    #[test]
    fn an_edit_without_a_manager_is_refused() {
        assert!(require_managers(&[], "add").is_err());
        assert!(require_managers(&["cargo".to_string()], "add").is_ok());
    }

    #[test]
    fn an_edit_still_needs_a_package() {
        assert!(Cli::try_parse_from(["mpm", "cargo", "add"]).is_err());
    }

    #[test]
    fn a_version_is_only_ever_recorded_deliberately() {
        let plain = parse(&["mpm", "cargo", "add", "ripgrep"]);
        let Some(Command::Add(args)) = plain.command else {
            panic!("expected add")
        };
        assert!(!args.pin);

        let asked = parse(&["mpm", "cargo", "add", "--pin", "ripgrep"]);
        let Some(Command::Add(args)) = asked.command else {
            panic!("expected add")
        };
        assert!(args.pin);
    }

    #[test]
    fn the_host_flag_chooses_the_layer() {
        assert_eq!(LayerArgs { host: false }.resolve("thinkpad"), Layer::Common);
        assert_eq!(
            LayerArgs { host: true }.resolve("thinkpad"),
            Layer::Host("thinkpad".into())
        );
    }

    #[test]
    fn a_pin_is_one_quoted_argument() {
        let cli = parse(&["mpm", "cargo", "add", "ripgrep 14.1.0"]);
        let Some(Command::Add(args)) = cli.command else {
            panic!("expected add")
        };
        assert_eq!(args.packages.packages, vec!["ripgrep 14.1.0"]);
    }
}
