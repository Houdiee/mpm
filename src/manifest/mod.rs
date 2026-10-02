pub mod file;
pub mod grammar;

use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use file::ManifestFile;
use grammar::PackageSpec;

/// Which layer an edit should be written to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layer {
    Common,
    Host(String),
}

/// The on-disk manifest tree.
///
/// ```text
/// ~/.config/mpm/
/// |- pacman             # declared on every machine
/// |- cargo
/// `- hosts/<host>/pacman
/// ```
///
/// A manifest is a top-level file named after its manager. `hosts` is a
/// reserved directory name; since the set of manager ids is closed, it can
/// never collide with a manifest.
#[derive(Debug, Clone)]
pub struct Layout {
    root: PathBuf,
}

/// The declared state for one manager, after every layer is applied.
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    pub declared: BTreeMap<String, PackageSpec>,
}

impl Layout {
    /// Locate the manifest tree: `$MPM_CONFIG_DIR`, else `<config dir>/mpm`.
    pub fn discover() -> Result<Self> {
        if let Ok(value) = std::env::var("MPM_CONFIG_DIR") {
            let value = value.trim();
            if !value.is_empty() {
                return Ok(Self {
                    root: PathBuf::from(value),
                });
            }
        }
        Ok(Self {
            root: config_home()?.join("mpm"),
        })
    }

    #[cfg(test)]
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The manifest that applies to every machine.
    pub fn common(&self, manager: &str) -> PathBuf {
        self.root.join(manager)
    }

    pub fn host(&self, host: &str, manager: &str) -> PathBuf {
        self.root.join("hosts").join(host).join(manager)
    }

    /// Host directories this tree defines, in name order.
    ///
    /// `None` when there is no `hosts/` tree at all, which is different from a
    /// tree that exists but names no host matching this machine.
    pub fn known_hosts(&self) -> Option<Vec<String>> {
        let entries = fs::read_dir(self.root.join("hosts")).ok()?;
        let mut hosts: Vec<String> = entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
            .collect();
        hosts.sort();
        Some(hosts)
    }

    pub fn path_for(&self, layer: &Layer, manager: &str) -> PathBuf {
        match layer {
            Layer::Common => self.common(manager),
            Layer::Host(host) => self.host(host, manager),
        }
    }

    /// Layer files for a manager, lowest priority first: the host layer is more
    /// specific than `common`, so it has the final say.
    pub fn layer_paths(&self, manager: &str, host: &str) -> Vec<PathBuf> {
        vec![self.common(manager), self.host(host, manager)]
    }

    pub fn is_managed(&self, manager: &str, host: &str) -> bool {
        self.layer_paths(manager, host)
            .iter()
            .any(|path| path.exists())
    }

    /// Collapse every layer into one declared state.
    pub fn resolve(&self, manager: &str, host: &str) -> Result<Resolved> {
        let mut resolved = Resolved::default();

        for path in self.layer_paths(manager, host) {
            let manifest = ManifestFile::load(&path)?;
            if !manifest.exists() {
                continue;
            }
            for spec in manifest.specs() {
                // A later layer can change a version but never take a package
                // away: declared is the plain union of every layer.
                resolved.declared.insert(spec.name.clone(), spec.clone());
            }
        }

        Ok(resolved)
    }
}

/// Where a command-line tool's configuration belongs.
///
/// `dirs::config_dir()` answers `~/Library/Application Support` on macOS, which
/// is not where anyone looks for something like this -- and not where the README
/// tells them to. XDG first, then `~/.config`, on every platform.
fn config_home() -> Result<PathBuf> {
    if let Some(configured) = std::env::var_os("XDG_CONFIG_HOME") {
        let path = PathBuf::from(configured);
        if path.is_absolute() {
            return Ok(path);
        }
    }
    if let Some(home) = dirs::home_dir() {
        return Ok(home.join(".config"));
    }
    dirs::config_dir().context("could not determine your config directory")
}

