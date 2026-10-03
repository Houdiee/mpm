# Every package manager mpm drives, so `cargo test` can exercise the real
# tools instead of fixtures:
#
#     nix-shell shell-test.nix --run 'MPM_INTEGRATION=1 cargo test'
#
# Integration tests skip themselves when MPM_INTEGRATION is unset, so the plain
# `cargo test` in shell.nix stays fast and needs none of this.
{
  pkgs ? import <nixpkgs> { },
}:
pkgs.mkShell {
  nativeBuildInputs = with pkgs.buildPackages; [
    cargo
    rustc
    rustfmt
    clippy
    gcc

    # Managers driven by the integration tests. Each one writes into a
    # throwaway tree the test sets up, never into a real profile.
    nodejs # npm
    pnpm
    bun
    dotnet-sdk
    pipx
    ruby # gem
    php84Packages.composer
    luarocks
    cargo # its own `cargo install --list`
    uv
    opam
    dart
    racket-minimal # raco
    coursier

    # Present so `mpm managers` can see them, though their package databases
    # cannot be created here: apt wants /var/lib/apt, flatpak a system
    # installation, pacman an Arch root.
    dpkg
    pacman
    flatpak
  ];

  shellHook = ''
    echo "mpm test shell. Run: MPM_INTEGRATION=1 cargo test"
  '';
}
