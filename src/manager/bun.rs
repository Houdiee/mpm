use crate::exec::Invocation;
use crate::manager::node::parse_registry_tree;
use crate::manager::{Manager, batched, pinned_with};
use crate::manifest::grammar::PackageSpec;

pub struct Bun;

impl Manager for Bun {
    fn id(&self) -> &'static str {
        "bun"
    }

    fn install_commands(&self, packages: &[PackageSpec]) -> Vec<Invocation> {
        batched(
            Invocation::new("bun").args(["add", "-g"]),
            packages.iter().map(|spec| pinned_with(spec, "@")),
        )
    }

    fn uninstall_commands(&self, names: &[String]) -> Vec<Invocation> {
        batched(
            Invocation::new("bun").args(["remove", "-g"]),
            names.iter().cloned(),
        )
    }

    fn list_command(&self) -> Invocation {
        Invocation::new("bun").args(["pm", "ls", "-g"])
    }

    /// bun prints npm's box-drawing tree of `name@version`.
    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        parse_registry_tree(stdout)
    }

    fn supports_pinning(&self) -> bool {
        true
    }

    /// `bun pm ls -g` fails until its global directory has a package.json.
    fn empty_until_first_install(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `bun pm ls -g`.
    const LIST: &str = "\
/home/kerim/.bun/install/global node_modules (25)
├── @types/bun@1.4.2
└── typescript@7.0.2
";

    #[test]
    fn the_tree_is_parsed_and_the_header_skipped() {
        assert_eq!(
            Bun.parse_list(LIST),
            vec![
                PackageSpec::pinned("@types/bun", "1.4.2"),
                PackageSpec::pinned("typescript", "7.0.2"),
            ]
        );
    }

    #[test]
    fn pins_use_registry_syntax() {
        let commands = Bun.install_commands(&[PackageSpec::pinned("typescript", "7.0.2")]);
        assert_eq!(commands[0].args, vec!["add", "-g", "typescript@7.0.2"]);
    }

    #[test]
    fn removals_share_one_command() {
        let commands = Bun.uninstall_commands(&["typescript".to_string(), "esbuild".to_string()]);
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0].args,
            vec!["remove", "-g", "typescript", "esbuild"]
        );
    }
}
