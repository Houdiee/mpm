use crate::exec::Invocation;
use crate::manager::{Manager, batched, parse_two_column, pinned_with};
use crate::manifest::grammar::PackageSpec;

pub struct Pipx;

impl Manager for Pipx {
    fn id(&self) -> &'static str {
        "pipx"
    }

    fn install_commands(&self, packages: &[PackageSpec]) -> Vec<Invocation> {
        // --force because a plain install of an already-present package is a no-op
        // even at a different version. mpm only installs what is missing or at the
        // wrong version, so forcing is always what is meant.
        batched(
            Invocation::new("pipx").args(["install", "--force"]),
            packages.iter().map(|spec| pinned_with(spec, "==")),
        )
    }

    fn uninstall_commands(&self, names: &[String]) -> Vec<Invocation> {
        // `pipx uninstall a b` is rejected: one package per call.
        names
            .iter()
            .map(|name| Invocation::new("pipx").arg("uninstall").arg(name.as_str()))
            .collect()
    }

    fn upgrade_commands(&self, unpinned: &[String], _pinned: &[String]) -> Vec<Invocation> {
        unpinned
            .iter()
            .map(|name| Invocation::new("pipx").arg("upgrade").arg(name.as_str()))
            .collect()
    }

    fn list_command(&self) -> Invocation {
        Invocation::new("pipx").args(["list", "--short"])
    }

    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        parse_two_column(stdout)
    }

    /// PyPI keeps published releases; one disappears only if its own maintainer
    /// deletes it, not as a matter of policy.
    fn supports_pinning(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `pipx list --short`.
    const LIST: &str = "\
cowsay 5.0
pyfiglet 1.0.4
";

    #[test]
    fn short_output_is_parsed() {
        assert_eq!(
            Pipx.parse_list(LIST),
            vec![
                PackageSpec::pinned("cowsay", "5.0"),
                PackageSpec::pinned("pyfiglet", "1.0.4")
            ]
        );
    }

    #[test]
    fn nothing_installed_yields_nothing() {
        // The "nothing has been installed" notice goes to stderr.
        assert!(Pipx.parse_list("").is_empty());
    }

    #[test]
    fn installs_are_batched_and_forced() {
        let commands = Pipx.install_commands(&[
            PackageSpec::new("cowsay"),
            PackageSpec::pinned("pyfiglet", "1.0.4"),
        ]);
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0].args,
            vec!["install", "--force", "cowsay", "pyfiglet==1.0.4"]
        );
    }

    #[test]
    fn each_uninstall_gets_its_own_command() {
        let commands = Pipx.uninstall_commands(&["cowsay".to_string(), "pyfiglet".to_string()]);
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[1].args, vec!["uninstall", "pyfiglet"]);
    }

    #[test]
    fn each_upgrade_gets_its_own_command() {
        let commands = Pipx.upgrade_commands(&["cowsay".to_string(), "pyfiglet".to_string()], &[]);
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[1].args, vec!["upgrade", "pyfiglet"]);
    }
}
