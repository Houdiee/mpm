//! Parses what the package managers print, right now, in their own containers.
//!
//! A committed fixture cannot notice that a manager changed its output format in
//! last week's release, which is the one thing worth catching. Each test starts
//! the manager's own image, installs a known package, runs the real list command
//! and feeds the output straight into mpm's parser. Nothing is written to disk.
//!
//! Needs a container runtime, so these skip unless `MPM_CONTAINERS` is set:
//!
//!     MPM_CONTAINERS=1 cargo test container          # every manager
//!     MPM_CONTAINERS=1 cargo test container::apt     # just one
//!
//! `flatpak` is absent: installing anything needs a privileged container.

use super::get;
use crate::manifest::grammar::PackageSpec;
use std::process::Command;

/// How to make one manager produce a real package listing.
struct Recipe {
    /// Query to pass to the manager's own search, when it has one.
    search: &'static str,
    image: &'static str,
    /// Refresh repositories, or `":"` when nothing is needed.
    setup: &'static str,
    install: &'static str,
    list: &'static str,
    /// The package `install` adds, which must appear in the parsed output.
    planted: &'static str,
    /// Leave something behind the registry, then report it. Both empty when this
    /// manager has no outdated query. Replaces `setup` for that run, because
    /// refreshing and upgrading would leave nothing behind.
    stale: &'static str,
    outdated: &'static str,
    /// Package `outdated` must name. Empty where being behind depends on how
    /// stale the image is rather than on anything the recipe did, in which case
    /// only the shape of the output is checked.
    behind: &'static str,
}

fn runtime() -> Option<&'static str> {
    ["docker", "podman"].into_iter().find(|engine| {
        Command::new(engine)
            .arg("info")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    })
}

/// Install the planted package in a throwaway container and parse what the
/// manager says is installed.
fn verify(id: &str, recipe: Recipe) {
    if std::env::var_os("MPM_CONTAINERS").is_none() {
        eprintln!("skipped {id}: set MPM_CONTAINERS=1 to run against containers");
        return;
    }
    let Some(engine) = runtime() else {
        eprintln!("skipped {id}: no usable container runtime");
        return;
    };

    // `sh -c` rather than `sh -lc`: a login shell resets PATH and loses the
    // toolchain directories these images set through ENV.
    let script = format!(
        "{{ {setup} ; }} >/dev/null 2>&1 || true\n{install} >/dev/null 2>&1\n{list}",
        setup = recipe.setup,
        install = recipe.install,
        list = recipe.list,
    );
    let output = Command::new(engine)
        .args(["run", "--rm", recipe.image, "sh", "-c", &script])
        .output()
        .unwrap_or_else(|error| panic!("{id}: could not start {}: {error}", recipe.image));

    let listing = String::from_utf8_lossy(&output.stdout);
    assert!(
        !listing.trim().is_empty(),
        "{id}: `{}` printed nothing in {}\nstderr:\n{}",
        recipe.list,
        recipe.image,
        String::from_utf8_lossy(&output.stderr)
    );

    let manager = get(id).unwrap_or_else(|| panic!("`{id}` is not registered"));
    let parsed = manager.parse_list(&listing);

    assert!(
        parsed.iter().any(|spec| spec.name == recipe.planted),
        "{id}: mpm did not find `{}` in what {} printed:\n{listing}\nparsed: {:?}",
        recipe.planted,
        recipe.image,
        parsed
            .iter()
            .map(|spec| spec.name.as_str())
            .collect::<Vec<_>>()
    );

    // A drifting parser picks up decoration -- a tree glyph, a status column, an
    // `(empty)` note -- and anything parsed out must survive a manifest round trip.
    for spec in &parsed {
        let round_tripped = PackageSpec::parse(&spec.to_string());
        assert!(
            round_tripped.is_ok(),
            "{id}: parsed `{spec}`, which the manifest grammar rejects"
        );
    }

    eprintln!(
        "{id}: {} packages parsed from {}",
        parsed.len(),
        recipe.image
    );

    // The same container also answers whether the search command is right. An
    // empty query means this manager has no search to check.
    if recipe.search.is_empty() {
        return;
    }
    let Some(command) = manager.search_command(recipe.search) else {
        panic!("{id}: a search query was given but this manager offers no search");
    };
    let (program, args) = command.resolve();
    let quoted: Vec<String> = args.iter().map(|arg| format!("'{arg}'")).collect();
    let script = format!(
        "{{ {setup} ; }} >/dev/null 2>&1 || true\n{program} {}",
        quoted.join(" "),
        setup = recipe.setup
    );
    let searched = Command::new(engine)
        .args(["run", "--rm", recipe.image, "sh", "-c", &script])
        .output()
        .unwrap_or_else(|error| panic!("{id}: could not search: {error}"));

    let hits = String::from_utf8_lossy(&searched.stdout);
    assert!(
        hits.contains(recipe.search),
        "{id}: `{program} {}` found no `{}`:\n{hits}\nstderr:\n{}",
        quoted.join(" "),
        recipe.search,
        String::from_utf8_lossy(&searched.stderr)
    );
    eprintln!("{id}: search found `{}`", recipe.search);

    verify_outdated(id, &recipe, engine, manager.as_ref());
}

