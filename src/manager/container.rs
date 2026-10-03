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
#[derive(Default)]
struct Recipe {
    /// The manager's id, as [`get`] knows it.
    id: &'static str,
    /// Query to pass to the manager's own search, when it has one.
    search: &'static str,
    image: &'static str,
    /// Refresh repositories; empty when nothing is needed.
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

/// `:` is the shell no-op, so a case that needs no preparation leaves `setup`
/// empty rather than spelling it out.
fn shell_setup(setup: &str) -> &str {
    if setup.is_empty() { ":" } else { setup }
}

/// `docker run` arguments. The entrypoint is always overridden to `sh`: some
/// images declare one that would read the script as its own arguments.
fn run_args(recipe: &Recipe, script: &str) -> Vec<String> {
    vec![
        "run".to_string(),
        "--rm".to_string(),
        "--entrypoint".to_string(),
        "sh".to_string(),
        recipe.image.to_string(),
        "-c".to_string(),
        script.to_string(),
    ]
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
        // Both steps are braced before being silenced: a redirection binds to one
        // command, so `a && b >/dev/null` would leave `a`'s output in the listing
        // and mpm would parse it as a package.
        "{{ {setup} ; }} >/dev/null 2>&1 || true\n{{ {install} ; }} >/dev/null 2>&1\n{list}",
        setup = shell_setup(recipe.setup),
        install = recipe.install,
        list = recipe.list,
    );
    let output = Command::new(engine)
        .args(run_args(&recipe, &script))
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
        setup = shell_setup(recipe.setup)
    );
    let searched = Command::new(engine)
        .args(run_args(&recipe, &script))
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
        .args(run_args(recipe, &script))
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

/// Each case names its fields, so a manager with no search or no outdated query
/// leaves them out rather than passing an empty string in the right position.
fn case(recipe: Recipe) {
    let id = recipe.id;
    verify(id, recipe);
}

#[test]
fn apt() {
    case(Recipe {
        id: "apt",
        image: "debian:stable-slim",
        setup: "apt-get update -qq",
        install: "apt-get install -y -qq ripgrep",
        list: "apt-mark showmanual",
        planted: "ripgrep",
        search: "ripgrep",
        stale: "apt-get update -qq",
        outdated: "apt list --upgradable",
        ..Recipe::default()
    });
}

#[test]
fn pacman() {
    case(Recipe {
        id: "pacman",
        image: "archlinux:latest",
        setup: "pacman -Syu --noconfirm",
        install: "pacman -S --noconfirm ripgrep",
        list: "pacman -Qe",
        planted: "ripgrep",
        search: "ripgrep",
        // -Sy, not -Syu: refresh the index without upgrading, or nothing is behind.
        stale: "pacman -Sy --noconfirm",
        outdated: "pacman -Qu",
        ..Recipe::default()
    });
}

#[test]
fn gem() {
    case(Recipe {
        id: "gem",
        image: "ruby:slim",
        install: "gem install --no-document tilt",
        list: "gem list --local -d",
        planted: "tilt",
        search: "tilt",
        stale: "gem install --no-document tilt -v 2.0.11",
        outdated: "gem outdated",
        behind: "tilt",
        ..Recipe::default()
    });
}

#[test]
fn pipx() {
    case(Recipe {
        id: "pipx",
        image: "python:slim",
        setup: "pip install -q pipx",
        install: "pipx install cowsay",
        list: "pipx list --short",
        planted: "cowsay",
        ..Recipe::default()
    });
}

#[test]
fn composer() {
    case(Recipe {
        id: "composer",
        image: "composer:latest",
        install: "composer global require --no-interaction psr/log",
        list: "composer global show --direct",
        planted: "psr/log",
        search: "psr/log",
        stale: "composer global require --no-interaction psr/log:1.1.4",
        outdated: "composer global outdated --direct",
        behind: "psr/log",
        ..Recipe::default()
    });
}

#[test]
fn npm() {
    case(Recipe {
        id: "npm",
        image: "node:slim",
        install: "npm install -g is-odd",
        list: "npm list -g --depth=0",
        planted: "is-odd",
        search: "is-odd",
        stale: "npm install -g is-odd@2.0.0",
        outdated: "npm outdated -g",
        behind: "is-odd",
        ..Recipe::default()
    });
}

#[test]
fn pnpm() {
    case(Recipe {
        id: "pnpm",
        image: "node:slim",
        // pnpm refuses a global install until PNPM_HOME is on PATH.
        setup: "npm install -g pnpm && export PNPM_HOME=/root/.pnpm && mkdir -p $PNPM_HOME/bin && export PATH=$PNPM_HOME/bin:$PATH",
        install: "pnpm add -g is-odd",
        list: "pnpm list -g --depth=0",
        planted: "is-odd",
        ..Recipe::default()
    });
}

#[test]
fn bun() {
    case(Recipe {
        id: "bun",
        image: "oven/bun:slim",
        install: "bun add -g typescript",
        list: "bun pm ls -g",
        planted: "typescript",
        ..Recipe::default()
    });
}

