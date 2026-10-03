use crate::exec::Invocation;
use crate::manager::{Manager, batched};
use crate::manifest::grammar::PackageSpec;

pub struct Rustup;

impl Manager for Rustup {
    fn id(&self) -> &'static str {
        "rustup"
    }

    fn install_commands(&self, packages: &[PackageSpec]) -> Vec<Invocation> {
        batched(
            Invocation::new("rustup").args(["toolchain", "install"]),
            packages.iter().map(|spec| spec.name.clone()),
        )
    }

    fn uninstall_commands(&self, installed: &[PackageSpec]) -> Vec<Invocation> {
        batched(
            Invocation::new("rustup").args(["toolchain", "uninstall"]),
            installed.iter().map(|spec| spec.name.clone()),
        )
    }

    fn upgrade_commands(&self, _unpinned: &[String], _pinned: &[String]) -> Vec<Invocation> {
        // `rustup update` moves every channel at once and cannot be told to
        // leave one alone, so there is no upgrade mpm can drive safely.
        Vec::new()
    }

    /// `rustup show`, not `rustup toolchain list`, because it reports the host
    /// triple alongside the toolchains -- and the triple is what has to come off.
    ///
    /// `toolchain list` prints `1.80.0-x86_64-unknown-linux-gnu`, but install and
    /// uninstall both accept plain `1.80.0`. Keeping the long spelling meant a
    /// manifest saying `1.80.0` read as a different package from the toolchain
    /// installed, so mpm would install it (a no-op) and *uninstall* the real one
    /// in the same run. Stripping the host triple makes the manifest hold the
    /// spelling a person would write, and the one that round-trips.
    fn list_command(&self) -> Invocation {
        Invocation::new("rustup").arg("show")
    }

    fn parse_list(&self, stdout: &str) -> Vec<PackageSpec> {
        let host = stdout
            .lines()
            .find_map(|line| line.trim().strip_prefix("Default host:"))
            .map(str::trim);

        stdout
            .lines()
            // The toolchains sit under their own heading and its rule.
            .skip_while(|line| line.trim() != "installed toolchains")
            .skip(2)
            .take_while(|line| !line.trim().is_empty())
            .filter_map(|line| {
                let name = line.split_whitespace().next()?;
                // A toolchain built for another target keeps its triple: it
                // really is a different toolchain.
                let short = host
                    .and_then(|host| name.strip_suffix(host))
                    .map(|stem| stem.trim_end_matches('-'))
                    .unwrap_or(name);
                (!short.is_empty()).then(|| PackageSpec::new(short))
            })
            .collect()
    }

    /// A toolchain name *is* its version -- `1.80.0`, `stable`, `nightly-2026-01-01`
    /// -- so a separate version column would be a contradiction, and declaring
    /// one is rejected the way Homebrew's `node@20` is.
    fn name_selects_version(&self, _name: &str) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `rustup show` in rust:slim after installing 1.80.0 and
    // nightly.
    const LIST: &str = "\
Default host: x86_64-unknown-linux-gnu
rustup home:  /usr/local/rustup

installed toolchains
--------------------
nightly-x86_64-unknown-linux-gnu
1.80.0-x86_64-unknown-linux-gnu
1.99.0-x86_64-unknown-linux-gnu (active, default)

active toolchain
----------------
name: 1.99.0-x86_64-unknown-linux-gnu
active because: it's the default toolchain
installed targets:
  x86_64-unknown-linux-gnu
";

    #[test]
    fn the_host_triple_comes_off() {
        assert_eq!(
            Rustup.parse_list(LIST),
            vec![
                PackageSpec::new("nightly"),
                PackageSpec::new("1.80.0"),
                PackageSpec::new("1.99.0"),
            ]
        );
    }

    #[test]
    fn nothing_outside_the_toolchain_section_is_a_toolchain() {
        let names: Vec<String> = Rustup
            .parse_list(LIST)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names.len(), 3);
        for noise in ["Default", "rustup", "name:", "x86_64-unknown-linux-gnu"] {
            assert!(!names.contains(&noise.to_string()), "parsed `{noise}`");
        }
    }

    #[test]
    fn a_toolchain_for_another_target_keeps_its_triple() {
        // Stripping only the *host* triple: a cross toolchain is genuinely a
        // different toolchain and must not collapse onto the native one.
        let cross = "\
Default host: x86_64-unknown-linux-gnu

installed toolchains
--------------------
1.80.0-x86_64-unknown-linux-gnu
1.80.0-aarch64-apple-darwin

";
        assert_eq!(
            Rustup.parse_list(cross),
            vec![
                PackageSpec::new("1.80.0"),
                PackageSpec::new("1.80.0-aarch64-apple-darwin"),
            ]
        );
    }
}
