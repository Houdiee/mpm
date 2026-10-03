use crate::exec::Invocation;
use crate::manager::{Manager, batched, parse_two_column, pinned_with};
use crate::manifest::grammar::PackageSpec;

/// mise installs *versions of tools* rather than packages, which is the same
/// shape mpm already converges: a set of names, each optionally pinned. A version
/// manager fits the model better than some package managers do, because pinning
/// is the entire point of one.
pub struct Mise;

impl Manager for Mise {
    fn id(&self) -> &'static str {
        "mise"
    }

    /// `use --global`, not `install`: it installs the version *and* makes it the
    /// active one, which is what makes a declared version converge. A plain
    /// `install` adds a version alongside the active one and changes nothing mpm
    /// can see.
    fn install_commands(&self, packages: &[PackageSpec]) -> Vec<Invocation> {
        batched(
            Invocation::new("mise").args(["use", "--global"]),
            packages.iter().map(|spec| pinned_with(spec, "@")),
        )
    }

    fn uninstall_commands(&self, installed: &[PackageSpec]) -> Vec<Invocation> {
        batched(
            Invocation::new("mise").args(["uninstall", "--all"]),
            installed.iter().map(|spec| spec.name.clone()),
        )
    }

    fn upgrade_commands(&self, unpinned: &[String], _pinned: &[String]) -> Vec<Invocation> {
        batched(
            Invocation::new("mise").arg("upgrade"),
            unpinned.iter().cloned(),
        )
    }

    /// `--current`, not `--installed`.
    ///
    /// mise keeps every version it has ever installed, so `--installed` reports
    /// several rows named `node` and mpm -- which keys state by name -- could
    /// never tell which one a declaration meant. `--current` reports exactly one
    /// row per tool: the version actually in use. That is the thing worth
    /// converging, and it is unique, so a pin can be checked.
    ///
    /// Versions installed but not active are left alone rather than reported as
    /// drift; they are not state mpm claims to manage.
    fn list_command(&self) -> Invocation {
        Invocation::new("mise").args(["ls", "--current"])
    }

    /// `tool version <config source> <requested>`; only the first two columns are
    /// ours, and the rest are dropped.
    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        parse_two_column(stdout)
    }

    /// A tool has exactly one active version and `mise use --global` moves it.
    /// Verified: with 22.11.0 active and 20.18.0 also installed,
    /// `mise use --global node@20.18.0` leaves `ls --current` reporting 20.18.0.
    fn supports_pinning(&self) -> bool {
        true
    }

    /// `mise ls --current` says nothing at all before the first install.
    fn empty_until_first_install(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `mise ls --current` after `mise use -g node@22`.
    const LIST: &str = "node  22.23.3  ~/.config/mise/config.toml  22\npython  3.13.1\n";

    #[test]
    fn the_tool_and_its_version_are_parsed() {
        assert_eq!(
            Mise.parse_list(LIST),
            vec![
                PackageSpec::pinned("node", "22.23.3"),
                PackageSpec::pinned("python", "3.13.1"),
            ]
        );
    }

    #[test]
    fn a_pin_moves_the_active_version() {
        let commands = Mise.install_commands(&[PackageSpec::pinned("node", "20.18.0")]);
        assert_eq!(
            commands[0].args,
            vec!["use", "--global", "node@20.18.0"],
            "a plain `install` would add a version without activating it"
        );
    }

    #[test]
    fn only_the_active_version_is_listed() {
        assert_eq!(Mise.list_command().args, vec!["ls", "--current"]);
    }
}