/// Plant an out-of-date package, then check `parse_outdated` against what the
/// manager really reports.
///
/// Without this the outdated parsers are the one part of mpm with no live
/// coverage at all: they are only ever fed a fixture.
fn verify_outdated(id: &str, recipe: &Recipe, engine: &str, manager: &dyn super::Manager) {
    if recipe.outdated.is_empty() {
        return;
    }
    assert!(
        manager.outdated_command().is_some(),
        "{id}: the recipe gives an outdated command but the manager offers none"
    );

    let script = format!(
        "{{ {} ; }} >/dev/null 2>&1 || true\n{}",
        recipe.stale, recipe.outdated
    );
    let output = Command::new(engine)
        .args(["run", "--rm", recipe.image, "sh", "-c", &script])
        .output()
        .unwrap_or_else(|error| panic!("{id}: could not check outdated: {error}"));

    let listing = String::from_utf8_lossy(&output.stdout);
    let parsed = manager.parse_outdated(&listing);

    if recipe.behind.is_empty() {
        // Nothing was planted, so there may genuinely be nothing behind; what can
        // still be checked is that whatever is printed parses into usable entries.
        for spec in &parsed {
            assert!(
                spec.version.is_some(),
                "{id}: parsed `{spec}` out of `{}` with no available version:\n{listing}",
                recipe.outdated
            );
        }
        eprintln!("{id}: {} outdated entries parsed", parsed.len());
        return;
    }

    assert!(
        parsed.iter().any(|spec| spec.name == recipe.behind),
        "{id}: `{}` did not report `{}` as outdated:\n{listing}\nparsed: {:?}",
        recipe.outdated,
        recipe.behind,
        parsed
            .iter()
            .map(|spec| spec.name.as_str())
            .collect::<Vec<_>>()
    );
    eprintln!("{id}: outdated found `{}`", recipe.behind);
}

macro_rules! case {
    ($id:ident, $image:literal, $setup:literal, $install:literal, $list:literal,
     $planted:literal, $search:literal
     $(, outdated: $stale:literal => $outdated:literal, $behind:literal)?) => {
        #[test]
        fn $id() {
            #[allow(unused_mut)]
            let mut recipe = Recipe {
                search: $search,
                image: $image,
                setup: $setup,
                install: $install,
                list: $list,
                planted: $planted,
                stale: "",
                outdated: "",
                behind: "",
            };
            $(
                recipe.stale = $stale;
                recipe.outdated = $outdated;
                recipe.behind = $behind;
            )?
            verify(stringify!($id), recipe);
        }
    };
}

