# mpm

Declare the packages your machine should have. Let `mpm` converge to it.

`mpm` keeps a plaintext list of the packages each of your package managers should
have installed. `mpm status` tells you how the machine has drifted from those
lists; `mpm apply` installs what's missing and — only when you say so — removes
what isn't declared.

It is not a Nix replacement. It is the smallest thing that makes "my machine's
package set lives in git" true across every manager you already use.

```console
$ mpm status
pacman
  + ripgrep
  - nano

cargo
  ~ bat 0.24.0 (installed 0.23.0)

1 to install, 1 to change, 1 to remove
```

## Manifest format

One package per line. An optional second field pins a version. `#` starts a
comment, anywhere on the line.

```txt
# ~/.config/mpm/pacman
vim
ripgrep 14.1.0  # locked until the regex rewrite lands
ttf-fira-code
node@20         # a formula NAME -- brew ships versioned formulae
```

**Whitespace is the only separator.** That is deliberate: it is the one
character no package manager permits inside a package name. Every other
candidate is load-bearing somewhere — `@` in Homebrew formula names and npm
scopes, `=` in apt's own pin syntax, `:` in OCI digests. Names are therefore
opaque, and `node@20` means the formula called `node@20`.

That is the whole grammar. There are no other sigils: a manifest is a list of
packages, and a package is either wanted or absent from the file. Comments,
blank lines and your ordering are all preserved when mpm edits a file, including
comments trailing a line it rewrites.

Two things are rejected rather than quietly accepted, because both would
otherwise become a package with a strange name:

- `// old comment` — `#` is the only comment marker.
- any name starting with `-`. `pacman -S --noconfirm` is a very different
  command from installing a package called `--noconfirm`. Commands are built as
  argument vectors, which stops a name changing the *shape* of a command; it
  does not stop a name being read as an option.

## Why the host layer matters

A single flat list per manager cannot describe two machines. Your laptop needs
`tlp`; your desktop needs `nvidia`. Sync one list between them and convergence
will happily uninstall whichever one it doesn't know about.

```text
~/.config/mpm/
├── pacman               # declared on every machine
├── cargo
└── hosts/
    ├── thinkpad/pacman  # only on the host named thinkpad
    └── desktop/pacman
```

A manifest is a top-level file named after its manager. `hosts` is a reserved
directory name; since the set of manager ids is closed, it can never collide
with a manifest.

Declared packages are the **union** of the two layers. A later layer can change
a package's version, but never take a package away — so reading any one file
tells you a package is wanted, and nothing can silently withdraw it elsewhere.

That means machine-specific packages belong in the host layer, not the shared
one:

```txt
# ~/.config/mpm/pacman          -- genuinely universal
vim
ripgrep

# ~/.config/mpm/hosts/thinkpad/pacman
tlp                             # battery, laptop only

# ~/.config/mpm/hosts/desktop/pacman
nvidia
```

## Safety

Converging in both directions means `mpm` can uninstall things. The design
assumes that is dangerous:

- **`apply` shows a plan and asks.** A non-terminal stdin answers *no*; consent
  is never inferred from the absence of a terminal.
- **`--yes` is the whole opt-out.** It skips the prompt for installs and
  removals alike, so a script that passes it has said yes to both.
- **`--dry-run` cannot lie.** `status`, the dry run, and the real apply all
  compute and execute the same `Reconciliation` value, so the preview is the
  action.
- **Declaring a package is what keeps it.** Removal only ever targets packages
  no manifest mentions, and `mpm inherit` records everything installed -- kernel
  included. mpm keeps no list of packages it secretly refuses to touch.
- **Removal never touches configuration.** `apt remove`, not `apt purge`;
  `pacman -Rs`, not `-Rns`. Deleting a package's config is not a decision a
  package-list sync gets to make.

## Versions

A version means one thing everywhere: **this exact version, installable again at
any time**. A manager that cannot make that promise does not accept versions at
all — declaring one is an error, never an approximation.

