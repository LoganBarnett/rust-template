//! Regression fixture for the zig-linked cross/portable build helpers.  It
//! deliberately exercises the link paths the template's own crates do not:
//!
//! - `mkDarwinCrossPackages` — a C-compiling dependency (`ring`, whose build
//!   script compiles C/assembly in crane's deps-only phase), Apple framework
//!   linking (`CoreFoundation`, resolved against the SDK's `.tbd` stubs), and
//!   foundation built with its `auth` feature — the JWKS/OIDC TLS stack
//!   (`axum-jwt-auth` + `reqwest` + `rustls`) whose rustls provider must stay
//!   ring, not the aws-lc-rs C library (`aws-lc-sys`) that panics under zig
//!   cross-compilation to darwin.  All gated to macos targets.
//! - `mkGnuPortablePackages` — linking a nixpkgs shared library built against a
//!   modern glibc (`libasound`).  Its undefined references to post-2.17 glibc
//!   symbols are what the helper's `--allow-shlib-undefined` link flag has to
//!   tolerate.  The link is gated on the `link_asound` cfg, which `build.rs`
//!   emits only when the gnu-portable fixture build points it at a libasound
//!   directory (see `build.rs`).  It is a cfg rather than a Cargo dependency on
//!   purpose: an `alsa-sys` workspace dependency would be active on the Linux CI
//!   host for *every* workspace build (`cargo test`, `cargo clippy`, crane's
//!   shared deps-only build), all of which run without alsa in their
//!   environment and would fail on its pkg-config probe.  Routing the link
//!   through `build.rs` keeps libasound confined to the one build that supplies
//!   it.
//! - `mkWindowsCrossPackages` / `mkWindowsMsvcCrossPackages` — a C-compiling
//!   dependency (`ring`, again gated so it compiles in crane's deps-only phase)
//!   to exercise the Windows C toolchain, a link against a *non-default* Win32
//!   import library (`winmm`, emitted by `build.rs`), and the foundation crate
//!   built with its `server` feature — the server archetype's cross-compile
//!   shape (Unix-only systemd stubbed out, tokio-listener, axum, the embedded
//!   frontend, …) — so a toolchain regression or a Unix-only dependency slipped
//!   into the server feature fails here rather than only when a real server is
//!   cross-compiled.  All gated to windows targets, so no other build pulls
//!   them.
//!
//! The crate is never published or shipped; CI cross-builds it so a regression
//! in any of these paths fails the build.  See the "mkDarwinCrossPackages fails
//! for workspaces with C-compiling deps" and the mkGnuPortablePackages
//! shared-library-link entries in tasks.org.

use tap::Tap as _;

// A CoreFoundation symbol.  Referencing it from `report` forces the linker to
// resolve the framework, which needs the SDK's `.tbd` stub — the appleSdk path
// under test.
#[cfg(target_os = "macos")]
extern "C" {
  fn CFAbsoluteTimeGetCurrent() -> f64;
}

// Reference `ring` (the C-compiling dependency), a CoreFoundation symbol (the
// framework link), and foundation's `auth` JWKS decoder builder (the TLS/OIDC
// cross-compile shape) so none is pruned from the build graph before linking.
#[cfg(target_os = "macos")]
fn report() -> String {
  let digest = ring::digest::digest(&ring::digest::SHA256, b"cross-fixture");
  // SAFETY: CFAbsoluteTimeGetCurrent takes no arguments and returns a
  // CFTimeInterval (a plain f64); there is nothing to misuse.
  let now = unsafe { CFAbsoluteTimeGetCurrent() };
  // Monomorphise foundation's JWKS decoder builder so the reqwest/rustls auth
  // TLS stack is compiled and linked for the darwin target — the stack whose
  // rustls provider must resolve to ring, never the aws-lc-rs C library that
  // breaks this zig cross-build.  Held by reference through a black box so the
  // optimiser cannot prune it; never called — the darwin fixture is built and
  // signed, never run, so no crypto provider is registered.
  let build_decoder = &rust_template_foundation::auth::jwt::build_decoder::<
    rust_template_foundation::auth::jwt::ServiceClaims,
  >;
  std::hint::black_box(build_decoder);
  format!("darwin: {} digest bytes at t={now}", digest.as_ref().len())
}

// A libasound symbol, declared locally so no `alsa-sys` dependency enters the
// workspace graph (see the module comment).  `build.rs` supplies the
// `-lasound` link and the `link_asound` cfg together, so this extern is only
// compiled when libasound is actually on the link line.
#[cfg(link_asound)]
extern "C" {
  fn snd_asoundlib_version() -> *const std::os::raw::c_char;
}

// Reference the libasound symbol so the linker must resolve libasound.so — the
// nixpkgs shared library whose modern-glibc undefined symbols (lstat64@2.33,
// dlsym@2.34, …) are what mkGnuPortablePackages' --allow-shlib-undefined has to
// tolerate.  Without a live reference the linker's --as-needed pass would drop
// the unused library and skip the undefined-symbol check, guarding nothing.
#[cfg(all(link_asound, not(target_os = "macos")))]
fn report() -> String {
  // SAFETY: snd_asoundlib_version takes no arguments and returns a pointer to a
  // static, NUL-terminated version string owned by libasound; we only read it,
  // never free or mutate it.
  let version = unsafe { std::ffi::CStr::from_ptr(snd_asoundlib_version()) };
  format!("gnu-portable: libasound {}", version.to_string_lossy())
}

// A winmm symbol.  winmm is not linked by the windows target by default, so
// referencing it forces the linker to resolve winmm's import library from
// llvm-mingw's mingw-w64 tree — the appleSdk-equivalent path under test for
// Windows.  `build.rs` emits the `-lwinmm` link only for windows targets.
#[cfg(target_os = "windows")]
extern "system" {
  fn timeGetTime() -> u32;
}

// Reference `ring` (the C-compiling dependency, compiled here via llvm-mingw's
// clang), the winmm symbol (the import-library link), and foundation's server
// crate so none is pruned from the build graph before linking.  `notify_ready`
// is foundation's systemd readiness call; on Windows it is the no-op stub
// (systemd is Unix-only), so invoking it is safe under the wine smoke test and
// merely forces the linker to pull the foundation server code — the whole
// server cross-compile shape this fixture guards.
#[cfg(target_os = "windows")]
fn report() -> String {
  rust_template_foundation::server::systemd::notify_ready();
  let digest = ring::digest::digest(&ring::digest::SHA256, b"cross-fixture");
  // SAFETY: timeGetTime takes no arguments and returns a DWORD millisecond
  // tick count; there is nothing to misuse.
  let ticks = unsafe { timeGetTime() };
  format!("windows: {} digest bytes at {ticks} ticks", digest.as_ref().len())
}

#[cfg(all(
  not(target_os = "macos"),
  not(target_os = "windows"),
  not(link_asound)
))]
fn report() -> String {
  "other: nothing to cross-link".to_string()
}

fn main() {
  println!(
    "{}",
    report().tap(|message| eprintln!("cross-fixture built: {message}"))
  );
}
