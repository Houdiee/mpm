use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;

use crate::exec::Invocation;
use crate::manager::{Manager, parse_two_column, partition_pinned};
use crate::manifest::grammar::PackageSpec;

pub struct Arch;

/// pacman, paru and yay read the same local database, so there is only ever one
/// Arch package set. An AUR helper is an installation detail, not a separate
/// manager: as separate managers each would see the other's packages as drift.
enum Installer {
    /// `paru` or `yay`. They wrap pacman, handle repo and AUR packages alike,
    /// and call `sudo` themselves -- so they must not be run through it.
    Helper(&'static str),
    Pacman,
}

impl Installer {
    fn detect() -> Self {
        for helper in ["paru", "yay"] {
            if which::which(helper).is_ok() {
                return Installer::Helper(helper);
            }
        }
        Installer::Pacman
    }

    fn command(&self) -> Invocation {
        match self {
            Installer::Helper(helper) => Invocation::new(helper),
            Installer::Pacman => Invocation::new("pacman").as_root(),
        }
    }
}

impl Manager for Arch {
    fn id(&self) -> &'static str {
        "pacman"
    }

    fn install(&self, packages: &[PackageSpec]) -> Result<Vec<Invocation>> {
        if packages.is_empty() {
            return Ok(Vec::new());
        }

        let (pinned, loose) = partition_pinned(packages);
        let mut commands = Vec::new();

        if !loose.is_empty() {
            // `--needed` keeps an already-satisfied package from being
            // reinstalled when a partly-applied run is retried.
            commands.push(
                Installer::detect()
                    .command()
                    .args(["-S", "--needed"])
                    .args(loose.iter().map(|spec| spec.name.clone())),
            );
        }

        // The Arch Linux Archive keeps every build the official repositories
        // have shipped, and `pacman -U` takes the URL directly.
        for spec in pinned {
            let version = spec.version.as_deref().unwrap_or_default();
            let architecture = repo_architecture(&spec.name)?;
            commands.push(
                Invocation::new("pacman")
                    .as_root()
                    .arg("-U")
                    .arg(archive_url(&spec.name, version, &architecture)),
            );
        }
        Ok(commands)
    }

    fn uninstall(&self, names: &[String]) -> Vec<Invocation> {
        if names.is_empty() {
            return Vec::new();
        }
        // -Rs removes now-orphaned dependencies but keeps configuration. -Rns
        // would delete config too, which a package-list sync must not decide.
        vec![Invocation::new("pacman").as_root().arg("-Rs").args(names.iter().cloned())]
    }

    fn list(&self) -> Invocation {
        // -Qe lists explicitly-installed packages only, so dependencies pulled
        // in automatically never show up as undeclared drift.
        Invocation::new("pacman").arg("-Qe")
    }

    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        parse_two_column(stdout)
    }

    /// The archive holds only the official repositories, so a name `pacman -Si`
    /// does not know is an AUR package -- and AUR builds are archived nowhere.
    fn check_pins(&self, specs: &[&PackageSpec]) -> Result<()> {
        if specs.is_empty() {
            return Ok(());
        }
        let names: Vec<&str> = specs.iter().map(|spec| spec.name.as_str()).collect();
        let output = std::process::Command::new("pacman")
            .arg("-Si")
            .args(&names)
            .output()
            .context("could not run `pacman -Si` to check pinned packages")?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let known = known_names(&stdout);
        let missing: Vec<&str> =
            names.into_iter().filter(|name| !known.contains(name)).collect();

        if !missing.is_empty() {
            bail!(
                "no archived build exists for {} -- these are not in the official \
                 repositories, and AUR packages cannot be pinned. Declare them without a version",
                missing.join(", ")
            );
        }
        Ok(())
    }

    /// The Arch Linux Archive keeps every historical build permanently. The
    /// version must include the pkgrel as `pacman -Qe` reports it (`14.1.0-1`).
    fn supports_pinning(&self) -> bool {
        true
    }
}

