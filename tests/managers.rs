//! Drives the real package managers.
//!
//! Every parser in mpm was written against output captured from the actual
//! tool, but a fixture goes stale the moment the tool changes its mind. These
//! tests re-derive that output on demand, so an upstream format change fails
//! here rather than silently mis-parsing someone's machine.
//!
//! They need the managers on `$PATH`, so they are skipped unless
//! `MPM_INTEGRATION` is set:
//!
//!     nix-shell shell-test.nix --run 'MPM_INTEGRATION=1 cargo test'

use std::path::{Path, PathBuf};
use std::process::Command;

/// A manager to exercise: how to isolate it, and what to install.
struct Case {
    manager: &'static str,
    /// Environment that keeps the install inside the test's own directory.
    env: fn(&Path) -> Vec<(String, String)>,
    /// Package to install, which is also the name mpm should record.
    package: &'static str,
    /// Arguments the manager needs to reach the isolated tree.
    extra: fn(&Path) -> Vec<String>,
    /// False when mpm cannot address that tree, so only a direct install can be
    /// observed. luarocks needs `--tree` on every call, which mpm never passes.
    reachable: bool,
    /// Subcommand that installs one package.
    install: &'static [&'static str],
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            manager: "pipx",
            env: |root| {
                vec![
                    ("PIPX_HOME".into(), display(root, "pipx")),
                    ("PIPX_BIN_DIR".into(), display(root, "pipx/bin")),
                ]
            },
            package: "cowsay",
            extra: |_| Vec::new(),
            reachable: true,
            install: &["install", "--force"],
        },
        Case {
            manager: "gem",
            env: |root| {
                let home = display(root, "gems");
                vec![("GEM_HOME".into(), home.clone()), ("GEM_PATH".into(), home)]
            },
            package: "tilt",
            extra: |_| Vec::new(),
            reachable: true,
            install: &["install", "--no-document"],
        },
        Case {
            manager: "composer",
            env: |root| vec![("COMPOSER_HOME".into(), display(root, "composer"))],
            package: "psr/log",
            extra: |_| Vec::new(),
            reachable: true,
            install: &["global", "require", "--no-interaction"],
        },
        Case {
            manager: "npm",
            env: |root| vec![("npm_config_prefix".into(), display(root, "npm"))],
            package: "is-odd",
            extra: |_| Vec::new(),
            reachable: true,
            install: &["install", "-g"],
        },
        Case {
            manager: "luarocks",
            env: |_| Vec::new(),
            package: "inspect",
            extra: |root| vec!["--tree".to_string(), display(root, "lua")],
            reachable: false,
            install: &["install"],
        },
    ]
}

fn display(root: &Path, leaf: &str) -> String {
    root.join(leaf).display().to_string()
}

fn enabled() -> bool {
    std::env::var_os("MPM_INTEGRATION").is_some()
}

fn present(binary: &str) -> bool {
    Command::new(binary).arg("--version").output().is_ok()
}

fn scratch(tag: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("mpm-integration-{tag}-{unique}"));
    std::fs::create_dir_all(&path).expect("scratch directory");
    path
}

fn mpm(case: &Case, root: &Path, config: &Path, args: &[&str]) -> (String, bool) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mpm"));
    command
        .args(args)
        .env("MPM_CONFIG_DIR", config)
        .env("MPM_HOST", "integration")
        .env("MPM_SUDO", "");
    for (key, value) in (case.env)(root) {
        command.env(key, value);
    }
    let output = command.output().expect("mpm runs");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (text, output.status.success())
}

/// Install directly, so the test observes a real install rather than trusting
/// mpm to have made one.
fn install_directly(case: &Case, root: &Path) -> bool {
    let mut command = Command::new(case.manager);
    command
        .args((case.extra)(root))
        .args(case.install)
        .arg(case.package);
    for (key, value) in (case.env)(root) {
        command.env(key, value);
    }
    command
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

#[test]
fn inherit_sees_what_the_manager_really_installed() {
    if !enabled() {
        eprintln!("skipped: set MPM_INTEGRATION to run against real managers");
        return;
    }

    let mut exercised = Vec::new();
    for case in cases() {
        if !present(case.manager) || !case.reachable {
            continue;
        }

        let root = scratch(case.manager);
        let config = root.join("config");

        assert!(
            install_directly(&case, &root),
            "`{}` could not install `{}` into the test tree",
            case.manager,
            case.package
        );

        let (text, ok) = mpm(&case, &root, &config, &[case.manager, "inherit"]);
        assert!(ok, "mpm inherit failed for {}:\n{text}", case.manager);

        let manifest = std::fs::read_to_string(config.join(case.manager))
            .unwrap_or_else(|_| panic!("{} manifest was not written", case.manager));
        assert!(
            manifest.lines().any(|line| line.trim() == case.package),
            "`{}` is missing from the {} manifest mpm wrote:\n{manifest}",
            case.package,
            case.manager
        );

        // Having inherited the real state, mpm must consider the machine clean.
        let (text, ok) = mpm(&case, &root, &config, &[case.manager, "status"]);
        assert!(
            ok,
            "{} still reports drift after inherit:\n{text}",
            case.manager
        );

        let _ = std::fs::remove_dir_all(&root);
        exercised.push(case.manager);
    }

    assert!(
        !exercised.is_empty(),
        "no managers were available to exercise"
    );
    eprintln!("read path verified against: {}", exercised.join(", "));
}

#[test]
fn apply_installs_and_removes_for_real() {
    if !enabled() {
        eprintln!("skipped: set MPM_INTEGRATION to run against real managers");
        return;
    }

    let mut exercised = Vec::new();
    for case in cases() {
        if !present(case.manager) || !case.reachable {
            continue;
        }

        let root = scratch(&format!("apply-{}", case.manager));
        let config = root.join("config");
        std::fs::create_dir_all(&config).expect("config directory");
        std::fs::write(config.join(case.manager), format!("{}\n", case.package)).expect("manifest");

        let (text, ok) = mpm(&case, &root, &config, &[case.manager, "apply", "--yes"]);
        assert!(ok, "mpm apply failed for {}:\n{text}", case.manager);

        let (text, ok) = mpm(&case, &root, &config, &[case.manager, "status"]);
        assert!(
            ok,
            "{} reports drift after apply installed it:\n{text}",
            case.manager
        );

        // Undeclare it and converge again: the package must actually go.
        std::fs::write(config.join(case.manager), "").expect("empty manifest");
        let (text, ok) = mpm(&case, &root, &config, &[case.manager, "apply", "--yes"]);
        assert!(
            ok,
            "mpm apply failed to remove for {}:\n{text}",
            case.manager
        );

        let (text, ok) = mpm(&case, &root, &config, &[case.manager, "status"]);
        assert!(
            ok,
            "{} still reports drift after the removal:\n{text}",
            case.manager
        );

        let _ = std::fs::remove_dir_all(&root);
        exercised.push(case.manager);
    }

    assert!(
        !exercised.is_empty(),
        "no managers were available to exercise"
    );
    eprintln!("full apply verified against: {}", exercised.join(", "));
}
