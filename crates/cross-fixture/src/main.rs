//! Regression fixture for `mkDarwinCrossPackages`.  It deliberately exercises
//! the two darwin cross-compile paths the template's own crates do not: a
//! C-compiling dependency (`ring`, whose build script compiles C/assembly in
//! crane's deps-only phase) and Apple framework linking (`CoreFoundation`,
//! resolved against the SDK's `.tbd` stubs).  The crate is never published or
//! shipped; CI cross-builds it so a regression in either path fails the build.
//! See the "mkDarwinCrossPackages fails for workspaces with C-compiling deps"
//! entry in tasks.org.

use tap::Tap as _;

// A CoreFoundation symbol.  Referencing it from `report` forces the linker to
// resolve the framework, which needs the SDK's `.tbd` stub — the appleSdk path
// under test.
#[cfg(target_os = "macos")]
extern "C" {
  fn CFAbsoluteTimeGetCurrent() -> f64;
}

// Reference `ring` (the C-compiling dependency) and a CoreFoundation symbol
// (the framework link) so neither is pruned from the build graph before
// linking.
#[cfg(target_os = "macos")]
fn report() -> String {
  let digest = ring::digest::digest(&ring::digest::SHA256, b"cross-fixture");
  // SAFETY: CFAbsoluteTimeGetCurrent takes no arguments and returns a
  // CFTimeInterval (a plain f64); there is nothing to misuse.
  let now = unsafe { CFAbsoluteTimeGetCurrent() };
  format!("darwin: {} digest bytes at t={now}", digest.as_ref().len())
}

#[cfg(not(target_os = "macos"))]
fn report() -> String {
  "non-darwin: nothing to cross-link".to_string()
}

fn main() {
  println!(
    "{}",
    report().tap(|message| eprintln!("cross-fixture built: {message}"))
  );
}
