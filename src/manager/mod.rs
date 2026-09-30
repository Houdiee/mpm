use anyhow::Result;

use crate::exec::Invocation;
use crate::manifest::grammar::PackageSpec;

pub mod apt;
pub mod arch;
pub mod brew;
pub mod cargo;
pub mod dotnet;
pub mod node;

/// Every package manager mpm can drive, in the order they are reported.
pub const ALL: &[&str] = &["apt", "brew", "cargo", "dotnet", "npm", "pacman", "pnpm"];

/// One package manager, described as argument vectors rather than shell strings.
///
/// `install` and `uninstall` return a *list* of invocations because some
/// managers accept only one package at a time (`dotnet tool install`) and some
/// need a different command shape per package (a pinned `pacman -U <url>`).
pub trait Manager {
    fn id(&self) -> &'static str;

    /// Executable probed with `which` to decide whether this manager exists here.
    fn binary(&self) -> &'static str {
        self.id()
    }

    /// Build the commands that install these packages.
    ///
    /// Fallible because working out how to fetch an exact version can itself
    /// fail -- a pinned Arch package has to be located in the archive first.
    fn install(&self, packages: &[PackageSpec]) -> Result<Vec<Invocation>>;

    fn uninstall(&self, names: &[String]) -> Vec<Invocation>;

    fn list(&self) -> Invocation;

    /// Turn the output of [`Manager::list`] into installed packages, with
    /// versions where the manager reports them.
    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec>;

    /// Whether an exact version can be installed *and installed again later*.
    ///
    /// A claim about reproducibility, not about today: a manager whose old
    /// versions vanish from its index does not qualify. Where this is false,
    /// declaring a version is rejected rather than approximated.
    fn supports_pinning(&self) -> bool {
        false
    }

    /// Confirm these exact versions can actually be obtained.
    ///
    /// Runs on every `status`, so a version nothing can supply is reported even
    /// while it happens to match what is installed today.
    fn check_pins(&self, _specs: &[&PackageSpec]) -> Result<()> {
        Ok(())
    }

    /// Whether this manager's package *names* can themselves select a version.
    ///
    /// Homebrew ships `node@20` and `python@3.12` as formula names. Declaring
    /// such a name *and* a version is a contradiction, and is rejected rather
    /// than silently resolved one way or the other.
    fn name_selects_version(&self, _name: &str) -> bool {
        false
    }
}

pub fn get(id: &str) -> Option<Box<dyn Manager>> {
    match id {
        "apt" => Some(Box::new(apt::Apt)),
        "brew" => Some(Box::new(brew::Brew)),
        "cargo" => Some(Box::new(cargo::Cargo)),
        "dotnet" => Some(Box::new(dotnet::Dotnet)),
        "npm" => Some(Box::new(node::Node { tool: node::NodeTool::Npm })),
        "pacman" => Some(Box::new(arch::Arch)),
        "pnpm" => Some(Box::new(node::Node { tool: node::NodeTool::Pnpm })),
        _ => None,
    }
}

pub fn present(manager: &dyn Manager) -> bool {
    which::which(manager.binary()).is_ok()
}

pub(crate) fn partition_pinned(packages: &[PackageSpec]) -> (Vec<&PackageSpec>, Vec<&PackageSpec>) {
    packages.iter().partition(|spec| spec.version.is_some())
}

/// Parse `name version` lines, tolerating extra trailing columns and blanks.
pub(crate) fn parse_two_column(stdout: &str) -> Vec<PackageSpec> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?;
            Some(match fields.next() {
                Some(version) => PackageSpec::pinned(name, version),
                None => PackageSpec::new(name),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_advertised_manager_is_constructible() {
        for id in ALL {
            let manager = get(id).unwrap_or_else(|| panic!("`{id}` is advertised but not registered"));
            assert_eq!(&manager.id(), id);
        }
    }

    #[test]
    fn the_advertised_list_is_alphabetical() {
        let mut sorted = ALL.to_vec();
        sorted.sort();
        assert_eq!(ALL, sorted.as_slice());
    }

    #[test]
    fn unknown_manager_is_rejected() {
        assert!(get("fisher").is_none());
        assert!(get("paru").is_none(), "AUR helpers are an installer detail, not a manager");
        assert!(get("").is_none());
    }

    #[test]
    fn only_managers_with_a_durable_index_accept_versions() {
        // Registries that keep every published version qualify; an index that
        // prunes old ones does not.
        for id in ["cargo", "npm", "pnpm", "dotnet", "pacman"] {
            assert!(get(id).expect("registered").supports_pinning(), "`{id}` should accept versions");
        }
        for id in ["brew", "apt"] {
            assert!(!get(id).expect("registered").supports_pinning(), "`{id}` should reject versions");
        }
    }
}
