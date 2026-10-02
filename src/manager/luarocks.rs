use crate::exec::Invocation;
use crate::manager::{Manager, batched, parse_two_column};
use crate::manifest::grammar::PackageSpec;

pub struct Luarocks;

impl Manager for Luarocks {
    fn id(&self) -> &'static str {
        "luarocks"
    }

    fn install_commands(&self, packages: &[PackageSpec]) -> Vec<Invocation> {
        batched(
            Invocation::new("luarocks").arg("install"),
            packages.iter().map(|spec| spec.name.clone()),
        )
    }

    fn uninstall_commands(&self, names: &[String]) -> Vec<Invocation> {
        batched(
            Invocation::new("luarocks").arg("remove"),
            names.iter().cloned(),
        )
    }

    fn search_command(&self, query: &str) -> Option<Invocation> {
        Some(Invocation::new("luarocks").args(["search"]).arg(query))
    }

    fn list_command(&self) -> Invocation {
        Invocation::new("luarocks").args(["list", "--porcelain"])
    }

    /// Tab-separated `name version status path`; only the first two matter.
    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        parse_two_column(stdout)
    }

    /// LuaRocks takes a version as a separate trailing argument, which cannot be
    /// given per package in one batched command.
    fn supports_pinning(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `luarocks list --porcelain`.
    const LIST: &str =
        "inspect\t3.1.3-0\tinstalled\t/home/kerim/.luarocks/lib/luarocks/rocks-5.2\n";

    #[test]
    fn porcelain_output_is_parsed() {
        assert_eq!(
            Luarocks.parse_list(LIST),
            vec![PackageSpec::pinned("inspect", "3.1.3-0")]
        );
    }

    #[test]
    fn the_trailing_path_is_dropped() {
        assert_eq!(
            Luarocks.parse_list(LIST)[0].version.as_deref(),
            Some("3.1.3-0")
        );
    }

    #[test]
    fn removals_share_one_command() {
        let commands =
            Luarocks.uninstall_commands(&["inspect".to_string(), "penlight".to_string()]);
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].args, vec!["remove", "inspect", "penlight"]);
    }
}
