use crate::exec::Invocation;
use crate::manager::{Manager, batched};
use crate::manifest::grammar::PackageSpec;

pub struct Krew;

impl Manager for Krew {
    fn id(&self) -> &'static str {
        "krew"
    }

    /// krew installs itself as a kubectl plugin, so it answers to `kubectl-krew`
    /// directly and needs no working cluster to list or install.
    fn binary(&self) -> &'static str {
        "kubectl-krew"
    }

    fn install_commands(&self, packages: &[PackageSpec]) -> Vec<Invocation> {
        batched(
            Invocation::new("kubectl-krew").arg("install"),
            packages.iter().map(|spec| spec.name.clone()),
        )
    }

    fn uninstall_commands(&self, installed: &[PackageSpec]) -> Vec<Invocation> {
        batched(
            Invocation::new("kubectl-krew").arg("uninstall"),
            installed.iter().map(|spec| spec.name.clone()),
        )
    }

    fn upgrade_commands(&self, unpinned: &[String], _pinned: &[String]) -> Vec<Invocation> {
        batched(
            Invocation::new("kubectl-krew").arg("upgrade"),
            unpinned.iter().cloned(),
        )
    }

    fn search_command(&self, query: &str) -> Option<Invocation> {
        Some(Invocation::new("kubectl-krew").arg("search").arg(query))
    }

    fn list_command(&self) -> Invocation {
        Invocation::new("kubectl-krew").arg("list")
    }

    /// Plugin names, one per line, with no version column. The "add this to your
    /// PATH" notice krew prints goes to stderr, so stdout is just the names.
    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        stdout
            .lines()
            .filter_map(|line| {
                let name = line.split_whitespace().next()?;
                if name.is_empty() || name == "PLUGIN" {
                    return None;
                }
                Some(PackageSpec::new(name))
            })
            .collect()
    }

    /// `krew list` reports no version, so a declared one could never be checked.
    fn supports_pinning(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `kubectl-krew list` after installing ctx and ns.
    const LIST: &str = "ctx\nkrew\nns\n";

    #[test]
    fn plugin_names_are_parsed() {
        assert_eq!(
            Krew.parse_list(LIST),
            vec![
                PackageSpec::new("ctx"),
                PackageSpec::new("krew"),
                PackageSpec::new("ns"),
            ]
        );
    }

    #[test]
    fn nothing_comes_back_pinned() {
        assert!(Krew.parse_list(LIST).iter().all(|s| s.version.is_none()));
    }
}