#[test]
fn cargo() {
    case(Recipe {
        id: "cargo",
        image: "rust:slim",
        install: "cargo install --quiet cargo-expand",
        list: "cargo install --list",
        planted: "cargo-expand",
        search: "ripgrep",
        ..Recipe::default()
    });
}

#[test]
fn dotnet() {
    case(Recipe {
        id: "dotnet",
        image: "mcr.microsoft.com/dotnet/sdk:latest",
        install: "dotnet tool install -g csharpier",
        list: "dotnet tool list -g",
        planted: "csharpier",
        search: "csharpier",
        ..Recipe::default()
    });
}

#[test]
fn apk() {
    case(Recipe {
        id: "apk",
        image: "alpine:latest",
        setup: "apk update",
        install: "apk add ripgrep",
        list: "apk info",
        planted: "ripgrep",
        search: "ripgrep",
        ..Recipe::default()
    });
}

#[test]
fn xbps() {
    case(Recipe {
        id: "xbps",
        image: "ghcr.io/void-linux/void-glibc-full:latest",
        setup: "xbps-install -Syu xbps",
        install: "xbps-install -y ripgrep",
        list: "xbps-query -m",
        planted: "ripgrep",
        search: "ripgrep",
        ..Recipe::default()
    });
}

#[test]
fn dnf() {
    case(Recipe {
        id: "dnf",
        image: "fedora:latest",
        install: "dnf install -y ripgrep",
        list: "dnf repoquery --userinstalled --qf '%{name} %{evr}\\n'",
        planted: "ripgrep",
        search: "ripgrep",
        ..Recipe::default()
    });
}

// Homebrew on Linux. The formula set and output format are shared with macOS, so
// this covers the parser even though the platform differs.
#[test]
fn brew() {
    case(Recipe {
        id: "brew",
        image: "homebrew/brew:latest",
        install: "brew install --quiet hello",
        list: "brew list --formula --versions",
        planted: "hello",
        search: "hello",
        ..Recipe::default()
    });
}

// No official LuaRocks image; Alpine's package installs as `luarocks-5.4`, which
// is why this spells it out rather than using mpm's own `luarocks`.
#[test]
fn luarocks() {
    case(Recipe {
        id: "luarocks",
        image: "alpine:latest",
        setup: "apk add --no-cache luarocks5.4 lua5.4-dev build-base",
        install: "luarocks-5.4 install penlight",
        list: "luarocks-5.4 list --porcelain",
        planted: "penlight",
        ..Recipe::default()
    });
}

#[test]
fn zypper() {
    case(Recipe {
        id: "zypper",
        image: "opensuse/tumbleweed",
        install: "zypper --non-interactive install ripgrep",
        list: "zypper --quiet search --installed-only --details --type package",
        planted: "ripgrep",
        search: "ripgrep",
        ..Recipe::default()
    });
}

#[test]
fn nix() {
    case(Recipe {
        id: "nix",
        image: "nixos/nix",
        install: "nix-env --install hello",
        list: "nix-env -q",
        planted: "hello",
        search: "hello",
        ..Recipe::default()
    });
}

#[test]
fn uv() {
    case(Recipe {
        id: "uv",
        image: "ghcr.io/astral-sh/uv:debian",
        install: "uv tool install cowsay",
        list: "uv tool list",
        planted: "cowsay",
        ..Recipe::default()
    });
}

#[test]
fn opam() {
    case(Recipe {
        id: "opam",
        image: "ocaml/opam",
        install: "opam install --yes ocamlfind",
        list: "opam list --installed-roots --short --columns=name,version",
        planted: "ocamlfind",
        search: "ocamlfind",
        ..Recipe::default()
    });
}

#[test]
fn dart_pub() {
    case(Recipe {
        id: "pub",
        image: "dart:stable",
        install: "dart pub global activate http",
        list: "dart pub global list",
        planted: "http",
        ..Recipe::default()
    });
}

#[test]
fn raco() {
    case(Recipe {
        id: "raco",
        image: "racket/racket:latest",
        install: "raco pkg install --auto --batch rackunit-lib",
        list: "raco pkg show",
        planted: "rackunit-lib",
        ..Recipe::default()
    });
}

// No image ships coursier, so the setup step fetches its launcher the way its
// own install instructions do.
#[test]
fn coursier() {
    case(Recipe {
        id: "coursier",
        image: "eclipse-temurin:21",
        setup: "curl -fsSL https://github.com/coursier/launchers/raw/master/coursier > /usr/local/bin/cs && chmod +x /usr/local/bin/cs",
        install: "cs install scalafmt",
        list: "cs list",
        planted: "scalafmt",
        ..Recipe::default()
    });
}

// `go version -m <dir>` instead of a subcommand that does not exist: the bin
// directory is the installed set, and Go stamps the module and version into
// every binary it builds.
#[test]
fn go() {
    case(Recipe {
        id: "go",
        image: "golang:latest",
        install: "go install github.com/rakyll/hey@v0.1.5",
        list: "go version -m /go/bin",
        planted: "github.com/rakyll/hey",
        ..Recipe::default()
    });
}