| manager | versions | why |
|-|-|-|
| cargo | **yes** | crates.io is immutable; yanked crates still install by exact version |
| npm, pnpm, bun | **yes** | the registry is immutable; unpublish is restricted |
| dotnet | **yes** | NuGet is immutable |
| pipx | **yes** | PyPI keeps published releases |
| composer | **yes** | Packagist keeps published releases |
| apt, dnf | **no** | old versions are pruned from the archives |
| apk, xbps | **no** | one version per release branch |
| brew | **no** | no general way to install an old version — use `node@20` |
| flatpak | **no** | builds are addressed by commit, not version |
| luarocks | **no** | takes a version as a separate argument, not per package |
| **gem** | **no** | versions coexist, so a pin can never converge (below) |
| **pacman** | **no** | excluding a pin from an upgrade is a partial upgrade (below) |

A name that already selects a version *and* a version is a contradiction:
`node@20 20.11.0` is rejected.

### When a version can be pinned at all

Two things have to be true, and they rule out more managers than you would guess.

**The manager must keep exactly one version of a package.** RubyGems does not:

```console
$ gem install tilt -v 2.0.11 && gem install tilt -v 2.3.0
$ gem list | grep tilt
tilt (2.3.0, 2.0.11)        # both, side by side
```

Pinning `tilt 2.0.11` there would have mpm read the newest (2.3.0), install
2.0.11 — which succeeds and removes nothing — and report the same drift on every
run afterwards, forever.

**A package must own its own dependencies.** This is what rules out pacman. If
`A` is pinned and `B` depends on a newer `A`, then upgrading `B` while holding
`A` is a *partial upgrade*, which Arch does not support and which leaves `B`
linked against a version of `A` that is not installed. `pacman --ignore` will do
it and only warn.

So pinning is offered exactly where a package is installed once and carries its
own tree: cargo, npm, pnpm, bun, pipx, dotnet, composer. Everywhere else a
declared version is rejected.

### What mpm does when a version will not move anyway

It cannot force a manager to honour a pin. What it can do is refuse to claim
success. After `apply` runs, mpm reads the installed state back and names
anything the run was supposed to change but did not:

```console
$ mpm cargo apply --yes
1 to change
cargo done
  $ cargo install ripgrep@14.1.0

warning: the following did not take effect:
  cargo
    ripgrep is still at 14.0.0, not 14.1.0
A version that will not move is usually a dependency holding it there; unpin it,
or pin whatever requires it too.
```

A command that exits zero and changes nothing is the one failure a convergence
tool must not report as success. This catches the whole class: a pin fighting a
dependency, a manager that declines to downgrade, a repin that quietly no-ops.

## Getting started

```console
$ mpm inherit         # put what is installed under management
$ mpm status          # should be clean
$ cd ~/.config/mpm && git init && git add -A && git commit -m "my machine"
```

On a second machine, clone the config and:

```console
$ mpm status                        # see what this machine is missing
$ mpm apply                         # install it, remove what is undeclared
```

Host-specific packages belong in the host layer, not the shared list:

```console
$ mpm pacman add --host tlp
```

## Commands

```text
mpm [MANAGERS] <command> [PACKAGE]...
```

Managers come first and are comma-separated; packages are space-separated.

| Command | Action |
|-|-|
| `mpm [managers] status` | show drift; exits non-zero when the machine doesn't match |
| `mpm [managers] apply` | converge (plan, confirm, execute) |
| `mpm [managers] apply --dry-run` | show the plan and stop |
| `mpm [managers] inherit` | put installed packages under management, merging |
| `mpm <managers> add <pkg>…` | declare packages, for `apply` to install |
| `mpm <managers> add --pin <pkg>…` | declare them at the version installed right now |
| `mpm <managers> pin <pkg>…` | lock packages you already have |
| `mpm <managers> unpin <pkg>…` | let them track whatever is current |
| `mpm <managers> remove <pkg>…` | undeclare and uninstall now |
| `mpm [managers] search <query>` | search each manager's own index |
| `mpm [managers] outdated` | declared packages with a newer version available |
| `mpm [managers] upgrade` | bring packages up to date, leaving pins alone |
| `mpm managers` | supported managers, host, manifest path |

```console
$ mpm status                        # every managed manager
$ mpm cargo,npm status              # just these two
$ mpm pacman,brew inherit
$ mpm cargo add ripgrep bat fd      # one manager, three packages
$ mpm cargo,npm add typescript      # one package, two manifests
```