/// The architecture the archive files this package under.
///
/// Fonts, themes and pure-Python packages are `any`, so `uname -m` would 404
/// for all of them.
fn repo_architecture(name: &str) -> Result<String> {
    let output = std::process::Command::new("pacman")
        .args(["-Si", name])
        .output()
        .context("could not run `pacman -Si` to locate the package")?;

    if !output.status.success() {
        bail!(
            "`{name}` is not in the official repositories, so no archived build exists \
             (AUR packages cannot be pinned -- declare `{name}` without a version)"
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_architecture(&stdout)
        .with_context(|| format!("could not read the architecture of `{name}` from `pacman -Si`"))
}

fn known_names(stdout: &str) -> BTreeSet<&str> {
    stdout
        .lines()
        .filter_map(|line| {
            let (field, value) = line.split_once(':')?;
            if field.trim() == "Name" { Some(value.trim()) } else { None }
        })
        .collect()
}

fn parse_architecture(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let (field, value) = line.split_once(':')?;
        if field.trim() != "Architecture" {
            return None;
        }
        let value = value.trim();
        if value.is_empty() { None } else { Some(value.to_string()) }
    })
}

/// Packages are filed under their first letter, named
/// `<name>-<version>-<architecture>.pkg.tar.zst`.
fn archive_url(name: &str, version: &str, architecture: &str) -> String {
    let initial = name.chars().next().unwrap_or('_');
    format!(
        "https://archive.archlinux.org/packages/{initial}/{name}/{name}-{version}-{architecture}.pkg.tar.zst"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const QE: &str = "\
git 2.46.0-1
linux 6.10.6.arch1-1
ripgrep 14.1.0-1
";

    const SI: &str = "\
Repository      : extra
Name            : ripgrep
Version         : 14.1.1-1
Architecture    : x86_64
Depends On      : gcc-libs  pcre2
";

    const SI_ANY: &str = "\
Repository      : extra
Name            : ttf-fira-code
Version         : 6.2-4
Architecture    : any
";

    #[test]
    fn explicit_packages_are_parsed_with_versions() {
        assert_eq!(
            Arch.parse_list(QE),
            vec![
                PackageSpec::pinned("git", "2.46.0-1"),
                PackageSpec::pinned("linux", "6.10.6.arch1-1"),
                PackageSpec::pinned("ripgrep", "14.1.0-1"),
            ]
        );
    }

    #[test]
    fn listing_always_uses_pacman() {
        let list = Arch.list();
        assert_eq!(list.program, "pacman");
        assert!(!list.needs_root, "a query needs no privileges");
    }

    #[test]
    fn removal_keeps_configuration() {
        let commands = Arch.uninstall(&["vim".to_string()]);
        assert_eq!(commands[0].args[0], "-Rs");
        assert!(commands[0].needs_root);
    }

    #[test]
    fn architecture_is_read_from_pacman_si() {
        assert_eq!(parse_architecture(SI).as_deref(), Some("x86_64"));
    }

    #[test]
    fn an_any_architecture_package_is_recognised() {
        assert_eq!(parse_architecture(SI_ANY).as_deref(), Some("any"));
    }

    #[test]
    fn archive_urls_are_built_from_name_version_and_architecture() {
        assert_eq!(
            archive_url("ripgrep", "14.1.0-1", "x86_64"),
            "https://archive.archlinux.org/packages/r/ripgrep/ripgrep-14.1.0-1-x86_64.pkg.tar.zst"
        );
        assert_eq!(
            archive_url("ttf-fira-code", "6.2-4", "any"),
            "https://archive.archlinux.org/packages/t/ttf-fira-code/ttf-fira-code-6.2-4-any.pkg.tar.zst"
        );
    }

    #[test]
    fn unpinned_packages_share_one_command() {
        let commands = Arch.install(&[PackageSpec::new("vim"), PackageSpec::new("git")]).expect("builds");
        assert_eq!(commands.len(), 1);
        assert_eq!(&commands[0].args[..2], &["-S", "--needed"]);
    }

}

#[cfg(test)]
mod pin_check_tests {
    use super::*;

    const SI_MULTI: &str = "\
Repository      : extra
Name            : ripgrep
Architecture    : x86_64

Repository      : extra
Name            : ttf-fira-code
Architecture    : any
";

    #[test]
    fn names_are_collected_from_a_multi_package_query() {
        let known = known_names(SI_MULTI);
        assert!(known.contains("ripgrep"));
        assert!(known.contains("ttf-fira-code"));
        assert_eq!(known.len(), 2);
        assert!(!known.contains("my-aur-tool"));
    }

    #[test]
    fn nothing_pinned_needs_no_query() {
        Arch.check_pins(&[]).expect("no query, no failure");
    }
}
