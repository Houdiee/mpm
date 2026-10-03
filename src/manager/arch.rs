use crate::exec::Invocation;
use crate::manager::{Manager, batched, parse_two_column};
use crate::manifest::grammar::PackageSpec;

pub struct Arch;

/// pacman, paru and yay read the same local database, so there is only ever one
/// Arch package set. An AUR helper is an installation detail, not a separate
/// manager: as separate managers each would see the other's packages as drift.
enum Installer {
    /// `paru` or `yay`. They wrap pacman, handle repo and AUR packages alike,
    /// and call `sudo` themselves -- so they must not be run through it.
    Helper(&'static str),
    Pacman,
}

impl Installer {
    fn detect() -> Self {
        for helper in ["paru", "yay"] {
            if which::which(helper).is_ok() {
                return Installer::Helper(helper);
            }
        }
        Installer::Pacman
    }

    fn command(&self) -> Invocation {
        match self {
            Installer::Helper(helper) => Invocation::new(helper),
            Installer::Pacman => Invocation::new("pacman").with_root(),
        }
    }
}

impl Manager for Arch {
    fn id(&self) -> &'static str {
        "pacman"
    }

    fn install_commands(&self, packages: &[PackageSpec]) -> Vec<Invocation> {
        // `--needed` keeps an already-satisfied package from being reinstalled
        // when a partly-applied run is retried.
        batched(
            Installer::detect().command().args(["-S", "--needed"]),
            packages.iter().map(|spec| spec.name.clone()),
        )
    }

    fn uninstall_commands(&self, installed: &[PackageSpec]) -> Vec<Invocation> {
        // -Rs removes now-orphaned dependencies but keeps configuration. -Rns
        // would delete config too, which a package-list sync must not decide.
        batched(
            Invocation::new("pacman").with_root().arg("-Rs"),
            installed.iter().map(|spec| spec.name.clone()),
        )
    }

    fn upgrade_commands(&self, _unpinned: &[String], pinned: &[String]) -> Vec<Invocation> {
        // Arch supports no partial upgrade: `-Syu` with the pins ignored is the
        // only safe shape. `--ignore` may be repeated.
        let mut command = Installer::detect().command().args(["-Syu", "--noconfirm"]);
        for name in pinned {
            command = command.arg("--ignore").arg(name.as_str());
        }
        vec![command]
    }

    fn outdated_command(&self) -> Option<Invocation> {
        Some(Invocation::new("pacman").arg("-Qu"))
    }

    /// `name installed -> available`
    fn parse_outdated(&self, stdout: &str) -> Vec<PackageSpec> {
        stdout
            .lines()
            .filter_map(|line| {
                let (name, rest) = line.split_once(char::is_whitespace)?;
                let available = rest.split("-> ").nth(1)?.split_whitespace().next()?;
                Some(PackageSpec::pinned(name.trim(), available))
            })
            .collect()
    }

    fn search_command(&self, query: &str) -> Option<Invocation> {
        Some(Invocation::new("pacman").args(["-Ss"]).arg(query))
    }