case!(
    apt,
    "debian:stable-slim",
    "apt-get update -qq",
    "apt-get install -y -qq ripgrep",
    "apt-mark showmanual",
    "ripgrep",
    "ripgrep",
    outdated: "apt-get update -qq" => "apt list --upgradable", ""
);

case!(
    pacman,
    "archlinux:latest",
    "pacman -Syu --noconfirm",
    "pacman -S --noconfirm ripgrep",
    "pacman -Qe",
    "ripgrep",
    "ripgrep",
    // -Sy, not -Syu: refresh the index without upgrading, or nothing is behind.
    outdated: "pacman -Sy --noconfirm" => "pacman -Qu", ""
);

case!(
    gem,
    "ruby:slim",
    ":",
    "gem install --no-document tilt",
    "gem list --local -d",
    "tilt",
    "tilt",
    outdated: "gem install --no-document tilt -v 2.0.11" => "gem outdated", "tilt"
);

case!(
    pipx,
    "python:slim",
    "pip install -q pipx",
    "pipx install cowsay",
    "pipx list --short",
    "cowsay",
    ""
);

case!(
    composer,
    "composer:latest",
    ":",
    "composer global require --no-interaction psr/log",
    "composer global show --direct",
    "psr/log",
    "psr/log",
    outdated: "composer global require --no-interaction psr/log:1.1.4"
        => "composer global outdated --direct", "psr/log"
);

case!(
    npm,
    "node:slim",
    ":",
    "npm install -g is-odd",
    "npm list -g --depth=0",
    "is-odd",
    "is-odd",
    outdated: "npm install -g is-odd@2.0.0" => "npm outdated -g", "is-odd"
);

// pnpm refuses a global install until PNPM_HOME is on PATH, so the setup step
// arranges that before anything is installed.
case!(
    pnpm,
    "node:slim",
    "npm install -g pnpm && export PNPM_HOME=/root/.pnpm && mkdir -p $PNPM_HOME/bin && export PATH=$PNPM_HOME/bin:$PATH",
    "pnpm add -g is-odd",
    "pnpm list -g --depth=0",
    "is-odd",
    ""
);

case!(
    bun,
    "oven/bun:slim",
    ":",
    "bun add -g typescript",
    "bun pm ls -g",
    "typescript",
    ""
);

case!(
    cargo,
    "rust:slim",
    ":",
    "cargo install --quiet cargo-expand",
    "cargo install --list",
    "cargo-expand",
    "ripgrep"
);

case!(
    dotnet,
    "mcr.microsoft.com/dotnet/sdk:latest",
    ":",
    "dotnet tool install -g csharpier",
    "dotnet tool list -g",
    "csharpier",
    "csharpier"
);

case!(
    apk,
    "alpine:latest",
    "apk update",
    "apk add ripgrep",
    "apk info",
    "ripgrep",
    "ripgrep"
);

case!(
    xbps,
    "ghcr.io/void-linux/void-glibc-full:latest",
    "xbps-install -Syu xbps",
    "xbps-install -y ripgrep",
    "xbps-query -m",
    "ripgrep",
    "ripgrep"
);

case!(
    dnf,
    "fedora:latest",
    ":",
    "dnf install -y ripgrep",
    "dnf repoquery --userinstalled --qf '%{name} %{evr}\\n'",
    "ripgrep",
    "ripgrep"
);

// Homebrew on Linux. The formula set and output format are shared with macOS, so
// this covers the parser even though the platform differs. The image runs as the
// `linuxbrew` user with brew already on PATH.
case!(
    brew,
    "homebrew/brew:latest",
    ":",
    "brew install --quiet hello",
    "brew list --formula --versions",
    "hello",
    "hello"
);

// No official LuaRocks image; Alpine's package installs as `luarocks-5.4`, which
// is why the recipe spells it out rather than using mpm's own `luarocks`.
case!(
    luarocks,
    "alpine:latest",
    "apk add --no-cache luarocks5.4 lua5.4-dev build-base",
    "luarocks-5.4 install penlight",
    "luarocks-5.4 list --porcelain",
    "penlight",
    ""
);