`status`, `apply` and `inherit` act on every managed manager when none is named.
The editing commands require at least one, since they write to a manifest:

```console
$ mpm add ripgrep
error: name a package manager first, as in `mpm cargo add ...`
```

`outdated` is read-only and exits zero either way: a newer version existing is
not a fault, since an entry without a version means "any version" and the
manifest is satisfied. It separates the two cases, because only one is mpm's to
act on:

```console
$ mpm pacman outdated
pacman
  ripgrep 14.1.0-1 -> 15.2.0-1
  bat -> 0.25.0-1 (unpinned; an ordinary upgrade picks this up)

1 pinned package(s) behind.
```

`upgrade` is the one upgrade only mpm can run, because no package manager knows
which of its packages your manifests pin:

```console
$ mpm pacman upgrade --dry-run
pacman
  $ pacman -Syu --noconfirm --ignore ripgrep

note: 1 pinned package(s) left alone:
  pacman: ripgrep
```

Managers split two ways here, which is why this is per-manager rather than one
command. Arch forbids partial upgrades, so the only safe shape is a full `-Syu`
with the pins excluded. Everything else has no exclusion flag but upgrades named
packages happily, so it is handed the unpinned ones instead:

```console
$ mpm cargo upgrade --dry-run
cargo
  $ cargo install --force bat fd-find

note: 1 pinned package(s) left alone:
  cargo: ripgrep
```

Being a write, `upgrade` goes through the same plan, confirmation and parallel
execution as `apply`, and takes the same `--dry-run` and `--yes`. `pnpm`,
`flatpak`, `bun`, `luarocks` and `dotnet` have no upgrade mpm can drive safely
and are skipped.

`search` is a pass-through: it runs each manager's own search and shows the
output unchanged. Nothing is parsed, because a search result is prose --
descriptions, relevance ordering, highlighting -- and mpm has no business
reshaping it. What it adds is the question only mpm can answer: of the managers
*you* use, which have this?

```console
$ mpm search ripgrep
cargo
  ripgrep = "15.2.0"    # recursively searches directories for a regex pattern
pacman
  extra/ripgrep 15.2.0-1
```

Unlike the other commands, `search` considers every manager installed on the
machine rather than only managed ones -- deciding where to install something from
is the point. `pnpm` and `bun` have no search of their own and are skipped.

Bare `mpm` prints help rather than doing anything.

Versions are always deliberate. `mpm inherit` records **names only**, so ordinary
upgrades stay none of mpm's business — a manifest entry without a version means
"any version", and `pacman -Syu` upgrading it is not drift.

Pinning is reachable two ways, because it is both something you decide when
declaring a package and something you do to one you already have:

```console
$ mpm cargo add --pin ripgrep   # declare and lock in one step
$ mpm cargo pin ripgrep         # lock what is already declared
$ mpm cargo unpin ripgrep       # back to tracking current
```

Both read the installed version off the machine, so you never type one by hand.
You can still write the pair yourself: `mpm cargo add 'ripgrep 14.1.0'`. A
manager that cannot hold a version rejects the attempt — see above for which,
and why.

`add`, `pin`, `unpin` and `remove` take `--host` to target this machine's layer
instead of the shared one.

To edit a manifest by hand, open it — `mpm managers` prints the path.

If a `hosts/` tree exists but has no directory for this machine, mpm says so —
a mistyped host name would otherwise be silent, and every package in it would
become a removal candidate.

## How `apply` runs

Every manager runs in its own thread, because they share nothing — `npm` has no
reason to wait on `pacman`. Each reports as it finishes, with its output grouped
under it rather than interleaved with everyone else's:

```console
$ mpm apply --yes
brew done
  $ brew install htop
    ==> Downloading htop
pacman done
  $ pacman -S --needed vim
    installing vim...
```

A manager that fails is reported on its own and does not stop the others; the
run exits non-zero naming which ones failed. If anything needs root, mpm asks
once up front — workers capture their output, so an elevator prompting inside
one would block with nothing on screen to explain why.

## Supported managers

