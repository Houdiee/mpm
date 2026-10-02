use crate::exec::Invocation;
use crate::manager::{Manager, batched, split_hyphenated};
use crate::manifest::grammar::PackageSpec;

pub struct Nix;

impl Manager for Nix {
    fn id(&self) -> &'static str {
        "nix"
    }

    /// `nix-env`, not `nix profile`: its `-q` output is one `name-version` per
    /// line, where `nix profile list` is a multi-line block whose shape changed
    /// between Nix releases. Installing by name also means the name mpm declares
    /// is the name it reads back, which an attribute path like `nixpkgs.hello`
    /// would not be.
    fn binary(&self) -> &'static str {
        "nix-env"
    }

    fn install_commands(&self, packages: &[PackageSpec]) -> Vec<Invocation> {
        batched(
            Invocation::new("nix-env").arg("--install"),
            packages.iter().map(|spec| spec.name.clone()),
        )
    }

    fn uninstall_commands(&self, names: &[String]) -> Vec<Invocation> {
        batched(
            Invocation::new("nix-env").arg("--uninstall"),
            names.iter().cloned(),
        )
    }

    fn upgrade_commands(&self, unpinned: &[String], _pinned: &[String]) -> Vec<Invocation> {
        batched(
            Invocation::new("nix-env").arg("--upgrade"),
            unpinned.iter().cloned(),
        )
    }

    fn search_command(&self, query: &str) -> Option<Invocation> {
        Some(Invocation::new("nix-env").args(["-qa"]).arg(query))
    }

    fn list_command(&self) -> Invocation {
        Invocation::new("nix-env").arg("-q")
    }

    /// `hello-2.12.3`, one per line, split at the last hyphen before a digit.
    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        stdout.lines().filter_map(split_hyphenated).collect()
    }

    /// A channel offers one version of a derivation at a time, so an older one
    /// cannot be asked for by name.
    fn supports_pinning(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `nix-env -q` in nixos/nix after `nix-env -i hello`.
    const LIST: &str = "\
hello-2.12.3
iana-etc-20251215
man-db-2.13.1
nix-2.35.2
nss-cacert-3.123
openssh-10.3p1
";

    #[test]
    fn a_name_and_version_come_apart() {
        let parsed = Nix.parse_list(LIST);
        assert_eq!(parsed[0], PackageSpec::pinned("hello", "2.12.3"));
    }

    #[test]
    fn a_hyphenated_name_survives() {
        let parsed = Nix.parse_list(LIST);
        let cacert = parsed
            .iter()
            .find(|spec| spec.name == "nss-cacert")
            .expect("nss-cacert keeps its hyphen");
        assert_eq!(cacert.version.as_deref(), Some("3.123"));
        assert!(parsed.iter().any(|spec| spec.name == "iana-etc"));
        assert!(parsed.iter().any(|spec| spec.name == "man-db"));
    }

    #[test]
    fn a_version_with_a_letter_still_splits() {
        let parsed = Nix.parse_list(LIST);
        let ssh = parsed
            .iter()
            .find(|spec| spec.name == "openssh")
            .expect("openssh");
        assert_eq!(ssh.version.as_deref(), Some("10.3p1"));
    }

    #[test]
    fn nothing_is_run_as_root() {
        // nix-env writes a per-user profile; elevating would write root's instead.
        assert!(!Nix.install_commands(&[PackageSpec::new("hello")])[0].needs_root);
        assert!(!Nix.uninstall_commands(&["hello".to_string()])[0].needs_root);
    }
}
