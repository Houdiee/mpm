use crate::exec::Invocation;
use crate::manager::{Manager, batched};
use crate::manifest::grammar::PackageSpec;

pub struct Zypper;

impl Manager for Zypper {
    fn id(&self) -> &'static str {
        "zypper"
    }

    fn install_commands(&self, packages: &[PackageSpec]) -> Vec<Invocation> {
        batched(
            Invocation::new("zypper")
                .with_root()
                .args(["--non-interactive", "install"]),
            packages.iter().map(|spec| spec.name.clone()),
        )
    }

    fn uninstall_commands(&self, installed: &[PackageSpec]) -> Vec<Invocation> {
        // `remove`, not `remove --clean-deps`: pulling orphaned dependencies out is
        // not something a package-list sync should decide on its own.
        batched(
            Invocation::new("zypper")
                .with_root()
                .args(["--non-interactive", "remove"]),
            installed.iter().map(|spec| spec.name.clone()),
        )
    }

    fn upgrade_commands(&self, unpinned: &[String], _pinned: &[String]) -> Vec<Invocation> {
        batched(
            Invocation::new("zypper")
                .with_root()
                .args(["--non-interactive", "update"]),
            unpinned.iter().cloned(),
        )
    }

    fn search_command(&self, query: &str) -> Option<Invocation> {
        Some(Invocation::new("zypper").args(["search"]).arg(query))
    }

    /// `--details` adds the version column; `--type package` keeps patterns and
    /// products out.
    fn list_command(&self) -> Invocation {
        Invocation::new("zypper").args([
            "--quiet",
            "search",
            "--installed-only",
            "--details",
            "--type",
            "package",
        ])
    }

    /// A pipe-delimited table whose status column separates `i+`, installed
    /// because it was asked for, from `i`, installed as a dependency.
    ///
    /// That column is the whole reason this listing is used rather than
    /// `rpm -qa`: without it every dependency on the system reads as drift.
    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        stdout
            .lines()
            .filter_map(|line| {
                let mut fields = line.split('|').map(str::trim);
                if fields.next()? != "i+" {
                    return None;
                }
                let name = fields.next()?;
                let _type = fields.next()?;
                let version = fields.next()?;
                if name.is_empty() || version.is_empty() {
                    return None;
                }
                Some(PackageSpec::pinned(name, version))
            })
            .collect()
    }

    /// openSUSE drops superseded builds from its repositories, so a version
    /// declared today stops being installable.
    fn supports_pinning(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `zypper --quiet search --installed-only --details --type
    // package` in opensuse/tumbleweed.
    const LIST: &str = "\n\
S  | Name                | Type    | Version                        | Arch   | Repository
---+---------------------+---------+--------------------------------+--------+-------------------------
i+ | aaa_base            | package | 84.87+git20260924.144354a1-1.1 | x86_64 | openSUSE-Tumbleweed-Oss
i+ | bash                | package | 5.3.20-9.1                     | x86_64 | openSUSE-Tumbleweed-Oss
i  | bash-sh             | package | 5.3.20-9.1                     | noarch | openSUSE-Tumbleweed-Oss
i  | boost-license1_92_0 | package | 1.92.0-1.1                     | noarch | openSUSE-Tumbleweed-Oss
";

    #[test]
    fn only_explicitly_installed_packages_are_declared() {
        let names: Vec<String> = Zypper
            .parse_list(LIST)
            .into_iter()
            .map(|spec| spec.name)
            .collect();
        assert_eq!(names, vec!["aaa_base", "bash"]);
    }

    #[test]
    fn a_dependency_is_not_a_removal_candidate() {
        // `bash-sh` is marked `i`, not `i+`. Declaring it a removal would have
        // `apply` tear out packages the system depends on.
        let names: Vec<String> = Zypper
            .parse_list(LIST)
            .into_iter()
            .map(|spec| spec.name)
            .collect();
        assert!(!names.contains(&"bash-sh".to_string()));
    }

    #[test]
    fn the_version_column_is_kept_whole() {
        let parsed = Zypper.parse_list(LIST);
        assert_eq!(
            parsed[0].version.as_deref(),
            Some("84.87+git20260924.144354a1-1.1")
        );
    }

    #[test]
    fn the_header_and_rule_are_not_packages() {
        let parsed = Zypper.parse_list(LIST);
        assert!(parsed.iter().all(|spec| spec.name != "Name"));
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn changes_need_root() {
        assert!(Zypper.install_commands(&[PackageSpec::new("ripgrep")])[0].needs_root);
        assert!(Zypper.uninstall_commands(&[PackageSpec::new("ripgrep")])[0].needs_root);
    }

    #[test]
    fn searching_does_not() {
        assert!(!Zypper.search_command("rg").expect("search").needs_root);
    }
}
