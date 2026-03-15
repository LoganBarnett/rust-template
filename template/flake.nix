{
  description = "Rust Template - Best-in-class Rust project setup";
  inputs = {
    nixpkgs.url = github:NixOS/nixpkgs/25.11;
    rust-overlay.url = "github:oxalica/rust-overlay";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, rust-overlay, crane }@inputs: let
    forAllSystems = nixpkgs.lib.genAttrs nixpkgs.lib.systems.flakeExposed;
    overlays = [
      (import rust-overlay)
    ];
    pkgsFor = system: import nixpkgs {
      inherit system;
      overlays = overlays;
    };

    # ============================================================================
    # WORKSPACE CRATES CONFIGURATION
    # ============================================================================
    # Define all workspace crates here. This makes it easy to:
    # - Generate packages
    # - Generate apps
    # - Generate overlays
    # - Keep package lists consistent across the flake
    #
    # When customizing this template for your project:
    # 1. Update the names below to match your project
    # 2. Add/remove crates as needed
    # 3. The package and app generation will automatically update
    # ============================================================================
    workspaceCrates = {
      # CRATE:cli:begin
      # CLI application
      cli = {
        name = "rust-template-cli";
        binary = "rust-template-cli";
        description = "CLI application";
      };
      # CRATE:cli:end

      # CRATE:web:begin
      # Web service
      web = {
        name = "rust-template-web";
        binary = "rust-template-web";
        description = "Web service";
      };
      # CRATE:web:end

      # Note: The 'lib' crate is not included here as it doesn't produce a binary
    };

    # Development shell packages.
    devPackages = pkgs: let
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
      pkgs.cargo-sweep
      pkgs.pkg-config
      pkgs.openssl
      pkgs.jq
    ];
  in {

    devShells = forAllSystems (system: {
      default = (pkgsFor system).mkShell {
        buildInputs = devPackages (pkgsFor system);
        shellHook = ''
          echo "Rust Template development environment"
          echo ""
          echo "Available Cargo packages (use 'cargo build -p <name>'):"
          cargo metadata --no-deps --format-version 1 2>/dev/null | \
            jq -r '.packages[].name' | \
            sort | \
            sed 's/^/  • /' || echo "  Run 'cargo init' to get started"
        '';
      };
    });

    # ============================================================================
    # PACKAGES
    # ============================================================================
    packages = forAllSystems (system: let
      pkgs = pkgsFor system;
      craneLib = (crane.mkLib pkgs).overrideToolchain (p: p.rust-bin.stable.latest.default);

      # Common build arguments shared by all crates
      commonArgs = {
        src = craneLib.cleanCargoSource ./.;
        buildInputs = with pkgs; [
          openssl
        ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin (with pkgs.darwin; [
          libiconv
        ]);
        nativeBuildInputs = with pkgs; [
          pkg-config
        ];
        # Run only unit tests (--lib --bins), skip integration tests in tests/ directories
        # Integration tests may require external services not available in Nix sandbox
        # Full test suite can be run locally with 'cargo test --all'
        cargoTestExtraArgs = "--lib --bins";
      };

      # Build individual crate packages from workspaceCrates.
      cratePackages = pkgs.lib.mapAttrs (key: crate:
        craneLib.buildPackage (commonArgs // {
          pname = crate.name;
          cargoExtraArgs = "-p ${crate.name}";
        })
      ) workspaceCrates;

    in cratePackages // {
      # Build all workspace binaries together.
      # Update pname to match your project name.
      default = craneLib.buildPackage (commonArgs // { pname = "rust-template"; });
    });

    # ============================================================================
    # APPS
    # ============================================================================
    apps = forAllSystems (system: let
      pkgs = pkgsFor system;
    in pkgs.lib.mapAttrs (key: crate: {
      type = "app";
      program = "${self.packages.${system}.${key}}/bin/${crate.binary}";
    }) workspaceCrates);

    # ============================================================================
    # OVERLAYS
    # ============================================================================
    # Uncomment to expose your packages as an overlay
    # ============================================================================
    # overlays.default = final: prev:
    #   pkgs.lib.mapAttrs' (key: crate:
    #     pkgs.lib.nameValuePair crate.name self.packages.${final.system}.${key}
    #   ) workspaceCrates;

  };

}
