# Customizing the Template for Your Project

This guide explains how to adapt this template for your specific project.

## Quick Rename Guide

### 1. Update Project Names

Replace `rust-template` with your project name throughout:

```bash
# Example: renaming to "my-app"
PROJECT_NAME="my-app"

# Update Rust code and configs
find . -type f \( -name "*.toml" -o -name "*.rs" -o -name "*.md" \) \
  -not -path "*/target/*" \
  -exec sed -i '' "s/rust-template/${PROJECT_NAME}/g" {} +

find . -type f \( -name "*.toml" -o -name "*.rs" -o -name "*.md" \) \
  -not -path "*/target/*" \
  -exec sed -i '' "s/rust_template/${PROJECT_NAME//-/_}/g" {} +
```

### 2. Update Cargo.toml Metadata

Edit the workspace `Cargo.toml`:

```toml
[workspace.package]
version = "0.1.0"
edition = "2021"
authors = ["Your Name <your.email@example.com>"]
license = "MIT OR Apache-2.0"  # or your preferred license
repository = "https://github.com/yourusername/your-project"
```

### 3. Update flake.nix Workspace Crates

Edit `flake.nix` and update the `workspaceCrates` section:

```nix
workspaceCrates = {
  # CLI application
  cli = {
    name = "my-app-cli";           # Cargo package name
    binary = "my-app-cli";         # Binary executable name
    description = "CLI application";
  };

  # Web service
  web = {
    name = "my-app-web";
    binary = "my-app-web";
    description = "Web service";
  };

  # Add more crates as needed
  # worker = {
  #   name = "my-app-worker";
  #   binary = "my-app-worker";
  #   description = "Background worker";
  # };
};
```

**Why maintain this list?**

The `workspaceCrates` configuration in `flake.nix` is the **single source of truth** for:
- Nix package generation (when uncommented)
- App generation (`nix run .#cli`, `nix run .#web`)
- Overlay generation for NixOS modules
- Consistent naming across the flake

This prevents repetitive boilerplate and makes it easy to add/remove workspace members.

### 4. Rename Directories

```bash
# If you want to rename the crate directories too:
mv crates/cli "crates/${PROJECT_NAME}-cli"
mv crates/web "crates/${PROJECT_NAME}-web"
mv crates/lib "crates/${PROJECT_NAME}-lib"

# Update Cargo.toml workspace members accordingly
```

## Adding New Workspace Crates

### Example: Adding a Background Worker

1. **Create the crate:**

```bash
cargo new --bin crates/worker
```

2. **Add to workspace `Cargo.toml`:**

```toml
[workspace]
members = [
    "crates/cli",
    "crates/lib",
    "crates/web",
    "crates/worker",  # New!
]
```

3. **Update package Cargo.toml:**

```toml
# crates/worker/Cargo.toml
[package]
name = "my-app-worker"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true
repository.workspace = true

[[bin]]
name = "my-app-worker"
path = "src/main.rs"

[dependencies]
my-app-lib = { workspace = true }
# ... other dependencies
```

4. **Add to `flake.nix` workspaceCrates:**

```nix
workspaceCrates = {
  cli = { ... };
  web = { ... };

  # Add the new worker
  worker = {
    name = "my-app-worker";
    binary = "my-app-worker";
    description = "Background worker";
  };
};
```

5. **Implement with the same patterns:**
   - Create `config.rs` with staged configuration types
   - Use semantic error types with `thiserror`
   - Add integration tests

## Removing Crates You Don't Need

### Example: Removing the Web Crate

1. **Remove from workspace:**

```toml
# Cargo.toml
[workspace]
members = [
    "crates/cli",
    "crates/lib",
    # "crates/web",  # Commented out or removed
]
```

2. **Remove from flake.nix:**

```nix
workspaceCrates = {
  cli = {
    name = "my-app-cli";
    binary = "my-app-cli";
    description = "CLI application";
  };

  # web = { ... };  # Removed
};
```

3. **Delete the directory:**

```bash
rm -rf crates/web
```

## Customizing Logging

### Adding Custom Log Fields

Edit `crates/lib/src/logging.rs` to add project-specific logging configuration:

```rust
pub struct LoggingConfig {
    pub level: LogLevel,
    pub format: LogFormat,
    pub service_name: String,      // Add custom fields
    pub environment: Environment,  // Add custom fields
}
```

### Adding More Log Formats

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Text,
    Json,
    Logfmt,  // Add new format
}
```

## Customizing Configuration

### Adding Application-Specific Config

Each application (`cli`, `web`) has its own `config.rs`. Add your settings there:

```rust
// In crates/cli/src/config.rs

#[derive(Debug)]
pub struct Config {
    pub log_level: LogLevel,
    pub log_format: LogFormat,

    // Add your application-specific config
    pub database_url: String,
    pub api_key: Option<String>,
    pub max_retries: u32,
}

impl Config {
    pub fn from_cli_and_file(cli: CliRaw) -> Result<Self, ConfigError> {
        // Merge CLI args, config file, and defaults
        // Validate and construct final Config
    }
}

```

Remember to update:
- `CliRaw` (for command-line arguments)
- `ConfigFileRaw` (for file-based config)
- `from_cli_and_file()` merge logic
- Semantic error types if validation fails

## Enabling Nix Package Building

When you're ready to build Nix packages, uncomment the packages section in `flake.nix`:

```nix
# Remove the # comments from the packages section
packages = forAllSystems (system: let
  pkgs = pkgsFor system;
  # ... rest of the configuration
```

Then build with:

```bash
# Build specific package
nix build .#cli
nix build .#web

# Build all packages
nix build .#default

# Run directly
nix run .#cli -- --help
nix run .#web -- --host 0.0.0.0 --port 8080
```

## Creating NixOS Modules

The web application uses separate `--host` and `--port` arguments (rather than a combined bind address) specifically to make it easier to create NixOS module options:

```nix
# Example NixOS module options
options.services.my-app = {
  enable = mkEnableOption "My App";

  host = mkOption {
    type = types.str;
    default = "127.0.0.1";
    description = "Host to bind to";
  };

  port = mkOption {
    type = types.port;
    default = 3000;
    description = "Port to listen on";
  };
};

# These map directly to CLI args
ExecStart = "${cfg.package}/bin/my-app-web --host ${cfg.host} --port ${toString cfg.port}";
```

This is cleaner than trying to parse or construct a combined address string in Nix.

## Adding Additional Dependencies

### Workspace-Level Dependencies

Add to workspace `Cargo.toml`:

```toml
[workspace.dependencies]
# Your new dependency
sqlx = { version = "0.7", features = ["postgres", "runtime-tokio-native-tls"] }
```

Then use in any package:

```toml
# crates/cli/Cargo.toml
[dependencies]
sqlx = { workspace = true }
```

### Package-Specific Dependencies

If only one crate needs it:

```toml
# crates/web/Cargo.toml
[dependencies]
specific-crate = "1.0"  # Not in workspace dependencies
```

## Next Steps

- Review `ERROR_HANDLING.org` for error handling patterns
- Review `GETTING_STARTED.md` for development workflow
- Set up your CI/CD pipeline
- Customize the README.org with your project details
- Add your business logic!

## Template Maintenance

If you want to pull updates from the template into your customized project:

1. Add the template as a remote:
   ```bash
   git remote add template <template-repo-url>
   ```

2. Fetch changes:
   ```bash
   git fetch template
   ```

3. Cherry-pick specific improvements:
   ```bash
   git cherry-pick <commit-hash>
   ```

4. Or merge carefully:
   ```bash
   git merge template/main --no-commit
   # Resolve conflicts, keeping your customizations
   ```
