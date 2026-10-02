use crate::exec::Invocation;
use crate::manager::{Manager, batched, pinned_with};
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

    fn install_commands(&self, packages: &[PackageSpec]) -> Vec<Invocation> {
        if packages.is_empty() {
            return Vec::new();
        }
        // Not `Display`, which renders the manifest's `name version` -- not what a
        // registry client accepts on its command line.
        let specs = packages.iter().map(|spec| pinned_with(spec, "@"));
        vec![match self.tool {
            NodeTool::Npm => Invocation::new("npm").args(["install", "-g"]).args(specs),
            // `pnpm install -g` does not add a global package; `pnpm add -g` does.
            NodeTool::Pnpm => Invocation::new("pnpm").args(["add", "-g"]).args(specs),
        }]
    }

    fn uninstall_commands(&self, names: &[String]) -> Vec<Invocation> {
        if names.is_empty() {
            return Vec::new();
        }
        let names = names.iter().cloned();
        vec![match self.tool {
            NodeTool::Npm => Invocation::new("npm").args(["uninstall", "-g"]).args(names),
            NodeTool::Pnpm => Invocation::new("pnpm").args(["remove", "-g"]).args(names),
        }]
    }

    fn upgrade_commands(&self, unpinned: &[String], _pinned: &[String]) -> Vec<Invocation> {
        match self.tool {
            NodeTool::Npm => batched(
                Invocation::new("npm").args(["update", "-g"]),
                unpinned.iter().cloned(),
            ),
            NodeTool::Pnpm => Vec::new(),
        }
    }

    fn outdated_command(&self) -> Option<Invocation> {
        match self.tool {
            NodeTool::Npm => Some(Invocation::new("npm").args(["outdated", "-g"])),
            // `pnpm outdated -g` output has never been checked against a real pnpm.
            NodeTool::Pnpm => None,
        }
    }

    /// `Package Current Wanted Latest Location Depended by`, after a header row.
    fn parse_outdated(&self, stdout: &str) -> Vec<PackageSpec> {
        stdout
            .lines()
            .skip(1)
            .filter_map(|line| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                let [name, _current, _wanted, latest, ..] = fields.as_slice() else {
                    return None;
                };
                Some(PackageSpec::pinned(*name, *latest))
            })
            .collect()
    }

    fn search_command(&self, query: &str) -> Option<Invocation> {
        match self.tool {
            NodeTool::Npm => Some(Invocation::new("npm").arg("search").arg(query)),
            // pnpm has no search of its own.
            NodeTool::Pnpm => None,
        }
    }

    fn list_command(&self) -> Invocation {
        match self.tool {
            NodeTool::Npm => Invocation::new("npm").args(["list", "-g", "--depth=0"]),
            NodeTool::Pnpm => Invocation::new("pnpm").args(["list", "-g", "--depth=0"]),
        }
    }

    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        match self.tool {
            NodeTool::Npm => parse_registry_tree(stdout),
            NodeTool::Pnpm => parse_pnpm(stdout),
        }
    }

    fn supports_pinning(&self) -> bool {
        true
    }

    /// `npm list -g` exits 254 with ENOENT until the global prefix exists.
    fn empty_until_first_install(&self) -> bool {
        true
    }
}

