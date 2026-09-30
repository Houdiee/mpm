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
| npm, pnpm | **yes** | the registry is immutable; unpublish is restricted |
| dotnet | **yes** | NuGet is immutable |
| pacman | **yes** | the Arch Linux Archive keeps every official build permanently |
| apt | **no** | Debian and Ubuntu prune old versions; `name=version` works today and fails later |
| brew | **no** | no general way to install an old version — use a versioned formula, `node@20` |

Two further rules:

- A name that already selects a version *and* a version is a contradiction:
  `node@20 20.11.0` is rejected.
- On pacman the version must include the pkgrel exactly as `pacman -Qe` prints
  it (`14.1.0-1`), and the package must be in the official repositories. **AUR
  packages cannot be pinned** — nothing archives them — and that is reported by
  `mpm status`, not only when installing.

### A note on Arch

`pacman`, `paru` and `yay` read the same database, so mpm exposes one `pacman`
manager. An AUR helper is an installation detail: if `paru` or `yay` is on
`$PATH` it is used to install, and everything else goes through `pacman`.

A pinned package is fetched straight from the archive:

```console
$ pacman -U https://archive.archlinux.org/packages/r/ripgrep/ripgrep-14.1.0-1-x86_64.pkg.tar.zst
```

Two things this does not do: the dependencies come from your *current* repos, so
a very old package may not resolve; and `pacman -Syu` will upgrade a pinned
package back. mpm treats that the same as any other drift — `status` reports it
and `apply` puts it back.
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
$ mpm add --host pacman tlp
```

## Commands

| Command | Action |
|-|-|
| `mpm status [manager…]` | show drift; exits non-zero when the machine doesn't match |
| `mpm apply [manager…]` | converge (plan, confirm, execute) |
| `mpm apply --dry-run` | show the plan and stop |
| `mpm inherit [manager…]` | put installed packages under management, merging |
| `mpm add <manager> <pkg>…` | declare packages, for `apply` to install |
| `mpm add --pin <manager> <pkg>…` | declare them at the version installed right now |
| `mpm pin <manager> <pkg>…` | lock packages you already have |
| `mpm unpin <manager> <pkg>…` | let them track whatever is current |
| `mpm remove <manager> <pkg>…` | undeclare and uninstall now |
| `mpm managers` | supported managers, host, manifest path |

Managers are positional. `status`, `apply` and `inherit` take any number and act
on every managed manager when given none; the editing commands take the manager
first, then packages:

```console
$ mpm status                  # every managed manager
$ mpm status cargo npm        # just these two
$ mpm inherit pacman brew
$ mpm add cargo ripgrep bat   # manager first, then packages
$ mpm add cargo,npm fd        # one package, two manifests
```

Bare `mpm` prints help rather than doing anything.

Versions are always deliberate. `mpm inherit` records **names only**, so ordinary
upgrades stay none of mpm's business — a manifest entry without a version means
"any version", and `pacman -Syu` upgrading it is not drift.

Pinning is reachable two ways, because it is both something you decide when
declaring a package and something you do to one you already have:

```console
$ mpm add --pin pacman ripgrep   # declare and lock in one step
$ mpm pin pacman ripgrep         # lock what is already declared
$ mpm unpin pacman ripgrep       # back to tracking current
```

Both read the installed version off the machine, so you never type a version by
hand — which matters on Arch, where the pkgrel (`14.1.0-1`) is part of it. You
can still write the pair yourself if you want: `mpm add cargo 'ripgrep 14.1.0'`.

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

`apt`, `brew`, `cargo`, `dotnet`, `npm`, `pacman`, `pnpm`.

A manager is used when its executable is on `$PATH` *and* it has at least one
manifest layer. `mpm managers` shows both.

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
