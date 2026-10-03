use crate::exec::Invocation;
use crate::manager::Manager;
use crate::manifest::grammar::PackageSpec;

/// pyenv manages Python versions, so a declaration names a version directly --
/// the same shape as `rustup`, where the toolchain name carries the version.
pub struct Pyenv;

impl Manager for Pyenv {
    fn id(&self) -> &'static str {
        "pyenv"
    }

    /// One per call: `pyenv install` builds a Python and takes a single version.
    fn install_commands(&self, packages: &[PackageSpec]) -> Vec<Invocation> {
        packages
            .iter()
            .map(|spec| {
                Invocation::new("pyenv")
                    .args(["install", "--skip-existing"])
                    .arg(spec.name.as_str())
            })
            .collect()
    }

    /// `--force` so removal does not stop to ask.
    fn uninstall_commands(&self, installed: &[PackageSpec]) -> Vec<Invocation> {
        installed
            .iter()
            .map(|spec| {
                Invocation::new("pyenv")
                    .args(["uninstall", "--force"])
                    .arg(spec.name.as_str())
            })
            .collect()
    }

    /// `--bare` prints one version per line and leaves out both the `*` active
    /// marker and the `system` entry, which is the host's own Python rather than
    /// anything pyenv installed.
    fn list_command(&self) -> Invocation {
        Invocation::new("pyenv").args(["versions", "--bare"])
    }

    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && *line != "system")
            .map(PackageSpec::new)
            .collect()
    }

    /// A version *is* the name here, so a separate version column would be a
    /// contradiction -- as with `rustup`.
    fn name_selects_version(&self, _name: &str) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `pyenv versions --bare` after `pyenv install 3.12.8`.
    const LIST: &str = "3.12.8\n3.11.9\n";

    #[test]
    fn versions_are_the_names() {
        assert_eq!(
            Pyenv.parse_list(LIST),
            vec![PackageSpec::new("3.12.8"), PackageSpec::new("3.11.9")]
        );
    }

    #[test]
    fn the_hosts_own_python_is_not_managed() {
        // `pyenv versions` shows `system`; mpm must not offer to uninstall it.
        let names: Vec<String> = Pyenv
            .parse_list("system\n3.12.8\n")
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, vec!["3.12.8"]);
    }
}
