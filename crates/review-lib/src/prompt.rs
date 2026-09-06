//! The reviewer's instructions, built into the binary so the tree under review
//! cannot change them.

/// The system prompt handed to the reviewer.  It lives in a file beside this
/// module so it reads and diffs as prose.  Crane's source filter keeps Cargo
/// sources alone and would drop the file, which the compiler then fails to
/// find; the flake's workspace source is told to keep it.
pub const REVIEWER: &str = include_str!("prompt.md");
