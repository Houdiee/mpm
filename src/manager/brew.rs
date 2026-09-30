use anyhow::Result;

use crate::manager::{Manager, parse_two_column};
use crate::exec::Invocation;
use crate::manifest::grammar::PackageSpec;

pub struct Brew;

impl Manager for Brew {
    fn id(&self) -> &'static str {
        "brew"
    }

    fn install(&self, packages: &[PackageSpec]) -> Result<Vec<Invocation>> {
        if packages.is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![Invocation::new("brew").arg("install").args(packages.iter().map(|s| s.name.clone()))])
    }

    fn uninstall(&self, names: &[String]) -> Vec<Invocation> {
        if names.is_empty() {
            return Vec::new();
        }
        vec![Invocation::new("brew").arg("uninstall").args(names.iter().cloned())]
    }

    fn list(&self) -> Invocation {
        Invocation::new("brew").args(["list", "--formula", "--versions"])
    }

    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        parse_two_column(stdout)
    }

    /// Homebrew ships versioned formulae as distinct packages: `node@20`,
    /// `python@3.12`, `postgresql@16`. The `@` belongs to the name.
    fn name_selects_version(&self, name: &str) -> bool {
        match name.split_once('@') {
            Some((head, tail)) => {
                !head.is_empty() && tail.starts_with(|character: char| character.is_ascii_digit())
            }
            None => false,
        }
    }

    /// Homebrew cannot install an arbitrary old version; use a versioned
    /// formula such as `node@20` instead.
    fn supports_pinning(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_output_is_parsed() {
        let out = "git 2.46.0\nripgrep 14.1.0\nvim 9.1.0750\n";
        assert_eq!(
            Brew.parse_list(out),
            vec![
                PackageSpec::pinned("git", "2.46.0"),
                PackageSpec::pinned("ripgrep", "14.1.0"),
                PackageSpec::pinned("vim", "9.1.0750"),
            ]
        );
    }

    #[test]
    fn a_versioned_formula_keeps_its_name_intact() {
        assert_eq!(
            Brew.parse_list("node@20 20.11.0\n"),
            vec![PackageSpec::pinned("node@20", "20.11.0")]
        );
    }

    #[test]
    fn a_formula_with_several_versions_keeps_the_first() {
        assert_eq!(
            Brew.parse_list("openssl 3.3.1 3.2.0\n"),
            vec![PackageSpec::pinned("openssl", "3.3.1")]
        );
    }

    #[test]
    fn versioned_formulae_are_recognised() {
        for name in ["node@20", "python@3.12", "postgresql@16"] {
            assert!(Brew.name_selects_version(name), "`{name}` should select a version");
        }
        for name in ["ripgrep", "vim"] {
            assert!(!Brew.name_selects_version(name));
        }
    }

    #[test]
    fn an_at_not_followed_by_a_digit_is_not_a_version() {
        assert!(!Brew.name_selects_version("@scope/pkg"));
        assert!(!Brew.name_selects_version("foo@bar"));
    }
}
