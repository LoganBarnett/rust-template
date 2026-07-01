// Emit the CoreFoundation framework link only for macos targets.  This makes
// the darwin-cross build exercise the SDK's `.tbd` framework stubs and the
// linker's `-F` framework search path — the appleSdk half of
// mkDarwinCrossPackages that the template's own crates never touch — while the
// native and static-musl builds, which have no such framework, are unaffected.
fn main() {
  if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
    println!("cargo:rustc-link-lib=framework=CoreFoundation");
  }
}
