//! Tests for `MergeConfig`'s `subcommand` field kind.
//!
//! These exercise the macro's generated `CliRaw` (clap parsing) and
//! `from_cli_and_file` (passthrough into the resolved `Config`).
//!
//! Each `MergeConfig` derive generates a `CliRaw`, `ConfigFileRaw`, and
//! `ConfigError` in its enclosing scope, so the two test fixtures live
//! in separate modules.

use clap::Parser;
use rust_template_foundation::logging::{LogFormat, LogLevel};
use rust_template_foundation::MergeConfig;

#[derive(Debug, Clone, clap::Args, PartialEq)]
struct PrintArgs {
  #[arg(long)]
  message: String,
}

#[derive(Debug, Clone, clap::Args, PartialEq)]
struct PlayArgs {
  #[arg(long)]
  track: String,
}

#[derive(Debug, Clone, clap::Subcommand, PartialEq)]
enum Commands {
  Print(PrintArgs),
  Play(PlayArgs),
  PlayContinuous(PlayArgs),
}

mod required_subcommand {
  use super::*;

  #[derive(Debug, Clone, MergeConfig)]
  #[merge_config(app_name = "subcommand-test")]
  pub struct Config {
    #[merge_config(common)]
    pub log_level: LogLevel,
    #[merge_config(common)]
    pub log_format: LogFormat,
    #[merge_config(default = "\"World\".to_string()")]
    pub name: String,
    #[merge_config(subcommand)]
    pub command: Commands,
  }

  #[test]
  fn parses_print_subcommand() {
    let cli =
      CliRaw::try_parse_from(["subcommand-test", "print", "--message", "hi"])
        .unwrap();
    let config = Config::from_cli_and_file(cli).unwrap();
    assert_eq!(
      config.command,
      Commands::Print(PrintArgs {
        message: "hi".to_string()
      }),
    );
    assert_eq!(config.name, "World");
  }

  #[test]
  fn parses_play_subcommand_with_top_level_flag() {
    let cli = CliRaw::try_parse_from([
      "subcommand-test",
      "--name",
      "Alice",
      "play",
      "--track",
      "song.mp3",
    ])
    .unwrap();
    let config = Config::from_cli_and_file(cli).unwrap();
    assert_eq!(config.name, "Alice");
    assert_eq!(
      config.command,
      Commands::Play(PlayArgs {
        track: "song.mp3".to_string()
      }),
    );
  }

  #[test]
  fn missing_subcommand_is_error() {
    // `command: Commands` (not `Option<Commands>`) makes the
    // subcommand required.
    let result = CliRaw::try_parse_from(["subcommand-test"]);
    assert!(result.is_err());
  }
}

mod optional_subcommand {
  use super::*;

  #[derive(Debug, Clone, MergeConfig)]
  #[merge_config(app_name = "subcommand-test-opt")]
  pub struct Config {
    #[merge_config(common)]
    pub log_level: LogLevel,
    #[merge_config(common)]
    pub log_format: LogFormat,
    #[merge_config(subcommand)]
    pub command: Option<Commands>,
  }

  #[test]
  fn absent() {
    let cli = CliRaw::try_parse_from(["subcommand-test-opt"]).unwrap();
    let config = Config::from_cli_and_file(cli).unwrap();
    assert_eq!(config.command, None);
  }

  #[test]
  fn present() {
    let cli = CliRaw::try_parse_from([
      "subcommand-test-opt",
      "print",
      "--message",
      "hello",
    ])
    .unwrap();
    let config = Config::from_cli_and_file(cli).unwrap();
    assert_eq!(
      config.command,
      Some(Commands::Print(PrintArgs {
        message: "hello".to_string()
      })),
    );
  }
}
