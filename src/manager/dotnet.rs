use anyhow::Result;

use crate::manager::Manager;
use crate::exec::Invocation;
use crate::manifest::grammar::PackageSpec;

pub struct Dotnet;

impl Manager for Dotnet {
    fn id(&self) -> &'static str {
        "dotnet"
    }

    // One tool per call: the synopsis is `dotnet tool install <PACKAGE_NAME> -g`,
    // singular. .NET 10 added `name@version` but still takes one tool at a time.
    fn install(&self, packages: &[PackageSpec]) -> Result<Vec<Invocation>> {
        Ok(packages
            .iter()
            .map(|spec| {
                let command =
                    Invocation::new("dotnet").args(["tool", "install", "-g"]).arg(spec.name.as_str());
                match &spec.version {
                    Some(version) => command.arg("--version").arg(version.as_str()),
                    None => command,
                }
            })
            .collect())
    }

    fn uninstall(&self, names: &[String]) -> Vec<Invocation> {
        names
            .iter()
            .map(|name| Invocation::new("dotnet").args(["tool", "uninstall", "-g"]).arg(name.as_str()))
            .collect()
    }

    fn list(&self) -> Invocation {
        Invocation::new("dotnet").args(["tool", "list", "-g"])
    }

    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        // Anchoring on the row of dashes survives header wording and locale.
        let lines: Vec<&str> = stdout.lines().collect();
        let body = match lines.iter().position(|line| line.trim_start().starts_with("---")) {
            Some(index) => &lines[index + 1..],
            None => lines.get(2..).unwrap_or(&[]),
        };

        body.iter()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let name = parts.next()?;
                Some(match parts.next() {
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

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "\
Package Id          Version      Commands
-------------------------------------------
dotnet-ef           8.0.8        dotnet-ef
csharpier           0.29.0       dotnet-csharpier
";

    #[test]
    fn header_and_separator_are_skipped() {
        assert_eq!(
            Dotnet.parse_list(LIST),
            vec![PackageSpec::pinned("dotnet-ef", "8.0.8"), PackageSpec::pinned("csharpier", "0.29.0")]
        );
    }

    #[test]
    fn missing_separator_falls_back_to_skipping_two_lines() {
        let out = "Package Id Version Commands\ndotnet-ef 8.0.8 dotnet-ef\n";
        assert!(Dotnet.parse_list(out).is_empty());
    }

    #[test]
    fn each_tool_gets_its_own_command() {
        let cmds = Dotnet.uninstall(&["dotnet-ef".to_string(), "csharpier".to_string()]);
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].args, vec!["tool", "uninstall", "-g", "dotnet-ef"]);
        assert_eq!(cmds[1].args, vec!["tool", "uninstall", "-g", "csharpier"]);
    }

    #[test]
    fn pins_use_the_version_flag() {
        let cmds = Dotnet.install(&[PackageSpec::pinned("dotnet-ef", "8.0.8")]).expect("builds");
        assert_eq!(cmds[0].args, vec!["tool", "install", "-g", "dotnet-ef", "--version", "8.0.8"]);
    }
}
