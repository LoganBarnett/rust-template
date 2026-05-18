{
  description = "Rust Template - Best-in-class Rust project setup";
  inputs = {
    # LLM: Do NOT change this URL unless explicitly directed. This is the
    # correct format for nixpkgs stable (25.11 is correct, not nixos-25.11).
    nixpkgs.url = "github:NixOS/nixpkgs/25.11";
    rust-overlay.url = "github:oxalica/rust-overlay";
    crane.url = "github:ipetkov/crane";
    changelog-roller.url = "github:LoganBarnett/changelog-roller";
    foundation.url = "github:LoganBarnett/rust-template";
    foundation.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    crane,
    changelog-roller,
    foundation,
  } @ inputs: let
    forAllSystems =
      nixpkgs.lib.genAttrs nixpkgs.lib.systems.flakeExposed;
    perSystem = forAllSystems (system: let
      pkgs = import nixpkgs {
        inherit system;
        overlays = [(import rust-overlay)];
      };
      craneLib =
        (crane.mkLib pkgs).overrideToolchain
        (p: p.rust-bin.stable.latest.default);
    in
      foundation.lib.mkRustProject {
        inherit self pkgs craneLib;
        name = "rust-template";
        crates = {
          # CRATE_ENTRIES

          # Note: The 'lib' crate is not included here as it doesn't
          # produce a binary.
        };
        extraDevPackages = [
          pkgs.cargo-sweep
          pkgs.jq
          # Elm toolchain
          pkgs.elmPackages.elm
          pkgs.elmPackages.elm-format
          pkgs.elm2nix
          # Unified formatter
          pkgs.treefmt
          pkgs.alejandra
          pkgs.prettier
          pkgs.just
          changelog-roller.packages.${system}.default
        ];
        shellHook = ''
          echo "Rust Template development environment"
          echo ""
          echo "Available Cargo packages (use 'cargo build -p <name>'):"
          cargo metadata --no-deps --format-version 1 2>/dev/null | \
            jq -r '.packages[].name' | \
            sort | \
            sed 's/^/  • /' || echo "  Run 'cargo init' to get started"

          echo ""
          echo "Elm frontend (frontend/):"
          echo "  Build:   cd frontend && elm make src/Main.elm --output public/elm.js"
          echo "  Format:  treefmt"
          echo "  After changing elm.json dependency versions, regenerate Nix files:"
          echo "    cd frontend"
          echo "    elm2nix convert 2>/dev/null > elm-srcs.nix"
          echo "    elm2nix snapshot"
          echo "    git add elm-srcs.nix registry.dat && git commit"
        '';
      });
  in {
    devShells =
      nixpkgs.lib.mapAttrs (_: p: {default = p.devShell;}) perSystem;
    packages = nixpkgs.lib.mapAttrs (_: p: p.packages) perSystem;
    apps = nixpkgs.lib.mapAttrs (_: p: p.apps) perSystem;

    # ================================================================
    # NIXOS MODULES
    # ================================================================
    nixosModules = {
      server = import ./nix/modules/nixos-server.nix {
        inherit self foundation;
      };
      default = self.nixosModules.server;
    };

    # ================================================================
    # DARWIN MODULES
    # ================================================================
    darwinModules = {
      server = import ./nix/modules/darwin-server.nix {
        inherit self foundation;
      };
      default = self.darwinModules.server;
    };
  };
}