    fn list_command(&self) -> Invocation {
        // -Qe lists explicitly-installed packages only, so dependencies pulled
        // in automatically never show up as undeclared drift.
        Invocation::new("pacman").arg("-Qe")
    }

    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        parse_two_column(stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `pacman -Qu` in archlinux:latest.
    const OUTDATED: &str = "\
coreutils 9.11-2 -> 9.12-2
glibc 2.44+r24+g16be1518495f-1 -> 2.44+r50+g1848099f063e-1
";

    const QE: &str = "\
git 2.46.0-1
linux 6.10.6.arch1-1
ripgrep 14.1.0-1
";

    #[test]
    fn upgradable_packages_are_parsed_with_the_version_available() {
        assert_eq!(
            Arch.parse_outdated(OUTDATED),
            vec![
                PackageSpec::pinned("coreutils", "9.12-2"),
                PackageSpec::pinned("glibc", "2.44+r50+g1848099f063e-1"),
            ]
        );
    }

    #[test]
    fn an_upgrade_excludes_every_pin_and_never_goes_partial() {
        // Pins are excluded from a full -Syu, not the other way round.
        let commands = Arch.upgrade_commands(
            &["bat".to_string()],
            &["ripgrep".to_string(), "linux".to_string()],
        );
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0].args,
            vec![
                "-Syu",
                "--noconfirm",
                "--ignore",
                "ripgrep",
                "--ignore",
                "linux"
            ]
        );
        assert!(
            !commands[0].args.iter().any(|arg| arg == "bat"),
            "no package is named"
        );
    }

    #[test]
    fn explicit_packages_are_parsed_with_versions() {
        assert_eq!(
            Arch.parse_list(QE),
            vec![
                PackageSpec::pinned("git", "2.46.0-1"),
                PackageSpec::pinned("linux", "6.10.6.arch1-1"),
                PackageSpec::pinned("ripgrep", "14.1.0-1"),
            ]
        );
    }

    #[test]
    fn listing_always_uses_pacman() {
        let list = Arch.list_command();
        assert_eq!(list.program, "pacman");
        assert!(!list.needs_root, "a query needs no privileges");
    }

    #[test]
    fn removal_keeps_configuration() {
        let commands = Arch.uninstall_commands(&[PackageSpec::new("vim")]);
        assert_eq!(commands[0].args[0], "-Rs");
        assert!(commands[0].needs_root);
    }

    #[test]
    fn unpinned_packages_share_one_command() {
        let commands = Arch.install_commands(&[PackageSpec::new("vim"), PackageSpec::new("git")]);
        assert_eq!(commands.len(), 1);
        assert_eq!(&commands[0].args[..2], &["-S", "--needed"]);
    }
}

/// Checks the parser against a real `pacman` rather than a fixture that can go
/// stale. The local database is plain text, so one explicit package and one
/// dependency can be laid down directly.
///
/// Runs only under `nix-shell shell-test.nix` with `MPM_INTEGRATION=1`.
#[cfg(test)]
mod real_pacman {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn seed(root: &std::path::Path, name: &str, version: &str, dependency: bool) {
        let entry = root.join("local").join(format!("{name}-{version}"));
        fs::create_dir_all(&entry).expect("db entry");
        let mut desc = format!("%NAME%\n{name}\n\n%VERSION%\n{version}\n");
        if dependency {
            desc.push_str("\n%REASON%\n1\n");
        }
        fs::write(entry.join("desc"), desc).expect("desc");
    }

    #[test]
    fn a_real_pacman_reports_only_explicit_packages() {
        if std::env::var_os("MPM_INTEGRATION").is_none() {
            eprintln!("skipped: set MPM_INTEGRATION and use shell-test.nix");
            return;
        }
        if Command::new("pacman").arg("--version").output().is_err() {
            eprintln!("skipped: pacman is not on PATH");
            return;
        }

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("mpm-pacman-{unique}"));
        let db = root.join("db");
        fs::create_dir_all(db.join("local")).expect("root");
        // The version marker lives inside `local/`, not at the dbpath root.
        fs::write(db.join("local/ALPM_DB_VERSION"), "9\n").expect("db version");
        seed(&db, "ripgrep", "14.1.1-1", false);
        seed(&db, "pcre2", "10.44-1", true);

        let config = root.join("pacman.conf");
        fs::write(&config, "[options]\nArchitecture = x86_64\n").expect("config");

        let run = |query: &str| {
            let output = Command::new("pacman")
                .arg("--config")
                .arg(&config)
                .arg("--dbpath")
                .arg(&db)
                .arg(query)
                .output()
                .expect("pacman runs");
            assert!(
                output.status.success(),
                "`pacman {query}` failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).into_owned()
        };

        // -Q lists everything, -Qe only what was asked for.
        let everything = Arch.parse_list(&run("-Q"));
        let explicit = Arch.parse_list(&run("-Qe"));

        let names = |specs: &[PackageSpec]| {
            specs
                .iter()
                .map(|spec| spec.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&everything), vec!["pcre2", "ripgrep"]);
        assert_eq!(
            names(&explicit),
            vec!["ripgrep"],
            "-Qe must leave out the dependency"
        );
        assert_eq!(explicit[0].version.as_deref(), Some("14.1.1-1"));

        let _ = fs::remove_dir_all(&root);
    }
}
