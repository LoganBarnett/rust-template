# xwin-sdk — the Microsoft Visual C++ CRT and Windows SDK, repacked by xwin into
# a cross-compilation sysroot and vendored as a fixed-output derivation so the
# sandboxed windows-msvc build can link offline, with no Docker.
#
# This backs the *opt-in* MSVC path (mkWindowsMsvcCrossPackages).  The default
# Windows path is gnullvm (mkWindowsCrossPackages), which links no Microsoft
# code and needs none of this; MSVC is only for a dependency that requires the
# MSVC ABI.  xwin downloads Microsoft's CRT/SDK, which Microsoft's licence
# governs — and `--accept-license` (env XWIN_ACCEPT_LICENSE) is consumed only
# here, at the fetch step, never at compile time.  So a project that opts into
# MSVC accepts that licence visibly in its own flake by evaluating this
# derivation, exactly as the darwin cross build surfaces the Apple SDK's
# `allowUnfree` in the consumer's flake — the consent is never hidden in the
# foundation library.
#
# `--disable-symlinks` is deliberate and load-bearing.  By default xwin adds
# case-fixing symlinks (Windows.h ⇽ windows.h, kernel32.Lib ⇽ kernel32.lib) for
# case-sensitive filesystems.  Those symlinks (a) fail to even create on a
# case-insensitive macOS filesystem — the source and target differ only by case
# and cannot coexist (xwin issue #31) — and (b) would make the splat tree's
# content, and therefore this derivation's output hash, differ between build
# hosts.  Dropping them makes the tree byte-identical on every host, so one
# `outputHash` serves Linux and macOS alike.
#
# The casing the symlinks would have fixed is handled two ways instead.  For
# libraries: the SDK ships mixed-case names (kernel32.Lib, WinMM.Lib) but rustc
# and lld-link resolve import libraries by the lowercase name (kernel32.lib), so
# every library filename is lowercased below — that resolves identically on
# case-sensitive Linux and case-insensitive macOS, and the rename is
# host-independent (same result everywhere, so the hash stays stable).  For
# headers: they keep their original casing, and clang-cl's MS-compatibility mode
# resolves the case at compile time for the rare C dependency that includes an
# SDK header.
#
# The manifest version is pinned so the fetched SDK/CRT is reproducible; bump it
# and re-pin `outputHash` together on a deliberate SDK update.  Both target
# architectures are splatted from one fetch (`--arch x86_64,aarch64`), mirroring
# how one llvm-mingw tree serves both gnullvm arches.
#
# Returns a derivation whose tree is the splat sysroot, with a `crt/` subtree
# (`include`, `lib/<arch>`) and a `sdk/` subtree (`include/{ucrt,um,shared}`,
# `lib/{um,ucrt}/<arch>`).
{pkgs}:
pkgs.stdenvNoCC.mkDerivation {
  pname = "xwin-msvc-sdk";
  # The Visual Studio manifest channel version, pinned for reproducibility.
  version = "17";
  # xwin downloads, unpacks, and splats Microsoft's CRT/SDK — the whole point of
  # this derivation; the buildCommand below invokes it.
  nativeBuildInputs = [pkgs.xwin];
  # A fixed-output derivation: xwin fetches Microsoft packages over the network,
  # which only a FOD is permitted to do in the sandbox.  Because
  # `--disable-symlinks` leaves the tree symlink-free, its content is identical
  # on every build host, so this single recursive hash is host-independent.
  outputHashMode = "recursive";
  outputHashAlgo = "sha256";
  outputHash = "sha256-Gixx0Zkh0E5bCyEJXj780WUIlFfhxQ84/TsNjGoUivs=";
  # xwin fetches over HTTPS, so it needs the CA bundle.
  SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
  buildCommand = ''
    export HOME="$TMPDIR"
    mkdir --parents "$out"
    xwin \
      --accept-license \
      --manifest-version 17 \
      --arch x86_64,aarch64 \
      --variant desktop \
      --cache-dir "$TMPDIR/xwin-cache" \
      splat \
      --disable-symlinks \
      --output "$out"
    # Lowercase every import-library filename (see the header comment): the SDK
    # ships kernel32.Lib but rustc/lld-link ask for kernel32.lib, and without
    # the case-fixing symlinks the exact-case lookup fails on a case-sensitive
    # host.
    # Only `.lib` files are touched — the CRT `.obj` startup files already ship
    # lowercase, and the stray `.dll`s are not linked by name.  The rename goes
    # through a temporary name because a direct case-only `mv` is rejected on a
    # case-insensitive build host (macOS), where the two names are one file.
    find "$out/crt/lib" "$out/sdk/lib" -type f -iname '*.lib' | while read -r libfile; do
      lower="$(dirname "$libfile")/$(basename "$libfile" | tr '[:upper:]' '[:lower:]')"
      if [ "$libfile" != "$lower" ]; then
        mv "$libfile" "$libfile.lc-tmp"
        mv "$libfile.lc-tmp" "$lower"
      fi
    done
  '';
}
