use crate::exec::Invocation;
use crate::manager::{Manager, batched, parse_two_column, pinned_with};
use crate::manifest::grammar::PackageSpec;

pub struct Composer;

impl Manager for Composer {
    fn id(&self) -> &'static str {
        "composer"
    }

    fn install_commands(&self, packages: &[PackageSpec]) -> Vec<Invocation> {
        batched(
            Invocation::new("composer").args(["global", "require", "--no-interaction"]),
            packages.iter().map(|spec| pinned_with(spec, ":")),
        )
    }

    fn uninstall_commands(&self, names: &[String]) -> Vec<Invocation> {
        batched(
            Invocation::new("composer").args(["global", "remove", "--no-interaction"]),
            names.iter().cloned(),
        )
    }

    fn upgrade_commands(&self, unpinned: &[String], _pinned: &[String]) -> Vec<Invocation> {
        batched(
            Invocation::new("composer").args(["global", "update", "--no-interaction"]),
            unpinned.iter().cloned(),
        )
    }

    fn outdated_command(&self) -> Option<Invocation> {
        // --direct for the same reason as `list_command`: a transitive
        // dependency is not something the manifest can act on.
        Some(Invocation::new("composer").args(["global", "outdated", "--direct"]))
    }

    /// `name installed ~ available description`
    fn parse_outdated(&self, stdout: &str) -> Vec<PackageSpec> {
        stdout
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let name = fields.next()?;
                let _installed = fields.next()?;
                let available = fields.find(|field| *field != "~")?;
                Some(PackageSpec::pinned(name, available))
            })
            .collect()
    }

    fn search_command(&self, query: &str) -> Option<Invocation> {
        Some(Invocation::new("composer").args(["search"]).arg(query))
    }

    /// `--direct` because plain `show` also lists transitive dependencies, which
    /// are installed but not declared -- mpm's definition of a removal candidate.
    /// Without it, `apply` tries to remove a package's own dependencies.
    fn list_command(&self) -> Invocation {
        Invocation::new("composer").args(["global", "show", "--direct"])
    }

    /// `name version description`, column-aligned; the description is dropped.
    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        parse_two_column(stdout)
    }

    fn supports_pinning(&self) -> bool {
        true
    }

    /// `composer global show` fails until the global directory has a
    /// composer.json, which is the state of a machine that has never used it.
    fn empty_until_first_install(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `composer global outdated` in composer:latest.
    const OUTDATED: &str = "psr/log 1.1.4 ~ 3.0.2 Common interface for logging libraries\n";

    // Captured from `composer global show`.
    const LIST: &str = "\
psr/container    2.0.2 Common Container Interface (PHP FIG PSR-11)
psr/log          3.0.2 Common interface for logging libraries
psr/simple-cache 3.0.0 Common interfaces for simple caching
";

    #[test]
    fn aligned_columns_are_parsed_and_descriptions_dropped() {
        assert_eq!(
            Composer.parse_list(LIST),
            vec![
                PackageSpec::pinned("psr/container", "2.0.2"),
                PackageSpec::pinned("psr/log", "3.0.2"),
                PackageSpec::pinned("psr/simple-cache", "3.0.0"),
            ]
        );
    }

    #[test]
    fn a_vendor_name_survives_intact() {
        assert_eq!(Composer.parse_list(LIST)[0].name, "psr/container");
    }

    #[test]
    fn pins_use_a_colon() {
        let commands = Composer.install_commands(&[
            PackageSpec::new("psr/log"),
            PackageSpec::pinned("psr/container", "2.0.1"),
        ]);
        assert_eq!(
            commands[0].args,
            vec![
                "global",
                "require",
                "--no-interaction",
                "psr/log",
                "psr/container:2.0.1"
            ]
        );
    }

    #[test]
    fn removals_share_one_command() {
        let commands =
            Composer.uninstall_commands(&["psr/log".to_string(), "psr/container".to_string()]);
        assert_eq!(commands.len(), 1);
    }

    #[test]
    fn only_required_packages_are_listed() {
        // Measured: after `global require guzzlehttp/guzzle`, `show` reports 9
        // packages and `show --direct` the 2 asked for.
        assert_eq!(
            Composer.list_command().args,
            vec!["global", "show", "--direct"]
        );
    }

    #[test]
    fn the_version_after_the_tilde_is_the_one_available() {
        assert_eq!(
            Composer.parse_outdated(OUTDATED),
            vec![PackageSpec::pinned("psr/log", "3.0.2")]
        );
    }
}
