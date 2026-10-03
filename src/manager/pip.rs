use crate::exec::Invocation;
use crate::manager::{Manager, batched, pinned_with};
use crate::manifest::grammar::PackageSpec;

pub struct Pip;

impl Manager for Pip {
    fn id(&self) -> &'static str {
        "pip"
    }

    /// `--user` throughout, which is what makes pip safe to converge.
    ///
    /// Without it, pip reports and can remove the Python packages the
    /// distribution installed, exactly the hazard `gem` had. The user site is a
    /// per-machine global thing mpm can own outright.
    ///
    /// On a distribution whose Python is marked externally managed (PEP 668) pip
    /// refuses to install at all, and mpm surfaces that refusal rather than
    /// passing `--break-system-packages` to defeat it. `pipx` and `uv` are the
    /// managers for that machine.
    fn install_commands(&self, packages: &[PackageSpec]) -> Vec<Invocation> {
        batched(
            Invocation::new("pip").args(["install", "--user"]),
            packages.iter().map(|spec| pinned_with(spec, "==")),
        )
    }

    fn uninstall_commands(&self, installed: &[PackageSpec]) -> Vec<Invocation> {
        batched(
            Invocation::new("pip").args(["uninstall", "--yes"]),
            installed.iter().map(|spec| spec.name.clone()),
        )
    }

    fn upgrade_commands(&self, unpinned: &[String], _pinned: &[String]) -> Vec<Invocation> {
        batched(
            Invocation::new("pip").args(["install", "--user", "--upgrade"]),
            unpinned.iter().cloned(),
        )
    }

    /// `--not-required` leaves out anything that is only present as another
    /// package's dependency, so a manifest holds what was asked for.
    fn list_command(&self) -> Invocation {
        Invocation::new("pip").args(["list", "--user", "--not-required", "--format=freeze"])
    }

    fn outdated_command(&self) -> Option<Invocation> {
        Some(Invocation::new("pip").args(["list", "--user", "--outdated", "--format=columns"]))
    }

    /// `Package Version Latest Type`, after a header and a rule.
    fn parse_outdated(&self, stdout: &str) -> Vec<PackageSpec> {
        stdout
            .lines()
            .skip_while(|line| !line.starts_with("---"))
            .skip(1)
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let name = fields.next()?;
                let latest = fields.nth(1)?;
                Some(PackageSpec::pinned(name, latest))
            })
            .collect()
    }

    /// `name==version`, pip's own freeze spelling.
    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        stdout
            .lines()
            .filter_map(|line| {
                let (name, version) = line.trim().split_once("==")?;
                if name.is_empty() || version.is_empty() {
                    return None;
                }
                Some(PackageSpec::pinned(name, version))
            })
            .collect()
    }

    /// PyPI keeps published releases.
    fn supports_pinning(&self) -> bool {
        true
    }

    /// A machine with nothing in its user site has no such directory, which pip
    /// reports as a failure rather than an empty list.
    fn empty_until_first_install(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `pip list --user --not-required --format=freeze` in
    // python:slim after `pip install --user cowsay requests`.
    const LIST: &str = "cowsay==6.1\nrequests==2.34.2\n";

    #[test]
    fn freeze_syntax_is_parsed() {
        assert_eq!(
            Pip.parse_list(LIST),
            vec![
                PackageSpec::pinned("cowsay", "6.1"),
                PackageSpec::pinned("requests", "2.34.2"),
            ]
        );
    }

    #[test]
    fn a_dependency_is_not_listed() {
        // `requests` pulls in urllib3 and certifi; `--not-required` keeps them
        // out, so they never read as drift.
        let names: Vec<String> = Pip.parse_list(LIST).into_iter().map(|s| s.name).collect();
        assert!(!names.contains(&"urllib3".to_string()));
    }

    #[test]
    fn every_command_stays_in_the_user_site() {
        assert!(Pip.list_command().args.contains(&"--user".to_string()));
        assert!(
            Pip.install_commands(&[PackageSpec::new("cowsay")])[0]
                .args
                .contains(&"--user".to_string())
        );
    }

    #[test]
    fn the_latest_column_is_the_one_available() {
        let outdated = "\
Package  Version Latest Type
-------- ------- ------ -----
cowsay   5.0     6.1    wheel
";
        assert_eq!(
            Pip.parse_outdated(outdated),
            vec![PackageSpec::pinned("cowsay", "6.1")]
        );
    }
}
