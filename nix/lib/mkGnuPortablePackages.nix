# mkGnuPortablePackages — glibc-dynamic Linux binaries that run off the Nix
# store, for one system.
#
# The native package a Nix build produces is dynamically linked against
# nixpkgs' glibc, so its ELF interpreter (PT_INTERP) is a `/nix/store/…-glibc/
# ld-linux-*.so.2` path and its RUNPATH points into the store.  On any host
# without that exact store path — i.e. every non-Nix machine — the kernel
# cannot find the interpreter at `execve` time and the binary fails to start,
# before LD_LIBRARY_PATH is ever consulted.  That store-locked build is the
# right artifact *on* Nix (consumers build it from the flake or a binary
# cache), but it is useless as a release download.
#
# The musl variant (mkMuslPackages) solves this by linking everything
# statically — perfect for a self-contained tool, but wrong for one that must
# use the host's shared libraries.  A library with a runtime plugin/dlopen
# ecosystem (ALSA's libasound, PulseAudio, GL, CUDA, …) cannot be statically
# absorbed: libasound dlopens host PCM plugins by name from the host's config,
# and a musl-static process cannot load those glibc-built plugins — and mixing
# musl-static libc with a dynamically loaded glibc library drags two libcs into
# one process.  Such a tool needs a *dynamic* glibc binary that links the
# host's copies at runtime.
#
# This helper builds exactly that: a glibc-dynamic binary that (1) carries the
# conventional FHS interpreter `/lib64/ld-linux-*.so.2` instead of a store path,
# and (2) references no glibc symbol version newer than 2.17, so it runs on any
# mainstream distro (2.17 is the manylinux2014 / RHEL 7 baseline).  It does so
# by linking through zig rather than nixpkgs' wrapped `ld`: cargo-zigbuild
# targets `<triple>.2.17`, and because zig is not Nix-wrapped it emits the
# standard interpreter with no store RUNPATH — there is nothing to un-patch
# afterward.  The floor is decoupled from whatever glibc nixpkgs currently
# ships, so a nixpkgs bump never raises the minimum host glibc.
#
# The result is named `<name>-gnu`.  It links host shared libraries by soname
# (libc.so.6, libasound.so.2, …): those must be present and ABI-compatible on
# the host at runtime, which is the accepted trade of the shared-library
# platform.  zig floors *glibc* only; other host libraries rely on soname/ABI
# stability, so link a project against a not-bleeding-edge copy of them.
#
# Only Linux systems have a gnu target; for any other system the helper returns
# an empty set, so a caller can merge its result unconditionally — exactly like
# mkMuslPackages.  Like the musl build, it skips the test phase: the same
# sources are already exercised by the native build and the workspace test
# check in the same release, so this is a repackage for portability rather than
# new code to gate.
#
# It runs through mkRustPackages/crane just like mkMuslPackages and
# mkDarwinCrossPackages, only with the gnu target and the zig build command
# swapped in, so the caller's whole commonArgs is threaded — buildInputs,
# nativeBuildInputs, env, every crane knob — and crane still handles vendoring,
# the shared deps-only build, and installing the binary.
#
# Usage (mirrors mkMuslPackages):
#
#   packages =
#     rustPackages.packages
#     // mkMuslPackages {inherit self pkgs system crates crane commonArgs;}
#     // mkGnuPortablePackages {
#       inherit self pkgs system crates crane commonArgs;
#     }
#     // {default = ...;};
#
# Returns a single-system attrset of `<name>-gnu` packages (empty on non-Linux).
{
  # Flake `self` — forwarded to mkRustPackages for `cleanCargoSource` and
  # per-crate override resolution.
  self,
  # Per-system nixpkgs with the rust-overlay applied (for `rust-bin`).
  pkgs,
  # The Nix system being built for; selects the gnu target triple.
  system,
  # Workspace crate map, the same value passed to mkRustPackages.
  crates,
  # The crane flake input, used to build a gnu-targeted crane lib.
  crane,
  # The caller's crane commonArgs (such as buildInputs, nativeBuildInputs, or
  # env), threaded through so a project's native dependencies reach the portable
  # build the same way they reach mkRustPackages; the gnu target-specifics below
  # are overlaid on top.  Defaults to empty for callers that pass none.
  commonArgs ? {},
}: let
  lib = pkgs.lib;
  mkRustPackages = import ./mkRustPackages.nix;
  # The rustc gnu target triple for each Linux system.  Systems absent here have
  # no portable gnu variant.  cargo-zigbuild sees the same triple with a `.2.17`
  # glibc-version suffix appended (below); rustc itself sees the plain triple,
  # so this is also the target directory crane installs from.
  gnuTargetFor = {
    x86_64-linux = "x86_64-unknown-linux-gnu";
    aarch64-linux = "aarch64-unknown-linux-gnu";
  };
  # The lowest glibc symbol version the binary is allowed to reference.  2.17
  # (RHEL 7 / 2013) is the manylinux2014 baseline; every currently-supported
  # distro ships a newer glibc, which is backward-compatible with it.
  glibcFloor = "2.17";
