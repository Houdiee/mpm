use anyhow::Result;

use crate::manager::Manager;
use crate::exec::Invocation;
use crate::manifest::grammar::PackageSpec;

pub struct Cargo;

impl Manager for Cargo {
    fn id(&self) -> &'static str {
        "cargo"
    }

    fn install(&self, packages: &[PackageSpec]) -> Result<Vec<Invocation>> {
        if packages.is_empty() {
            return Ok(Vec::new());
        }
        // `cargo install` is variadic and takes a version per crate, so the
        // whole set installs as one resolution rather than one per pin.
        Ok(vec![Invocation::new("cargo").arg("install").args(packages.iter().map(crate_arg))])
    }

    fn uninstall(&self, names: &[String]) -> Vec<Invocation> {
        if names.is_empty() {
            return Vec::new();
        }
        vec![Invocation::new("cargo").arg("uninstall").args(names.iter().cloned())]
    }

    fn list(&self) -> Invocation {
        Invocation::new("cargo").args(["install", "--list"])
    }

    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        // Crates start at column zero as `name vX.Y.Z:`; their binaries follow,
        // indented.
        stdout
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with(char::is_whitespace))
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let name = parts.next()?;
                let version = parts
                    .next()
                    .map(|v| v.trim_end_matches(':').trim_start_matches('v'))
                    .filter(|v| !v.is_empty());
                Some(match version {
                    Some(version) => PackageSpec::pinned(name, version),
                    None => PackageSpec::new(name),
                })
            })
            .collect()
    }

    fn supports_pinning(&self) -> bool {
        true
    }
}

/// `cargo install` spells a version `crate@version`.
fn crate_arg(spec: &PackageSpec) -> String {
    match &spec.version {
        Some(version) => format!("{}@{}", spec.name, version),
        None => spec.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "\
cargo-edit v0.12.2:
    cargo-add
    cargo-rm
ripgrep v14.1.0:
    rg
";

    #[test]
    fn crate_lines_are_parsed_and_binaries_ignored() {
        assert_eq!(
            Cargo.parse_list(LIST),
            vec![PackageSpec::pinned("cargo-edit", "0.12.2"), PackageSpec::pinned("ripgrep", "14.1.0")]
        );
    }

    #[test]
    fn every_crate_installs_in_one_command_carrying_its_own_version() {
        let cmds = Cargo
            .install(&[
                PackageSpec::new("bat"),
                PackageSpec::pinned("ripgrep", "14.1.0"),
                PackageSpec::pinned("fd-find", "10.1.0"),
            ])
            .expect("builds");
        assert_eq!(cmds.len(), 1, "one resolution, not one per pin");
        assert_eq!(cmds[0].args, vec!["install", "bat", "ripgrep@14.1.0", "fd-find@10.1.0"]);
    }

    #[test]
    fn nothing_to_install_means_no_commands() {
        assert!(Cargo.install(&[]).expect("builds").is_empty());
    }
}
