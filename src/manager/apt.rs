use crate::exec::Invocation;
use crate::manager::{Manager, batched, pinned_with};
use crate::manifest::grammar::PackageSpec;

pub struct Apt;

impl Manager for Apt {
    fn id(&self) -> &'static str {
        "apt"
    }

    fn install_commands(&self, packages: &[PackageSpec]) -> Vec<Invocation> {
        // apt-get, not apt: apt's own manual warns its CLI is unstable for
        // scripting. -y here because mpm already took confirmation; deliberately
        // not on removals below, where apt's prompt is a useful second gate.
        batched(
            Invocation::new("apt-get")
                .with_root()
                .args(["install", "-y"]),
            packages.iter().map(|spec| pinned_with(spec, "=")),
        )
    }

    fn uninstall_commands(&self, installed: &[PackageSpec]) -> Vec<Invocation> {
        // `remove`, never `purge`: purge also deletes the package's configuration
        // files, which is not something a package-list sync should decide.
        batched(
            Invocation::new("apt-get").with_root().arg("remove"),
            installed.iter().map(|spec| spec.name.clone()),
        )
    }

    fn upgrade_commands(&self, unpinned: &[String], _pinned: &[String]) -> Vec<Invocation> {
        batched(
            Invocation::new("apt-get")
                .with_root()
                .args(["install", "--only-upgrade", "-y"]),
            unpinned.iter().cloned(),
        )
    }

    fn outdated_command(&self) -> Option<Invocation> {
        Some(Invocation::new("apt").args(["list", "--upgradable"]))
    }

    /// `name/suite available arch [upgradable from: installed]`, after a
    /// `Listing...` header.
    fn parse_outdated(&self, stdout: &str) -> Vec<PackageSpec> {
        stdout
            .lines()
            .filter(|line| line.contains('/') && line.contains('['))
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let name = fields.next()?.split('/').next()?;
                let available = fields.next()?;
                Some(PackageSpec::pinned(name, available))
            })
            .collect()
    }

    fn search_command(&self, query: &str) -> Option<Invocation> {
        Some(Invocation::new("apt-cache").args(["search"]).arg(query))
    }

    fn list_command(&self) -> Invocation {
        // Manual packages only; the rest are dependencies.
        Invocation::new("apt-mark").arg("showmanual")
    }

    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        // apt-mark reports names without versions.
        stdout
            .lines()
            .filter_map(|line| {
                let name = line.split_whitespace().next()?;
                Some(PackageSpec::new(name))
            })
            .collect()
    }

    /// Declaring a version is rejected: Debian and Ubuntu prune old versions
    /// from their archives, so `name=version` works today and fails once the
    /// version is dropped. That is not a manifest that keeps its promise.
    fn supports_pinning(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `apt list --upgradable` in debian:stable-slim.
    const OUTDATED: &str = "\
Listing...
libpcre2-8-0/stable-security 10.46-1~deb13u3 amd64 [upgradable from: 10.46-1~deb13u2]
libssl3t64/stable-security 3.5.7-1~deb13u3 amd64 [upgradable from: 3.5.7-1~deb13u2]
";

    #[test]
    fn showmanual_output_is_parsed() {
        let out = "git\nripgrep\nvim\n";
        assert_eq!(
            Apt.parse_list(out),
            vec![
                PackageSpec::new("git"),
                PackageSpec::new("ripgrep"),
                PackageSpec::new("vim")
            ]
        );
    }

    #[test]
    fn pins_use_equals_syntax() {
        let cmds = Apt.install_commands(&[
            PackageSpec::pinned("ripgrep", "14.1.0"),
            PackageSpec::new("vim"),
        ]);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].args, vec!["install", "-y", "ripgrep=14.1.0", "vim"]);
    }

    #[test]
    fn removal_never_purges() {
        let names = vec![PackageSpec::new("vim")];
        assert_eq!(Apt.uninstall_commands(&names)[0].args[0], "remove");
    }

    #[test]
    fn removal_is_not_auto_confirmed() {
        let names = vec![PackageSpec::new("vim")];
        let cmd = &Apt.uninstall_commands(&names)[0];
        assert!(!cmd.args.iter().any(|a| a == "-y"));
    }

    #[test]
    fn install_and_remove_need_root() {
        assert!(Apt.install_commands(&[PackageSpec::new("vim")])[0].needs_root);
        assert!(Apt.uninstall_commands(&[PackageSpec::new("vim")])[0].needs_root);
    }

    #[test]
    fn the_suite_is_stripped_and_the_header_skipped() {
        assert_eq!(
            Apt.parse_outdated(OUTDATED),
            vec![
                PackageSpec::pinned("libpcre2-8-0", "10.46-1~deb13u3"),
                PackageSpec::pinned("libssl3t64", "3.5.7-1~deb13u3"),
            ]
        );
    }
}
