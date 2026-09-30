use anyhow::Result;
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
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Show how this machine differs from its manifests
    Status(Scope),

    /// Install and remove packages so this machine matches its manifests
    Apply(ApplyArgs),

    /// Put installed packages under management, merging with what the manifest says
    Inherit(InheritArgs),

    /// Declare packages, to be installed by `mpm apply`
    Add(AddArgs),

    /// Lock packages to the version installed right now
    Pin(PackageArgs),

    /// Let packages track whatever version is current
    Unpin(PackageArgs),

    /// Undeclare packages and uninstall them now
    Remove(PackageArgs),

    /// Show supported package managers and how this machine is configured
    Managers,
}

#[derive(Args)]
struct Scope {
    /// Package managers to act on. Defaults to every managed one
    #[arg(value_name = "MANAGER")]
    managers: Vec<String>,
}

#[derive(Args)]
struct ApplyArgs {
    #[command(flatten)]
    scope: Scope,

    /// Show the plan and exit without changing anything
    #[arg(long)]
    dry_run: bool,

    /// Do not ask for confirmation
    #[arg(short = 'y', long)]
    yes: bool,
}

#[derive(Args)]
struct InheritArgs {
    #[command(flatten)]
    scope: Scope,

    #[command(flatten)]
    layer: LayerArgs,
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
    /// Package manager to act on. Several may be given, comma-separated
    #[arg(value_name = "MANAGER")]
    manager: String,

    /// Packages. Quote one to carry a version: `'ripgrep 14.1.0'`
    #[arg(required = true, value_name = "PACKAGE")]
    packages: Vec<String>,

    #[command(flatten)]
    layer: LayerArgs,
}

impl PackageArgs {
    fn managers(&self) -> Vec<String> {
        split_managers(&self.manager)
    }
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
        if self.host { Layer::Host(host.to_string()) } else { Layer::Common }
    }
}

fn split_managers(spec: &str) -> Vec<String> {
    spec.split(',').map(str::trim).filter(|name| !name.is_empty()).map(str::to_string).collect()
}

fn main() -> ExitCode {
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

    // Bare `mpm` is someone asking what this does, not asking to run anything.
    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        return Ok(ExitCode::SUCCESS);
    };

    let ctx = Ctx::discover()?;

    match command {
        Command::Status(scope) => {
            // Non-zero on drift, so mpm is usable in CI and shell prompts.
            let drifted = commands::status::status(&ctx, &scope.managers)?;
            return Ok(if drifted { ExitCode::FAILURE } else { ExitCode::SUCCESS });
        }

        Command::Apply(args) => commands::apply::apply(
            &ctx,
            &args.scope.managers,
            &ApplyOpts { dry_run: args.dry_run, yes: args.yes },
        )?,

        Command::Inherit(args) => {
            let layer = args.layer.resolve(&ctx.host);
            commands::inherit::inherit(&ctx, &args.scope.managers, &layer)?;
        }

        Command::Add(args) => {
            let layer = args.packages.layer.resolve(&ctx.host);
            commands::edit::add(
                &ctx,
                &args.packages.managers(),
                &args.packages.packages,
                &layer,
                args.pin,
            )?;
        }

        Command::Pin(args) => {
            let layer = args.layer.resolve(&ctx.host);
            commands::edit::pin(&ctx, &args.managers(), &args.packages, &layer)?;
        }

        Command::Unpin(args) => {
            let layer = args.layer.resolve(&ctx.host);
            commands::edit::unpin(&ctx, &args.managers(), &args.packages, &layer)?;
        }

        Command::Remove(args) => {
            let layer = args.layer.resolve(&ctx.host);
            commands::edit::remove(&ctx, &args.managers(), &args.packages, &layer)?;
        }

        Command::Managers => commands::managers(&ctx)?,
    }

    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cli_definition_is_valid() {
        Cli::command().debug_assert();
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
    fn bare_invocation_asks_for_help_rather_than_acting() {
        let cli = Cli::try_parse_from(["mpm"]).expect("bare mpm parses");
        assert!(cli.command.is_none());
    }

    #[test]
    fn a_version_is_only_ever_recorded_deliberately() {
        let plain = Cli::try_parse_from(["mpm", "add", "cargo", "ripgrep"]).expect("parses");
        let Some(Command::Add(args)) = plain.command else { panic!("expected add") };
        assert!(!args.pin);

        let asked =
            Cli::try_parse_from(["mpm", "add", "--pin", "cargo", "ripgrep"]).expect("parses");
        let Some(Command::Add(args)) = asked.command else { panic!("expected add") };
        assert!(args.pin);
    }

    #[test]
    fn managers_are_positional_and_may_be_several() {
        let cli = Cli::try_parse_from(["mpm", "status", "cargo", "npm"]).expect("parses");
        let Some(Command::Status(scope)) = cli.command else { panic!("expected status") };
        assert_eq!(scope.managers, vec!["cargo", "npm"]);
    }

    #[test]
    fn no_manager_means_every_managed_one() {
        let cli = Cli::try_parse_from(["mpm", "status"]).expect("parses");
        let Some(Command::Status(scope)) = cli.command else { panic!("expected status") };
        assert!(scope.managers.is_empty());
    }

    #[test]
    fn an_edit_takes_the_manager_first_then_packages() {
        let cli = Cli::try_parse_from(["mpm", "add", "cargo", "ripgrep", "bat"]).expect("parses");
        let Some(Command::Add(args)) = cli.command else { panic!("expected add") };
        assert_eq!(args.packages.managers(), vec!["cargo"]);
        assert_eq!(args.packages.packages, vec!["ripgrep", "bat"]);
    }

    #[test]
    fn an_edit_may_name_several_managers_comma_separated() {
        let cli = Cli::try_parse_from(["mpm", "add", "cargo,npm", "ripgrep"]).expect("parses");
        let Some(Command::Add(args)) = cli.command else { panic!("expected add") };
        assert_eq!(args.packages.managers(), vec!["cargo", "npm"]);
        assert_eq!(args.packages.packages, vec!["ripgrep"]);
    }

    #[test]
    fn an_edit_needs_both_a_manager_and_a_package() {
        assert!(Cli::try_parse_from(["mpm", "add"]).is_err());
        assert!(Cli::try_parse_from(["mpm", "add", "cargo"]).is_err());
    }

    #[test]
    fn a_pin_is_one_quoted_argument() {
        let cli =
            Cli::try_parse_from(["mpm", "add", "cargo", "ripgrep 14.1.0"]).expect("parses");
        let Some(Command::Add(args)) = cli.command else { panic!("expected add") };
        assert_eq!(args.packages.packages, vec!["ripgrep 14.1.0"]);
    }
}
