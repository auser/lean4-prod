{
  description = "lean4-prod — whole-language Lean 4 → production Rust (LCNF-based, no_std + wasm)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            pkgs.lean4
            pkgs.cargo
            pkgs.rustc
            pkgs.clippy
            pkgs.rustfmt
            pkgs.lld
            pkgs.clang
            pkgs.python3
            pkgs.nodejs_22
            pkgs.typescript
            pkgs.kotlin
            pkgs.jdk
            pkgs.just
            pkgs.ripgrep
            pkgs.wasm-pack
          ];

          shellHook = ''
            # wasm target for the portable half
            # (prod-ir / prod-codegen / prod-wasm)
            if command -v rustup >/dev/null 2>&1; then
              rustup target add wasm32-unknown-unknown \
                >/dev/null 2>&1 || true
            fi

            # nix develop initially starts Bash. Replace only the
            # interactive shell with the user's configured zsh.
            if [[ "$-" == *i* ]]; then
              exec /bin/zsh -il
            fi
          '';
        };
      });
}
