// Wires the fixture's three host-link guards, each confined to the build that
// needs it so no plain workspace build pays for any:
//
// - macos: emit the CoreFoundation framework link, so the darwin-cross build
//   exercises the SDK's `.tbd` framework stubs and the linker's `-F` framework
//   search path — the appleSdk half of mkDarwinCrossPackages that the
//   template's own crates never touch.
//
// - windows: emit the winmm link, so the gnullvm cross build must resolve a
//   non-default Win32 system DLL's import library from llvm-mingw's mingw-w64
//   tree — the mkWindowsCrossPackages path that the template's own crates never
//   touch.  winmm is chosen because the windows target does not link it by
//   default, so the reference genuinely forces import-library resolution.
//
// - gnu-portable: when CROSS_FIXTURE_ASOUND_LIBDIR names a nixpkgs libasound
//   directory (set only by flake.nix's gnuPortableFixturePackages), emit the
//   `-lasound` link and a `link_asound` cfg so main.rs references a libasound
//   symbol.  This makes the `<name>-gnu` build link a modern-glibc host shared
//   library — the case that forced mkGnuPortablePackages'
//   --allow-shlib-undefined.  Routed through an env var rather than an
//   `alsa-sys` dependency so it stays out of the workspace graph and off the
//   Linux CI host's `cargo test`/`cargo clippy`/deps-only builds, which carry
//   no alsa.
fn main() {
  // Always register the cfg so its use in main.rs is a known — not an
  // unexpected — cfg under `-D warnings`.
  println!("cargo:rustc-check-cfg=cfg(link_asound)");

  if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
    println!("cargo:rustc-link-lib=framework=CoreFoundation");
  }

  if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
    println!("cargo:rustc-link-lib=dylib=winmm");
  }

  println!("cargo:rerun-if-env-changed=CROSS_FIXTURE_ASOUND_LIBDIR");
  if let Ok(libdir) = std::env::var("CROSS_FIXTURE_ASOUND_LIBDIR") {
    println!("cargo:rustc-link-search=native={libdir}");
    println!("cargo:rustc-link-lib=dylib=asound");
    println!("cargo:rustc-cfg=link_asound");
  }
}
