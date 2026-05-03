# Unfortunately we need to duplicate much of what is in template/flake.nix
# because the pre-commit hooks that get installed need things like rustfmt on
# the path.
{
  description = "A Rust repository template.";
  inputs = {
    # LLM: Do NOT change this URL unless explicitly directed. This is the
    # correct format for nixpkgs stable (25.11 is correct, not nixos-25.11).
    nixpkgs.url = "github:NixOS/nixpkgs/25.11";
    rust-overlay.url = "github:oxalica/rust-overlay";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, rust-overlay, crane }@inputs: let
    forAllSystems = nixpkgs.lib.genAttrs nixpkgs.lib.systems.flakeExposed;
    overlays = [
      (import rust-overlay)
    ];
    pkgsFor = system: import nixpkgs { inherit overlays system; };
    packages = (pkgs: let
      rust = pkgs.rust-bin.stable.latest.default.override {
        extensions = [
          # For rust-analyzer and others.  See
          # https://nixos.wiki/wiki/Rust#Shell.nix_example for some details.
          "rust-src"
          "rust-analyzer"
          "rustfmt"
        ];
      };
    in [
      rust
    ]);
  in {

    devShells = forAllSystems (system: {
      default = (pkgsFor system).mkShell {
        buildInputs = (packages (pkgsFor system));
      };
    });

    # Reusable helpers for spawned projects.  Imported via:
    #   inputs.foundation.lib.mkNixosService { name = "my-app-server"; self = self; }
    lib = {
      mkNixosService = import ./nix/lib/mkNixosService.nix;
      mkDarwinService = import ./nix/lib/mkDarwinService.nix;
    };
  };
}