in
  if !(gnuTargetFor ? ${system})
  then {}
  else let
    target = gnuTargetFor.${system};
    # cargo-zigbuild's versioned target selects the glibc floor for the Rust
    # link; the plain triple is what rustc/cargo and crane's install path use.
    zigbuildTarget = "${target}.${glibcFloor}";
    # zig's own target triple for C compilation, e.g. `x86_64-linux-gnu.2.17`.
    zigArch = lib.head (lib.splitString "-" target);
    zigCcTarget = "${zigArch}-linux-gnu.${glibcFloor}";
    # zig is the linker; the `.2.17` on --target sets the glibc floor for the
    # Rust link, and cargo-zigbuild strips it back to the plain triple for
    # cargo.
    buildCommand = "cargo zigbuild --release --target ${zigbuildTarget}";
    craneLib =
      (crane.mkLib pkgs).overrideToolchain
      (p: p.rust-bin.stable.latest.default.override {targets = [target];});
    # A dependency whose build script compiles C or assembly through the `cc`
    # crate does so in every cargo phase, not just the final `cargo zigbuild`.
    # cargo-zigbuild only sets the cc-crate cross vars for its own invocation,
    # so the other phases would fall back to the host `gcc` linked against
    # nixpkgs' newest glibc — reintroducing a symbol version above the floor
    # through the C dependency.  Pointing CC/CXX at zig-cc wrappers pinned to
    # the same `.2.17` target keeps the C toolchain, and its glibc floor,
    # identical in every phase.  These mirror the wrappers cargo-zigbuild writes
    # internally, and it honours ours because it only sets its own when unset.
    # zig is forced onto PATH so the wrapper works even under a build script
    # that scrubs it.  `-target` is single-dash deliberately: zig's `cc` driver
    # spells the target selector that way and offers no `--target` long form.
    zigCcArgs = "-target ${zigCcTarget}";
    zigCc = pkgs.writeShellScript "zigcc-${target}" ''
      export PATH="${pkgs.zig}/bin:$PATH"
      exec ${pkgs.cargo-zigbuild}/bin/cargo-zigbuild zig cc \
        -- ${zigCcArgs} "$@"
    '';
    zigCxx = pkgs.writeShellScript "zigcxx-${target}" ''
      export PATH="${pkgs.zig}/bin:$PATH"
      exec ${pkgs.cargo-zigbuild}/bin/cargo-zigbuild zig c++ \
        -- ${zigCcArgs} "$@"
    '';
    ccEnvTarget = lib.replaceStrings ["-"] ["_"] target;
    gnuArgs =
      commonArgs
      // {
        src = craneLib.cleanCargoSource self;
        # The plain triple: what rustc builds and where crane installs from
        # (`target/${target}/release`).  cargo-zigbuild strips the `.2.17` it
        # gets via --target down to this same triple for cargo, so the paths
        # line up.
        CARGO_BUILD_TARGET = target;
        # See the zigCc/zigCxx comment: route every phase's C compilation
        # through zig at the floored target, not just the final zigbuild.
        "CC_${ccEnvTarget}" = "${zigCc}";
        "CXX_${ccEnvTarget}" = "${zigCxx}";
        # A nixpkgs shared library handed in via buildInputs — libasound,
        # libpulse, libGL, … — is itself built against whatever glibc nixpkgs
        # currently ships, so it carries undefined references to symbols newer
        # than our 2.17 floor (lstat64@GLIBC_2.33, dlsym@GLIBC_2.34,
        # pow@GLIBC_2.29, __isoc23_strtoul@GLIBC_2.38, …).  Those are *not* this
        # binary's own glibc dependencies: they are resolved at runtime by the
        # host's glibc and the host's copy of that library, which is the entire
        # point of a portable-dynamic build linking host shared objects by
        # soname.  But zig's lld links executables with
        # --no-allow-shlib-undefined and so tries to satisfy a shared library's
        # own undefined symbols against our 2.17 stubs, cannot, and fails the
        # link.  --allow-shlib-undefined restores lld's tolerance for undefined
        # symbols that live in shared libraries (not in this crate's own
        # objects — those still error), which is exactly the
        # portable-against-host-library case this helper exists to serve.  Set
        # target-specifically so host build-script links keep the strict
        # default.  Matches mkDarwinCrossPackages' per-target RUSTFLAGS env var.
        "CARGO_TARGET_${lib.toUpper ccEnvTarget}_RUSTFLAGS" = "-Clink-arg=-Wl,--allow-shlib-undefined";
        # zig is the linker; cargo-zigbuild drives it (see buildCommand above).
        cargoBuildCommand = buildCommand;
        # The same sources are gated by the native build and the workspace test
        # check in this release; this is a portability repackage, not new code.
        doCheck = false;
        # cargo-zigbuild drives the build and link; zig is the linker and C
        # toolchain it invokes, and the source of the pinned glibc stubs.
        nativeBuildInputs =
          (commonArgs.nativeBuildInputs or [])
          ++ [pkgs.cargo-zigbuild pkgs.zig];
        # cargo-zigbuild caches under $HOME/.cache and zig under its own cache
        # dir; crane's HOME=/homeless-shelter is read-only, so point both at the
        # writable build tree.
        preBuild =
          (commonArgs.preBuild or "")
          + ''
            export HOME="$TMPDIR"
            export ZIG_GLOBAL_CACHE_DIR="$TMPDIR/zig-cache"
          '';
      };
    gnuPackages =
      (mkRustPackages {
        inherit self pkgs crates;
        craneLib = craneLib;
        commonArgs = gnuArgs;
      }).packages;
  in
    lib.mapAttrs'
    (name: pkg: lib.nameValuePair "${name}-gnu" pkg)
    gnuPackages
