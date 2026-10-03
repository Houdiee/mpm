pub mod apply;
pub mod edit;
pub mod inherit;
pub mod outdated;
pub mod search;
pub mod status;
pub mod upgrade;

use anyhow::{Context, Result, anyhow, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::mpsc;
use std::thread;

use crate::exec::Invocation;
use crate::manager::{self, Manager};
use crate::manifest::file::ManifestFile;
use crate::manifest::grammar::PackageSpec;
use crate::manifest::{Layer, Layout, hostname};
use crate::reconcile::{self, Reconciliation};
use crate::report;

/// Everything mpm needs to know about this machine.
pub struct Ctx {
    pub layout: Layout,
    pub host: String,
}

impl Ctx {
    pub fn discover() -> Result<Self> {
        let layout = Layout::discover()?;

        let host = hostname()?;
        warn_unmatched_host(&layout, &host);
        Ok(Self { layout, host })
    }
}

/// Warn when a `hosts/` tree exists but nothing in it is for this machine.
///
/// A mistyped directory is otherwise silent: its packages simply never count as
/// declared, and every one of them becomes a removal candidate.
fn warn_unmatched_host(layout: &Layout, host: &str) {
    let Some(known) = layout.known_hosts() else {
        return;
    };
    if known.is_empty() || known.iter().any(|name| name == host) {
        return;
    }
    eprintln!(
        "{} no host layer for `{host}`; `hosts/` has {}",
        report::problem("warning:"),
        known.join(", ")
    );
}

#[derive(Debug, Clone, Default)]
pub struct ApplyOpts {
    /// Show the plan and stop.
    pub dry_run: bool,
    /// Skip the confirmation prompt.
    pub yes: bool,
}

fn require(id: &str) -> Result<Box<dyn Manager>> {
    manager::get(id).ok_or_else(|| anyhow!("unknown package manager `{id}` (see `mpm managers`)"))
}

fn require_present(id: &str) -> Result<Box<dyn Manager>> {
    let found = require(id)?;
    if !manager::present(found.as_ref()) {
        bail!("`{id}` is not installed on this machine");
    }
    Ok(found)
}

fn present_by_id(id: &str) -> bool {
    manager::get(id).is_some_and(|found| manager::present(found.as_ref()))
}

/// Managers to act on: the ones named, else every managed one present here.
///
/// Named managers keep the order given and are de-duplicated, so `cargo,cargo`
/// is not two passes over the same manifest.
fn selected(ctx: &Ctx, requested: &[String]) -> Result<Vec<String>> {
    if requested.is_empty() {
        return Ok(manager::ALL
            .iter()
            .copied()
            .filter(|id| present_by_id(id) && ctx.layout.is_managed(id, &ctx.host))
            .map(str::to_string)
            .collect());
    }
    dedup(requested)
        .into_iter()
        .map(|id| require_present(&id).map(|_| id))
        .collect()
}

/// Keep the first occurrence of each name, in the order given.
pub(crate) fn dedup(names: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    names
        .iter()
        .filter(|name| seen.insert((*name).clone()))
        .cloned()
        .collect()
}

fn installed_map(manager: &dyn Manager) -> Result<BTreeMap<String, PackageSpec>> {
    let stdout = match manager.list_command().capture() {
        Ok(stdout) => stdout,
        // Some managers report a failure rather than an empty list until their
        // first global install; that is not a fault.
        Err(_) if manager.empty_until_first_install() => String::new(),
        Err(error) => return Err(error),
    };
    Ok(manager
        .parse_list(&stdout)
        .into_iter()
        .map(|spec| (spec.name.clone(), spec))
        .collect())
}

/// Reconcile one manager against its manifest.
///
/// Takes the manager as a parameter rather than looking it up, so tests can
/// drive this with a stand-in instead of a real package manager.
fn reconcile_manager(layout: &Layout, host: &str, manager: &dyn Manager) -> Result<Reconciliation> {
    let id = manager.id();
    let resolved = layout.resolve(id, host)?;

    // A version this manager cannot honour is an error, not a warning.
    reconcile::validate(manager, &resolved)?;

    let installed = installed_map(manager)
        .with_context(|| format!("could not list installed packages for `{id}`"))?;

    reconcile::compute(
        id,
        &resolved.declared,
        &installed,
        manager.supports_pinning(),
    )
}

/// Results come back in `ids` order, so output is identical run to run.
fn reconcile_all(ctx: &Ctx, ids: &[String]) -> Vec<(String, Result<Reconciliation>)> {
    let mut handles = Vec::with_capacity(ids.len());
    for id in ids {
        let id = id.clone();
        let layout = ctx.layout.clone();
        let host = ctx.host.clone();
        handles.push(thread::spawn(move || {
            let manager = require(&id)?;
            reconcile_manager(&layout, &host, manager.as_ref())
        }));
    }

    ids.iter()
        .cloned()
        .zip(handles)
        .map(|(id, handle)| {
            let result = handle
                .join()
                .unwrap_or_else(|_| Err(anyhow!("the worker for `{id}` panicked")));
            (id, result)
        })
        .collect()
}

fn nothing_managed(ctx: &Ctx) -> String {
    format!(
        "no package managers are managed yet.\nRun `mpm inherit` to put what is installed under management in {}",
        ctx.layout.root().display()
    )
}

pub fn managers(ctx: &Ctx) -> Result<()> {
    for id in manager::ALL {
        let present = if present_by_id(id) { "found" } else { "-" };
        let managed = if ctx.layout.is_managed(id, &ctx.host) {
            "managed"
        } else {
            "-"
        };
        println!("{id:<8} {present:<7} {managed}");
    }
    println!();
    println!("{}", report::dim(&format!("host: {}", ctx.host)));
    println!(
        "{}",
        report::dim(&format!("manifests: {}", ctx.layout.root().display()))
    );
    Ok(())
}

pub(crate) fn run_visibly(command: &Invocation) -> Result<()> {
    println!("{} {}", report::dim("$"), command.display());
    command.run()
}

#[cfg(test)]
pub(crate) mod fixture {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    pub struct Fake {
        pub installed: &'static str,
        pub pinning: bool,
    }

    impl Manager for Fake {
        fn id(&self) -> &'static str {
            "cargo" // borrow a real id so Layout paths line up
        }
        fn install_commands(&self, packages: &[PackageSpec]) -> Vec<Invocation> {
            vec![Invocation::new("true").args(packages.iter().map(|spec| spec.name.clone()))]
        }
        fn uninstall_commands(&self, installed: &[PackageSpec]) -> Vec<Invocation> {
            vec![Invocation::new("true").args(installed.iter().map(|s| s.name.clone()))]
        }
        fn list_command(&self) -> Invocation {
            Invocation::new("printf").arg("%s").arg(self.installed)
        }
        fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
            crate::manager::parse_two_column(stdout)
        }
        fn supports_pinning(&self) -> bool {
            self.pinning
        }
    }

    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new(tag: &str) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!("mpm-cmd-{tag}-{unique}"));
            fs::create_dir_all(&path).expect("temp dir");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    pub fn seed(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, body).expect("write");
    }

    pub fn run(layout: &Layout, fake: &Fake) -> Result<Reconciliation> {
        reconcile_manager(layout, "testbox", fake)
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;

    fn removed(changes: &Reconciliation) -> Vec<String> {
        changes
            .remove
            .iter()
            .map(|spec| spec.name.clone())
            .collect()
    }

    #[test]
    fn naming_a_manager_twice_is_one_pass() {
        let names = vec!["cargo".to_string(), "npm".to_string(), "cargo".to_string()];
        assert_eq!(dedup(&names), vec!["cargo".to_string(), "npm".to_string()]);
    }

    #[test]
    fn drift_is_computed_from_the_real_read_path() {
        let dir = TempDir::new("drift");
        let layout = Layout::at(&dir.0);
        seed(&layout.common("cargo"), "ripgrep\nvim\n");

        let fake = Fake {
            installed: "vim 9.1\nnano 8.0\n",
            pinning: false,
        };
        let changes = run(&layout, &fake).expect("reconciles");

        assert_eq!(changes.install, vec![PackageSpec::new("ripgrep")]);
        assert_eq!(removed(&changes), vec!["nano"]);
    }

    #[test]
    fn a_declared_package_is_never_a_removal_candidate() {
        let dir = TempDir::new("declared");
        let layout = Layout::at(&dir.0);
        seed(&layout.common("cargo"), "nano\n");

        let fake = Fake {
            installed: "nano 8.0\nbat 0.24\n",
            pinning: false,
        };
        let changes = run(&layout, &fake).expect("reconciles");

        assert_eq!(removed(&changes), vec!["bat"]);
    }

    #[test]
    fn the_host_layer_adds_to_the_shared_one() {
        let dir = TempDir::new("layers");
        let layout = Layout::at(&dir.0);
        seed(&layout.common("cargo"), "vim\n");
        seed(&layout.host("testbox", "cargo"), "tlp\n");

        let fake = Fake {
            installed: "vim 9.1\ntlp 1.6\n",
            pinning: false,
        };
        let changes = run(&layout, &fake).expect("reconciles");

        assert!(!changes.has_work(), "both layers count as declared");
    }

    #[test]
    fn an_unhonourable_version_stops_the_manager_before_any_work() {
        let dir = TempDir::new("badpin");
        let layout = Layout::at(&dir.0);
        seed(&layout.common("cargo"), "ripgrep 14.1.0\n");

        let fake = Fake {
            installed: "",
            pinning: false,
        };
        let error = run(&layout, &fake).expect_err("must refuse");
        assert!(
            error
                .to_string()
                .contains("cannot install a specific version")
        );
    }

    #[test]
    fn a_version_mismatch_becomes_a_repin() {
        let dir = TempDir::new("repin");
        let layout = Layout::at(&dir.0);
        seed(&layout.common("cargo"), "ripgrep 14.1.0\n");

        let fake = Fake {
            installed: "ripgrep 14.0.0\n",
            pinning: true,
        };
        let changes = run(&layout, &fake).expect("reconciles");

        assert_eq!(changes.repin.len(), 1);
        assert_eq!(changes.repin[0].installed, "14.0.0");
    }
}
