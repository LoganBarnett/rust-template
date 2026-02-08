# Error Handling Philosophy

This template uses **semantic error types** with `thiserror` rather than generic error wrapping with `anyhow`.

## Why Not `anyhow`?

While `anyhow` is convenient, it encourages lazy error handling that loses important context. Consider:

```rust
// BAD - Using anyhow blindly
fn load_config() -> anyhow::Result<Config> {
    let contents = std::fs::read_to_string("config.toml")?;  // What file? Why did it fail?
    let config = toml::from_str(&contents)?;  // What was wrong with the TOML?
    Ok(config)
}
```

When this fails, you get a generic error like "No such file or directory" with no context about:
- **What** file was being read
- **When** in the application lifecycle this happened (startup? runtime?)
- **Why** it matters (configuration is required for startup)

This is no better than a Python program that blows up with a stack trace.

## The Right Way: Semantic Error Types

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to read configuration file at {path:?} during startup: {source}")]
    FileRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to parse configuration file at {path:?}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("Configuration validation failed: {0}")]
    Validation(String),
}

fn load_config() -> Result<Config, ConfigError> {
    let path = PathBuf::from("config.toml");

    let contents = std::fs::read_to_string(&path)
        .map_err(|source| ConfigError::FileRead {
            path: path.clone(),
            source,
        })?;

    let config = toml::from_str(&contents)
        .map_err(|source| ConfigError::Parse {
            path: path.clone(),
            source,
        })?;

    Ok(config)
}
```

Now when it fails, you get:
```
Failed to read configuration file at "config.toml" during startup: Permission denied (os error 13)
```

This tells you:
- **What**: The configuration file at "config.toml"
- **When**: During startup
- **Why**: Permission denied
- **How to fix**: Check file permissions

## Error Type Hierarchy

Structure your error types in layers:

1. **Low-level errors** (in libraries/modules) - specific to that component
2. **Mid-level errors** (in business logic) - domain-specific
3. **Top-level errors** (in main.rs) - application lifecycle errors

Example from this template:

```
ApplicationError::ConfigurationLoad
    └─ ConfigError::FileRead
        └─ std::io::Error (Permission denied)
```

Each layer adds context about what the application was trying to do.

## Guidelines

1. **Every error variant should explain context**
   - What operation failed
   - Why it matters
   - Include relevant data (file paths, addresses, etc.)

2. **Use `#[source]` for error chains**
   - Preserves the full error chain
   - Allows inspection of root causes
   - Works with error reporting libraries

3. **Be specific in error messages**
   ```rust
   // BAD
   #[error("File error: {0}")]
   FileError(std::io::Error),

   // GOOD
   #[error("Failed to read configuration file at {path:?} during application startup: {source}")]
   ConfigFileRead {
       path: PathBuf,
       #[source]
       source: std::io::Error,
   },
   ```

4. **Handle errors at the right level**
   - Don't catch errors too early (loses context)
   - Don't let errors bubble up without adding context
   - Transform errors to add domain-specific context

5. **Document expected errors**
   ```rust
   /// Loads configuration from the specified path.
   ///
   /// # Errors
   ///
   /// Returns `ConfigError::FileRead` if the file cannot be read (missing file,
   /// permission denied, etc.)
   ///
   /// Returns `ConfigError::Parse` if the file contents are not valid TOML.
   pub fn load_config(path: &Path) -> Result<Config, ConfigError> {
       // ...
   }
   ```

## When `anyhow` Might Be Acceptable

The only time `anyhow` is acceptable is for **quick prototypes** or **throwaway scripts** where you're iterating rapidly. Once you know what errors can occur, convert to semantic error types.

Even in main(), we use semantic error types because the user deserves informative error messages.

## Testing Error Messages

Write tests that verify error messages contain the expected context:

```rust
#[test]
fn test_missing_config_file_error() {
    let result = Config::from_file(Path::new("nonexistent.toml"));

    let err = result.unwrap_err();
    let msg = err.to_string();

    // Verify the error message includes context
    assert!(msg.contains("nonexistent.toml"));
    assert!(msg.contains("startup") || msg.contains("configuration"));
}
```

## Additional Resources

- [thiserror documentation](https://docs.rs/thiserror/)
- [Error Handling in Rust](https://doc.rust-lang.org/book/ch09-00-error-handling.html)
- [Rust Error Handling Survey](https://blog.burntsushi.net/rust-error-handling/)