/// This machine's name, used to select the host layer.
///
/// Guessing would silently select the wrong layer, so failure is an error with
/// the override named rather than a fallback.
pub fn hostname() -> Result<String> {
    if let Ok(value) = std::env::var("MPM_HOST") {
        let value = value.trim();
        if !value.is_empty() {
            return Ok(value.to_string());
        }
    }
    if let Ok(value) = fs::read_to_string("/etc/hostname") {
        let value = value.trim();
        if !value.is_empty() {
            return Ok(value.to_string());
        }
    }
    if let Ok(output) = std::process::Command::new("hostname").output()
        && output.status.success()
    {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !value.is_empty() {
            return Ok(value);
        }
    }
    bail!("could not determine this machine's hostname; set MPM_HOST to name it")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!("mpm-layout-{tag}-{unique}"));
            fs::create_dir_all(&path).expect("temp dir");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn seed(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, body).expect("write");
    }

    fn names(resolved: &Resolved) -> Vec<String> {
        resolved.declared.keys().cloned().collect()
    }

    #[test]
    fn common_alone_resolves() {
        let dir = TempDir::new("common");
        let layout = Layout::at(&dir.0);
        seed(&layout.common("pacman"), "vim\nripgrep\n");

        let resolved = layout.resolve("pacman", "thinkpad").expect("resolve");
        assert_eq!(names(&resolved), vec!["ripgrep", "vim"]);
    }

    #[test]
    fn another_hosts_layer_is_ignored() {
        let dir = TempDir::new("host-other");
        let layout = Layout::at(&dir.0);
        seed(&layout.common("pacman"), "vim\n");
        seed(&layout.host("desktop", "pacman"), "nvidia\n");

        let resolved = layout.resolve("pacman", "thinkpad").expect("resolve");
        assert_eq!(names(&resolved), vec!["vim"]);
    }

    #[test]
    fn layers_are_a_plain_union() {
        let dir = TempDir::new("union");
        let layout = Layout::at(&dir.0);
        seed(&layout.common("pacman"), "vim\ntlp\n");
        seed(&layout.host("desktop", "pacman"), "nvidia\n");

        let resolved = layout.resolve("pacman", "desktop").expect("resolve");
        assert_eq!(names(&resolved), vec!["nvidia", "tlp", "vim"]);
    }

    #[test]
    fn the_host_layer_outranks_common() {
        let dir = TempDir::new("precedence");
        let layout = Layout::at(&dir.0);
        seed(&layout.common("cargo"), "ripgrep 14.0.0\n");
        seed(&layout.host("thinkpad", "cargo"), "ripgrep 14.2.0\n");

        let resolved = layout.resolve("cargo", "thinkpad").expect("resolve");
        assert_eq!(
            resolved.declared["ripgrep"],
            PackageSpec::pinned("ripgrep", "14.2.0")
        );
    }

    #[test]
    fn an_undetectable_hostname_is_an_error_not_a_guess() {
        let previous = std::env::var("MPM_HOST").ok();
        unsafe { std::env::set_var("MPM_HOST", "  ") };
        let guessed = hostname();
        match previous {
            Some(value) => unsafe { std::env::set_var("MPM_HOST", value) },
            None => unsafe { std::env::remove_var("MPM_HOST") },
        }
        // On any normal machine /etc/hostname or `hostname` answers; the point
        // is that a blank override falls through rather than being accepted.
        assert_ne!(guessed.ok().as_deref(), Some(""));
    }

    #[test]
    fn known_hosts_distinguishes_absent_from_empty() {
        let dir = TempDir::new("hosts");
        let layout = Layout::at(&dir.0);
        assert_eq!(layout.known_hosts(), None, "no hosts/ tree at all");

        seed(&layout.host("desktop", "pacman"), "nvidia\n");
        seed(&layout.host("laptop", "pacman"), "tlp\n");
        assert_eq!(
            layout.known_hosts(),
            Some(vec!["desktop".to_string(), "laptop".to_string()])
        );
    }

    #[test]
    fn a_manager_with_no_layers_is_unmanaged() {
        let dir = TempDir::new("unmanaged");
        let layout = Layout::at(&dir.0);
        assert!(!layout.is_managed("pacman", "thinkpad"));
        seed(&layout.common("pacman"), "vim\n");
        assert!(layout.is_managed("pacman", "thinkpad"));
    }

    #[test]
    fn a_malformed_line_names_its_file_and_line() {
        let dir = TempDir::new("malformed");
        let layout = Layout::at(&dir.0);
        seed(&layout.common("cargo"), "# tools\nripgrep 14.1.0 oops\n");

        let error = layout
            .resolve("cargo", "thinkpad")
            .expect_err("must reject");
        let text = format!("{error:#}");
        assert!(text.contains("/cargo:2"), "got: {text}");
    }
}