#[test]
fn rustup() {
    case(Recipe {
        id: "rustup",
        image: "rust:slim",
        install: "rustup toolchain install 1.80.0 --profile minimal",
        list: "rustup show",
        planted: "1.80.0",
        ..Recipe::default()
    });
}

#[test]
fn mise() {
    case(Recipe {
        id: "mise",
        image: "debian:stable-slim",
        setup: "apt-get update -qq && apt-get install -y -qq curl git ca-certificates && curl -fsSL https://mise.run | sh && export PATH=$HOME/.local/bin:$PATH",
        install: "export PATH=$HOME/.local/bin:$PATH && mise use --global node@22.11.0",
        list: "export PATH=$HOME/.local/bin:$PATH; mise ls --current",
        planted: "node",
        ..Recipe::default()
    });
}

#[test]
fn asdf() {
    case(Recipe {
        id: "asdf",
        image: "golang:latest",
        setup: "go install github.com/asdf-vm/asdf/cmd/asdf@latest && export PATH=/go/bin:$PATH && asdf plugin add nodejs",
        install: "export PATH=/go/bin:$PATH; asdf install nodejs 22.11.0 && asdf set --home nodejs 22.11.0",
        list: "export PATH=/go/bin:$PATH; asdf current",
        planted: "nodejs",
        ..Recipe::default()
    });
}

// pyenv builds Python from source, so the recipe installs a version and the
// listing is what mpm reads back: a bare version per line.
#[test]
fn pyenv() {
    case(Recipe {
        id: "pyenv",
        image: "python:slim",
        setup: "apt-get update -qq && apt-get install -y -qq git curl build-essential libssl-dev zlib1g-dev libbz2-dev libffi-dev libreadline-dev libsqlite3-dev && curl -fsSL https://pyenv.run | bash",
        install: "export PYENV_ROOT=$HOME/.pyenv; export PATH=$PYENV_ROOT/bin:$PATH; pyenv install --skip-existing 3.12.8",
        list: "export PYENV_ROOT=$HOME/.pyenv; export PATH=$PYENV_ROOT/bin:$PATH; pyenv versions --bare",
        planted: "3.12.8",
        ..Recipe::default()
    });
}

#[test]
fn volta() {
    case(Recipe {
        id: "volta",
        image: "debian:stable-slim",
        setup: "apt-get update -qq && apt-get install -y -qq curl ca-certificates && curl -fsSL https://get.volta.sh | bash -s -- --skip-setup && export VOLTA_HOME=$HOME/.volta && export PATH=$VOLTA_HOME/bin:$PATH",
        install: "export VOLTA_HOME=$HOME/.volta; export PATH=$VOLTA_HOME/bin:$PATH; volta install node@22",
        list: "export VOLTA_HOME=$HOME/.volta; export PATH=$VOLTA_HOME/bin:$PATH; volta list all",
        planted: "node",
        ..Recipe::default()
    });
}

// code-server shares VS Code's extension CLI, so it stands in for `code` and
// `codium` too -- neither ships an image, and all three take the same flags.
#[test]
fn code_server() {
    case(Recipe {
        id: "code-server",
        image: "codercom/code-server:latest",
        install: "code-server --install-extension esbenp.prettier-vscode",
        list: "code-server --list-extensions --show-versions",
        planted: "esbenp.prettier-vscode",
        ..Recipe::default()
    });
}

#[test]
fn pip() {
    case(Recipe {
        id: "pip",
        image: "python:slim",
        install: "pip install --user cowsay requests",
        list: "pip list --user --not-required --format=freeze",
        planted: "cowsay",
        stale: "pip install --user cowsay==5.0",
        outdated: "pip list --user --outdated --format=columns",
        behind: "cowsay",
        ..Recipe::default()
    });
}

#[test]
fn krew() {
    case(Recipe {
        id: "krew",
        image: "debian:stable-slim",
        setup: "apt-get update -qq && apt-get install -y -qq curl ca-certificates git && export KREW_ROOT=/tmp/krew && curl -fsSL https://github.com/kubernetes-sigs/krew/releases/latest/download/krew-linux_amd64.tar.gz | tar xz -C /tmp && /tmp/krew-linux_amd64 install krew && cp /tmp/krew/bin/kubectl-krew /usr/local/bin/",
        install: "export KREW_ROOT=/tmp/krew; kubectl-krew install ctx",
        list: "export KREW_ROOT=/tmp/krew; kubectl-krew list",
        planted: "ctx",
        search: "ctx",
        ..Recipe::default()
    });
}

#[test]
fn pixi() {
    case(Recipe {
        id: "pixi",
        image: "debian:stable-slim",
        setup: "apt-get update -qq && apt-get install -y -qq curl ca-certificates && curl -fsSL https://pixi.sh/install.sh | sh && export PATH=$HOME/.pixi/bin:$PATH",
        install: "export PATH=$HOME/.pixi/bin:$PATH; pixi global install jq==1.7.1",
        list: "export PATH=$HOME/.pixi/bin:$PATH; pixi global list",
        planted: "jq",
        ..Recipe::default()
    });
}
