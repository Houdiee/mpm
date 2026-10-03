use crate::exec::Invocation;
use crate::manifest::grammar::PackageSpec;
pub mod apk;
pub mod apt;
pub mod arch;
pub mod asdf;
pub mod brew;
pub mod cargo;
pub mod composer;
pub mod coursier;
pub mod dart;
pub mod dnf;
pub mod dotnet;
pub mod flatpak;
pub mod gem;
pub mod go;
pub mod krew;
pub mod luarocks;
pub mod mise;
pub mod nix;
pub mod node;
pub mod opam;
pub mod pip;
pub mod pipx;
pub mod pixi;
pub mod pyenv;
pub mod raco;
pub mod rustup;
pub mod uv;
pub mod volta;
pub mod vscode;
pub mod xbps;
pub mod zypper;

/// Every package manager mpm can drive, in the order they are reported.
pub const ALL: &[&str] = &[
    "apk",
    "apt",
    "asdf",
    "brew",
    "bun",
    "cargo",
    "code",
    "code-server",
    "codium",
    "composer",
    "coursier",
    "dnf",
    "dotnet",
    "flatpak",
    "gem",
    "go",
    "krew",
    "luarocks",
    "mise",
    "nix",
    "npm",
    "opam",
    "pacman",
    "pip",
    "pipx",
    "pixi",
    "pnpm",
    "pub",
    "pyenv",
    "raco",
    "rustup",
    "uv",
    "volta",
    "xbps",
    "zypper",
];
/// One package manager, described as argument vectors rather than shell strings.
///
/// These methods build command lines; they never run them. Running is
/// [`Invocation`]'s job and that is where failure is modelled, so returning a
/// plain value here is not a claim that the command succeeds -- only that there
/// is nothing to decide while assembling it. Several return a `Vec` because some
/// managers take one package per call, such as `dotnet tool install`.
pub trait Manager {
    fn id(&self) -> &'static str;
    /// Probed with `which` to decide whether this manager exists here.
    fn binary(&self) -> &'static str {
        self.id()
    }
    fn install_commands(&self, packages: &[PackageSpec]) -> Vec<Invocation>;
    /// Removal receives the *installed* specs, versions included.
    ///
    /// Some managers cannot remove a package by name alone, and mpm already
    /// knows the installed version from [`Manager::parse_list`]. Managers that
    /// need only the name take `.name`.
    fn uninstall_commands(&self, installed: &[PackageSpec]) -> Vec<Invocation>;
    fn list_command(&self) -> Invocation;
    /// Bring packages up to date leaving `pinned` untouched; empty means mpm has
    /// no upgrade it can drive safely here.
    ///
    /// Arch forbids partial upgrades, so its only correct shape is a full `-Syu`
    /// excluding the pins. The rest have no exclusion flag and take the unpinned
    /// names instead.
    fn upgrade_commands(&self, _unpinned: &[String], _pinned: &[String]) -> Vec<Invocation> {
        Vec::new()
    }
    /// May include packages mpm does not manage; the caller keeps only declared ones.
    fn outdated_command(&self) -> Option<Invocation> {
        None
    }
    fn parse_outdated(&self, _stdout: &str) -> Vec<PackageSpec> {
        Vec::new()
    }
    /// Shown as the manager printed it, not parsed: a search result is prose, and
    /// mpm has no reason to reshape it.
    fn search_command(&self, _query: &str) -> Option<Invocation> {
        None
    }
    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec>;
    /// Whether a failing `list` means "nothing installed yet". composer and bun
    /// refuse to list until their global directory has been initialised.
    fn empty_until_first_install(&self) -> bool {
        false
    }
    /// Whether an exact version can be installed *and installed again later* -- a
    /// claim about reproducibility, not about today. Where false, declaring a
    /// version is an error rather than approximated.
    fn supports_pinning(&self) -> bool {
        false
    }
    /// Whether a name can itself select a version, as Homebrew's `node@20` does.
    /// Such a name plus a version is a contradiction, and is rejected.
    fn name_selects_version(&self, _name: &str) -> bool {
        false
    }

    /// Whether several versions of one package can be installed at once.
    ///
    /// Where this is false, declaring a package twice is an error: the second
    /// line could only ever undo the first.
    fn allows_multiple_versions(&self) -> bool {
        false
    }
}
pub fn get(id: &str) -> Option<Box<dyn Manager>> {
    match id {
        "apk" => Some(Box::new(apk::Apk)),
        "apt" => Some(Box::new(apt::Apt)),
        "asdf" => Some(Box::new(asdf::Asdf)),
        "brew" => Some(Box::new(brew::Brew)),
        "bun" => Some(Box::new(node::Node {
            tool: node::NodeTool::Bun,
        })),
        "cargo" => Some(Box::new(cargo::Cargo)),
        "code" => Some(Box::new(vscode::VsCode {
            editor: vscode::Editor::Code,
        })),
        "code-server" => Some(Box::new(vscode::VsCode {
            editor: vscode::Editor::CodeServer,
        })),
        "codium" => Some(Box::new(vscode::VsCode {
            editor: vscode::Editor::Codium,
        })),
        "composer" => Some(Box::new(composer::Composer)),
        "coursier" => Some(Box::new(coursier::Coursier)),
        "dnf" => Some(Box::new(dnf::Dnf)),
        "dotnet" => Some(Box::new(dotnet::Dotnet)),
        "flatpak" => Some(Box::new(flatpak::Flatpak)),
        "gem" => Some(Box::new(gem::Gem)),
        "go" => Some(Box::new(go::Go)),
        "krew" => Some(Box::new(krew::Krew)),
        "luarocks" => Some(Box::new(luarocks::Luarocks)),
        "mise" => Some(Box::new(mise::Mise)),
        "nix" => Some(Box::new(nix::Nix)),
        "npm" => Some(Box::new(node::Node {
            tool: node::NodeTool::Npm,
        })),
        "opam" => Some(Box::new(opam::Opam)),
        "pacman" => Some(Box::new(arch::Arch)),
        "pip" => Some(Box::new(pip::Pip)),
        "pipx" => Some(Box::new(pipx::Pipx)),
        "pixi" => Some(Box::new(pixi::Pixi)),
        "pnpm" => Some(Box::new(node::Node {
            tool: node::NodeTool::Pnpm,
        })),
        "pub" => Some(Box::new(dart::Pub)),
        "pyenv" => Some(Box::new(pyenv::Pyenv)),
        "raco" => Some(Box::new(raco::Raco)),
        "rustup" => Some(Box::new(rustup::Rustup)),
        "uv" => Some(Box::new(uv::Uv)),
        "volta" => Some(Box::new(volta::Volta)),
        "xbps" => Some(Box::new(xbps::Xbps)),
        "zypper" => Some(Box::new(zypper::Zypper)),
        _ => None,
    }
}
pub fn present(manager: &dyn Manager) -> bool {
    which::which(manager.binary()).is_ok()
}
/// One command covering every package, or nothing when there are none.
pub(crate) fn batched<I, S>(command: Invocation, packages: I) -> Vec<Invocation>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut peekable = packages.into_iter().peekable();
    if peekable.peek().is_none() {
        return Vec::new();
    }
    vec![command.args(peekable)]
}
/// Spell a pinned version the way this manager wants it: `ripgrep@14.1.0`,
/// `ripgrep==14.1.0`, `ripgrep:14.1.0`, `ripgrep=14.1.0`.
pub(crate) fn pinned_with(spec: &PackageSpec, separator: &str) -> String {
    match &spec.version {
        Some(version) => format!("{}{separator}{version}", spec.name),
        None => spec.name.clone(),
    }
}
/// Split `name-version`, where names may contain hyphens too
/// (`alpine-baselayout-3.7.2-r1`).
///
/// The version begins at the *last* hyphen followed by a digit. Checked against
/// `xbps-uhelper getpkgname` over every package in a Void image.
pub(crate) fn split_hyphenated(entry: &str) -> Option<PackageSpec> {
    let entry = entry.trim();
    if entry.is_empty() {
        return None;
    }
    let mut boundary = entry.char_indices().filter(|(index, character)| {
        *character == '-' && entry[index + 1..].starts_with(|next: char| next.is_ascii_digit())
    });
    match boundary.next_back() {
        Some((at, _)) => Some(PackageSpec::pinned(&entry[..at], &entry[at + 1..])),
        None => Some(PackageSpec::new(entry)),
    }
}
/// Parse `name version` lines, tolerating extra trailing columns and blanks.
pub(crate) fn parse_two_column(stdout: &str) -> Vec<PackageSpec> {
    stdout
        .lines()
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
    #[test]
    fn every_advertised_manager_is_constructible() {
        for id in ALL {
            let manager =
                get(id).unwrap_or_else(|| panic!("`{id}` is advertised but not registered"));
            assert_eq!(&manager.id(), id);
        }
    }

    #[test]
    fn no_manager_claims_both_pinning_and_coexisting_versions() {
        // The reconciler keys declared and installed state by name, so it holds
        // one version per package. A manager that both pins *and* keeps versions
        // side by side needs that changed first: this is the tripwire for
        // enabling the pair without doing the work.
        for id in ALL {
            let manager = get(id).expect("registered");
            assert!(
                !(manager.supports_pinning() && manager.allows_multiple_versions()),
                "`{id}` claims both; reconcile keys state by name and cannot hold two"
            );
        }
    }

    #[test]
    fn the_roster_is_sorted_unique_and_counted() {
        // A manager registered in `get` but left out of `ALL` is invisible:
        // `mpm managers` never names it and no manifest selects it. Nothing else
        // catches that, so the count is deliberate.
        let mut sorted = ALL.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, ALL, "`ALL` must be sorted and free of duplicates");
        assert_eq!(ALL.len(), 35, "update this count when adding a manager");
    }
    #[test]
    fn the_advertised_list_is_alphabetical() {
        let mut sorted = ALL.to_vec();
        sorted.sort();
        assert_eq!(ALL, sorted.as_slice());
    }
    #[test]
    fn unknown_manager_is_rejected() {
        assert!(get("fisher").is_none());
        assert!(
            get("paru").is_none(),
            "AUR helpers are an installer detail, not a manager"
        );
        assert!(get("").is_none());
    }
    #[test]
    fn a_version_is_accepted_only_where_it_can_be_held() {
        // Two things have to be true to pin: the manager keeps exactly one
        // version of a package, and a package owns its own dependencies. gem
        // breaks the first (versions coexist, so a pin never converges) and
        // pacman the second (excluding a pin from an upgrade is a partial
        // upgrade, which Arch does not support).
        for id in ["cargo", "composer", "dotnet", "npm", "pipx", "pnpm", "bun"] {
            assert!(
                get(id).expect("registered").supports_pinning(),
                "`{id}` should accept versions"
            );
        }
        for id in [
            "apk", "apt", "brew", "dnf", "flatpak", "gem", "luarocks", "pacman", "xbps",
        ] {
            assert!(
                !get(id).expect("registered").supports_pinning(),
                "`{id}` should reject versions"
            );
        }
    }
}
#[cfg(test)]
mod helper_tests {
    use super::*;
    #[test]
    fn nothing_to_install_means_no_command() {
        assert!(batched(Invocation::new("x").arg("add"), Vec::<String>::new()).is_empty());
    }
    #[test]
    fn everything_shares_one_command() {
        let commands = batched(
            Invocation::new("x").arg("add"),
            vec!["a".to_string(), "b".to_string()],
        );
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].args, vec!["add", "a", "b"]);
    }
    #[test]
    fn a_pin_is_spelled_however_the_manager_wants() {
        let spec = PackageSpec::pinned("ripgrep", "14.1.0");
        assert_eq!(pinned_with(&spec, "@"), "ripgrep@14.1.0");
        assert_eq!(pinned_with(&spec, "=="), "ripgrep==14.1.0");
        assert_eq!(pinned_with(&spec, ":"), "ripgrep:14.1.0");
        assert_eq!(pinned_with(&PackageSpec::new("ripgrep"), "@"), "ripgrep");
    }
    #[test]
    fn a_hyphenated_entry_splits_at_the_last_hyphen_before_a_digit() {
        // Every expectation here is what `xbps-uhelper getpkgname` returns.
        for (entry, name, version) in [
            ("ripgrep-15.2.0_1", "ripgrep", "15.2.0_1"),
            (
                "alpine-baselayout-3.7.2-r1",
                "alpine-baselayout",
                "3.7.2-r1",
            ),
            (
                "alpine-baselayout-data-3.7.2-r1",
                "alpine-baselayout-data",
                "3.7.2-r1",
            ),
            ("wine-32bit-9.0_1", "wine-32bit", "9.0_1"),
            ("foo-2-1.0_1", "foo-2", "1.0_1"),
            (
                "python3-setuptools-69.0.3_1",
                "python3-setuptools",
                "69.0.3_1",
            ),
            ("gtk+-2.24.33_1", "gtk+", "2.24.33_1"),
            ("qt5-5.15.11_1", "qt5", "5.15.11_1"),
        ] {
            assert_eq!(
                split_hyphenated(entry),
                Some(PackageSpec::pinned(name, version)),
                "`{entry}` split wrongly"
            );
        }
    }
    #[test]
    fn an_entry_with_no_version_is_all_name() {
        assert_eq!(
            split_hyphenated("ripgrep"),
            Some(PackageSpec::new("ripgrep"))
        );
        assert_eq!(
            split_hyphenated("alpine-keys"),
            Some(PackageSpec::new("alpine-keys"))
        );
    }
    #[test]
    fn blank_lines_are_not_packages() {
        assert_eq!(split_hyphenated(""), None);
        assert_eq!(split_hyphenated("   "), None);
    }
}
#[cfg(test)]
mod container;
