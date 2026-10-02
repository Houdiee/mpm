use crate::exec::Invocation;
use crate::manager::{Manager, batched, pinned_with};
use crate::manifest::grammar::PackageSpec;

pub struct Uv;

impl Manager for Uv {
    fn id(&self) -> &'static str {
        "uv"
    }

    /// No `--force`: installing a different version replaces the old one and
    /// exits zero, verified against uv in its own image. The flag pipx needs is
    /// not needed here.
    fn install_commands(&self, packages: &[PackageSpec]) -> Vec<Invocation> {
        batched(
            Invocation::new("uv").args(["tool", "install"]),
            packages.iter().map(|spec| pinned_with(spec, "==")),
        )
    }

    fn uninstall_commands(&self, names: &[String]) -> Vec<Invocation> {
        batched(
            Invocation::new("uv").args(["tool", "uninstall"]),
            names.iter().cloned(),
        )
    }

    fn upgrade_commands(&self, unpinned: &[String], _pinned: &[String]) -> Vec<Invocation> {
        batched(
            Invocation::new("uv").args(["tool", "upgrade"]),
            unpinned.iter().cloned(),
        )
    }

    fn list_command(&self) -> Invocation {
        Invocation::new("uv").args(["tool", "list"])
    }

    /// `name vX.Y.Z` at column zero, with each tool's executables listed under it
    /// as `- name`. An empty install reports `No tools installed`.
    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        stdout
            .lines()
            .filter(|line| !line.starts_with(char::is_whitespace) && !line.starts_with('-'))
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let name = fields.next()?;
                let version = fields.next()?.strip_prefix('v')?;
                Some(PackageSpec::pinned(name, version))
            })
            .collect()
    }

    /// PyPI keeps published releases, so a pinned version stays installable.
    fn supports_pinning(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `uv tool list` in ghcr.io/astral-sh/uv:debian.
    const LIST: &str = "\
cowsay v6.1
- cowsay
ruff v0.14.2
- ruff
";

    #[test]
    fn tools_are_parsed_and_their_executables_ignored() {
        assert_eq!(
            Uv.parse_list(LIST),
            vec![
                PackageSpec::pinned("cowsay", "6.1"),
                PackageSpec::pinned("ruff", "0.14.2"),
            ]
        );
    }

    #[test]
    fn an_empty_install_is_not_a_package() {
        // `No tools installed` would otherwise parse as a package called `No`.
        assert!(Uv.parse_list("No tools installed\n").is_empty());
    }

    #[test]
    fn pins_use_a_double_equals() {
        let commands = Uv.install_commands(&[PackageSpec::pinned("cowsay", "6.1")]);
        assert_eq!(commands[0].args, vec!["tool", "install", "cowsay==6.1"]);
    }
}