`apk`, `apt`, `brew`, `bun`, `cargo`, `composer`, `dnf`, `dotnet`, `flatpak`, `gem`,
`luarocks`, `npm`, `pacman`, `pipx`, `pnpm`, `xbps`.

A manager is used when its executable is on `$PATH` *and* it has at least one
manifest layer. `mpm managers` shows both.

A manager earns a place here only when its *explicitly installed* set can be
listed, and only once its real output has been captured by running it. Nothing
here is written from documentation alone.

Each manager is one file under `src/manager/`, holding its command shapes, its
output parser, and tests against output captured from the real tool.

Some obvious candidates are deliberately absent:

- `go` and `deno` have no command that lists what they installed.
- `nix profile` and `helm plugin` list short names but install from flake refs
  and URLs, so what they report cannot be fed back to them.
- `rpm` and `zypper` list dependencies alongside the packages you asked for,
  with no flag to separate them -- the same reason mpm uses `pacman -Qe`
  rather than `pacman -Q`.

`apk` is the one exception to the "explicitly installed" rule: Alpine draws no
line between a package you asked for and one pulled in as a dependency, so
inheriting there records the base system too.

## Environment

| Variable | Effect |
|-|-|
| `MPM_CONFIG_DIR` | manifest tree location (default `<config dir>/mpm`) |
| `MPM_HOST` | override the detected hostname |
| `MPM_SUDO` | privilege elevator (default `sudo`; set empty to never escalate) |
| `NO_COLOR` | disable colour |

## Build

```console
$ cargo build --release
$ install -Dm755 target/release/mpm ~/.local/bin/mpm
```

Tests come in three tiers, fastest first.

**Unit tests** need nothing but a toolchain:

```console
$ nix-shell --run 'cargo test'
```

**Container tests** run each manager in its own image, install a known package
with the real tool, and feed what it prints straight into mpm's parser. Nothing
is recorded to disk, so a manager that changes its output format in next week's
release fails a test here rather than mis-parsing somebody's machine:

```console
$ MPM_CONTAINERS=1 cargo test container          # every manager
$ MPM_CONTAINERS=1 cargo test container::pacman  # just one
...
pacman: 2 packages parsed from archlinux:latest
apt: 1 packages parsed from debian:stable-slim
```

Covered: `apk` (Alpine), `apt` (Debian), `dnf` (Fedora), `pacman` (Arch),
`xbps` (Void), `gem`,
`pipx`, `composer`, `npm`, `pnpm`, `bun`, `cargo`, `dotnet`. The same container
also checks each manager's `search` command, so those are verified rather than
guessed. Only `flatpak` is left out: installing anything needs a privileged
container.

**End-to-end tests** drive mpm itself rather than just its parsers, against
managers that can be redirected by environment variable:

```console
$ nix-shell shell-test.nix --run 'MPM_INTEGRATION=1 cargo test'
...
read path verified against: pipx, gem, composer, npm
full apply verified against: pipx, gem, composer, npm
```

Each installs a package directly, has mpm `inherit` it, checks the manifest mpm
wrote, then declares and undeclares it and confirms `apply` really installs and
removes. Managers that can only be redirected by flag get their parser checked
instead -- pacman takes `--dbpath` as a flag that mpm never passes, so that test
lays down a throwaway local database and asserts `pacman -Qe` leaves out a
dependency while `pacman -Q` includes it.

All three tiers skip themselves when their environment variable is unset, so the
plain `cargo test` stays fast.

## Coming from metapam

The old flat files map straight onto the top level, and the old `name@version` syntax
becomes `name version`:

```console
$ mkdir -p ~/.config/mpm
$ cp ~/.config/metapam/* ~/.config/mpm/
$ mpm status          # then fix up any pins it rejects
```

Then fix up anything mpm rejects: `//` comments become `#`, and `name@version`
becomes `name version`. Drop any file for a manager that is no longer supported
(`fisher`, `yarn`, `bun`, `paru`, `yay` -- the last two are now part of `pacman`).

## Scope

`mpm` manages the *set* of globally installed packages. It deliberately does not
wrap `search`, `upgrade`, or `outdated` — your package manager already does those
better, and a unified interface over them is
[meta-package-manager](https://github.com/kdeldycke/meta-package-manager)'s job,
not this one.