/// Registry syntax, not manifest syntax: the last `@` is the separator and a
/// leading one is a scope. Kept out of `PackageSpec::parse`, where names stay opaque.
pub(crate) fn split_registry_spec(text: &str) -> PackageSpec {
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
pub(crate) fn parse_registry_tree(stdout: &str) -> Vec<PackageSpec> {
    stdout
        .lines()
        .skip(1) // the global prefix path
        .filter_map(|line| {
            // The byte offset from char_indices keeps multi-byte box-drawing
            // characters from splitting a char boundary.
            let start = line
                .char_indices()
                .find(|(_, character)| {
                    character.is_alphanumeric() || matches!(character, '@' | '(')
                })
                .map(|(index, _)| index)?;
            // npm prints `(empty)` under a prefix that holds nothing.
            if line[start..].starts_with('(') {
                return None;
            }
            let spec = split_registry_spec(&line[start..]);
            if spec.name.is_empty() {
                None
            } else {
                Some(spec)
            }
        })
        .collect()
}

/// pnpm prints a legend, the store path, then its dependencies.
///
/// pnpm 12 changed `name version` to npm's tree of `name@version`, so both
/// spellings are accepted. Everything before `dependencies:` is prose and would
/// otherwise parse as packages.
fn parse_pnpm(stdout: &str) -> Vec<PackageSpec> {
    stdout
        .lines()
        .skip_while(|line| !line.contains("dependencies:"))
        .skip(1)
        .filter_map(|line| {
            let start = line
                .char_indices()
                .find(|(_, character)| character.is_alphanumeric() || *character == '@')
                .map(|(index, _)| index)?;
            let entry = line[start..].trim();
            match entry.split_once(char::is_whitespace) {
                Some((name, version)) => Some(PackageSpec::pinned(name, version.trim())),
                None => {
                    let spec = split_registry_spec(entry);
                    if spec.name.is_empty() {
                        None
                    } else {
                        Some(spec)
                    }
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `npm outdated -g` in node:slim.
    const OUTDATED: &str = "\
Package  Current  Wanted  Latest  Location             Depended by
is-odd     2.0.0   3.0.1   3.0.1  node_modules/is-odd  global
npm      11.19.1  12.2.0  12.2.0  node_modules/npm     global
";

    const NPM_LIST: &str = "\
/usr/lib
├── @anthropic-ai/claude-code@1.0.0
├── corepack@0.24.0
└── typescript@5.3.3
";

    // Captured from `pnpm list -g --depth=0` under pnpm 12.
    const PNPM_LIST: &str = "\
Legend: production dependency, optional only, dev only

/root/.pnpm/global/v11 (PRIVATE)
│
│   dependencies:
└── is-odd@3.0.1
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
    fn an_empty_prefix_is_not_a_package() {
        // Captured from a real empty prefix; would otherwise parse as `empty)`.
        let npm = Node {
            tool: NodeTool::Npm,
        };
        assert!(npm.parse_list("/tmp/prefix/lib\n└── (empty)\n").is_empty());
    }

    #[test]
    fn pnpm_saying_it_has_nothing_is_not_a_package() {
        // Captured from `pnpm list -g --depth=0` with an empty global store.
        let pnpm = Node {
            tool: NodeTool::Pnpm,
        };
        assert!(pnpm.parse_list("No global packages found\n").is_empty());
    }

    #[test]
    fn pnpm_reads_the_dependencies_section() {
        let pnpm = Node {
            tool: NodeTool::Pnpm,
        };
        assert_eq!(
            pnpm.parse_list(PNPM_LIST),
            vec![PackageSpec::pinned("is-odd", "3.0.1")]
        );
    }

    #[test]
    fn the_older_pnpm_spelling_still_parses() {
        // Before pnpm 12 the listing was `name version` rather than a tree.
        let pnpm = Node {
            tool: NodeTool::Pnpm,
        };
        let old = "/root/.pnpm\n\ndependencies:\ntypescript 5.3.3\n";
        assert_eq!(
            pnpm.parse_list(old),
            vec![PackageSpec::pinned("typescript", "5.3.3")]
        );
    }

    #[test]
    fn the_legend_and_store_path_are_not_packages() {
        let pnpm = Node {
            tool: NodeTool::Pnpm,
        };
        let names: Vec<String> = pnpm
            .parse_list(PNPM_LIST)
            .into_iter()
            .map(|spec| spec.name)
            .collect();
        assert_eq!(names, vec!["is-odd"]);
    }

    #[test]
    fn pnpm_adds_rather_than_installs() {
        let pnpm = Node {
            tool: NodeTool::Pnpm,
        };
        assert_eq!(
            pnpm.install_commands(&[PackageSpec::new("typescript")])[0].args,
            vec!["add", "-g", "typescript"]
        );
    }

    #[test]
    fn a_pin_becomes_registry_syntax_not_manifest_syntax() {
        let npm = Node {
            tool: NodeTool::Npm,
        };
        let commands = npm.install_commands(&[PackageSpec::pinned("typescript", "5.3.3")]);
        assert_eq!(commands[0].args, vec!["install", "-g", "typescript@5.3.3"]);
    }

    #[test]
    fn scoped_pins_keep_the_scope() {
        let npm = Node {
            tool: NodeTool::Npm,
        };
        let commands = npm.install_commands(&[PackageSpec::pinned("@scope/pkg", "1.0.0")]);
        assert_eq!(commands[0].args, vec!["install", "-g", "@scope/pkg@1.0.0"]);
    }

    #[test]
    fn uninstall_never_carries_a_version() {
        let npm = Node {
            tool: NodeTool::Npm,
        };
        assert_eq!(
            npm.uninstall_commands(&["typescript".to_string()])[0].args,
            vec!["uninstall", "-g", "typescript"]
        );
    }

    #[test]
    fn the_latest_column_is_the_one_that_matters() {
        // Current, Wanted and Latest all sit side by side; picking the wrong
        // column would report a version nobody can install.
        let npm = Node {
            tool: NodeTool::Npm,
        };
        assert_eq!(
            npm.parse_outdated(OUTDATED),
            vec![
                PackageSpec::pinned("is-odd", "3.0.1"),
                PackageSpec::pinned("npm", "12.2.0"),
            ]
        );
    }

    #[test]
    fn pnpm_reports_nothing_because_its_format_is_unchecked() {
        let pnpm = Node {
            tool: NodeTool::Pnpm,
        };
        assert!(pnpm.outdated_command().is_none());
    }

    #[test]
    fn npm_upgrades_the_unpinned_and_pnpm_is_left_alone() {
        let npm = Node {
            tool: NodeTool::Npm,
        };
        assert_eq!(
            npm.upgrade_commands(&["is-odd".to_string()], &["typescript".to_string()])[0].args,
            vec!["update", "-g", "is-odd"]
        );
        let pnpm = Node {
            tool: NodeTool::Pnpm,
        };
        assert!(
            pnpm.upgrade_commands(&["is-odd".to_string()], &[])
                .is_empty()
        );
    }
}
