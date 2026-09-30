use anyhow::Result;

use crate::exec::Invocation;
use crate::manager::Manager;
use crate::manifest::grammar::PackageSpec;

pub enum NodeTool {
    Npm,
    Pnpm,
}

pub struct Node {
    pub tool: NodeTool,
}

impl Manager for Node {
    fn id(&self) -> &'static str {
        match self.tool {
            NodeTool::Npm => "npm",
            NodeTool::Pnpm => "pnpm",
        }
    }

    fn install(&self, packages: &[PackageSpec]) -> Result<Vec<Invocation>> {
        if packages.is_empty() {
            return Ok(Vec::new());
        }
        // Built explicitly rather than from `Display`: `Display` renders the
        // manifest's `name version`, which is not what a registry client
        // accepts on its command line.
        let specs = packages.iter().map(registry_arg);
        Ok(vec![match self.tool {
            NodeTool::Npm => Invocation::new("npm").args(["install", "-g"]).args(specs),
            // `pnpm install -g` does not add a global package; `pnpm add -g` does.
            NodeTool::Pnpm => Invocation::new("pnpm").args(["add", "-g"]).args(specs),
        }])
    }

    fn uninstall(&self, names: &[String]) -> Vec<Invocation> {
        if names.is_empty() {
            return Vec::new();
        }
        let names = names.iter().cloned();
        vec![match self.tool {
            NodeTool::Npm => Invocation::new("npm").args(["uninstall", "-g"]).args(names),
            NodeTool::Pnpm => Invocation::new("pnpm").args(["remove", "-g"]).args(names),
        }]
    }

    fn list(&self) -> Invocation {
        match self.tool {
            NodeTool::Npm => Invocation::new("npm").args(["list", "-g", "--depth=0"]),
            NodeTool::Pnpm => Invocation::new("pnpm").args(["list", "-g", "--depth=0"]),
        }
    }

    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        match self.tool {
            NodeTool::Npm => parse_npm(stdout),
            NodeTool::Pnpm => parse_pnpm(stdout),
        }
    }

    fn supports_pinning(&self) -> bool {
        true
    }
}

fn registry_arg(spec: &PackageSpec) -> String {
    match &spec.version {
        Some(version) => format!("{}@{}", spec.name, version),
        None => spec.name.clone(),
    }
}

/// Registry syntax, not manifest syntax: here the last `@` really is the
/// separator, and a leading one is a scope. It lives here so that
/// `PackageSpec::parse` can keep manifest names opaque.
fn split_registry_spec(text: &str) -> PackageSpec {
    match text.rfind('@') {
        Some(at) if at > 0 => {
            let (name, rest) = text.split_at(at);
            let version = rest[1..].trim();
            if version.is_empty() {
                PackageSpec::new(name)
            } else {
                PackageSpec::pinned(name, version)
            }
        }
        _ => PackageSpec::new(text),
    }
}

/// npm prints the global prefix, then a box-drawing tree of `name@version`.
fn parse_npm(stdout: &str) -> Vec<PackageSpec> {
    stdout
        .lines()
        .skip(1) // the global prefix path
        .filter_map(|line| {
            // The byte offset from char_indices keeps multi-byte box-drawing
            // characters from splitting a char boundary.
            let start = line
                .char_indices()
                .find(|(_, character)| character.is_alphanumeric() || *character == '@')
                .map(|(index, _)| index)?;
            let spec = split_registry_spec(&line[start..]);
            if spec.name.is_empty() {
                None
            } else {
                Some(spec)
            }
        })
        .collect()
}

/// pnpm prints a legend and prefix, then `name version` under `dependencies:`.
fn parse_pnpm(stdout: &str) -> Vec<PackageSpec> {
    stdout
        .lines()
        .skip_while(|line| !line.contains("dependencies:"))
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?;
            Some(match fields.next() {
                Some(version) => PackageSpec::pinned(name, version),
                None => PackageSpec::new(name),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NPM_LIST: &str = "\
/usr/lib
├── @anthropic-ai/claude-code@1.0.0
├── corepack@0.24.0
└── typescript@5.3.3
";

    const PNPM_LIST: &str = "\
Legend: production dependency, optional only, dev only

/home/kerim/.local/share/pnpm/global/5

dependencies:
@anthropic-ai/claude-code 1.0.0
typescript 5.3.3
";

    #[test]
    fn npm_scoped_packages_survive_parsing() {
        let npm = Node {
            tool: NodeTool::Npm,
        };
        assert_eq!(
            npm.parse_list(NPM_LIST),
            vec![
                PackageSpec::pinned("@anthropic-ai/claude-code", "1.0.0"),
                PackageSpec::pinned("corepack", "0.24.0"),
                PackageSpec::pinned("typescript", "5.3.3"),
            ]
        );
    }

    #[test]
    fn pnpm_reads_the_dependencies_section() {
        let pnpm = Node {
            tool: NodeTool::Pnpm,
        };
        assert_eq!(
            pnpm.parse_list(PNPM_LIST),
            vec![
                PackageSpec::pinned("@anthropic-ai/claude-code", "1.0.0"),
                PackageSpec::pinned("typescript", "5.3.3"),
            ]
        );
    }

    #[test]
    fn pnpm_adds_rather_than_installs() {
        let pnpm = Node {
            tool: NodeTool::Pnpm,
        };
        assert_eq!(
            pnpm.install(&[PackageSpec::new("typescript")])
                .expect("builds")[0]
                .args,
            vec!["add", "-g", "typescript"]
        );
    }

    #[test]
    fn a_pin_becomes_registry_syntax_not_manifest_syntax() {
        let npm = Node {
            tool: NodeTool::Npm,
        };
        let commands = npm
            .install(&[PackageSpec::pinned("typescript", "5.3.3")])
            .expect("builds");
        assert_eq!(commands[0].args, vec!["install", "-g", "typescript@5.3.3"]);
    }

    #[test]
    fn scoped_pins_keep_the_scope() {
        let npm = Node {
            tool: NodeTool::Npm,
        };
        let commands = npm
            .install(&[PackageSpec::pinned("@scope/pkg", "1.0.0")])
            .expect("builds");
        assert_eq!(commands[0].args, vec!["install", "-g", "@scope/pkg@1.0.0"]);
    }

    #[test]
    fn uninstall_never_carries_a_version() {
        let npm = Node {
            tool: NodeTool::Npm,
        };
        assert_eq!(
            npm.uninstall(&["typescript".to_string()])[0].args,
            vec!["uninstall", "-g", "typescript"]
        );
    }
}
