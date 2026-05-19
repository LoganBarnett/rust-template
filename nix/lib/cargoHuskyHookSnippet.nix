# cargoHuskyHookSnippet — shell snippet that symlinks cargo-husky hooks
# from `.cargo-husky/hooks/` into `.git/hooks/` when the devShell is
# entered from the repository root.
#
# Returned as a string suitable for splicing into a `mkShell`
# `shellHook` via antiquotation:
#
#   shellHook = ''
#     ${foundation.lib.cargoHuskyHookSnippet pkgs}
#     echo "welcome to my project"
#   '';
#
# The symlink target is computed with `realpath --relative-to` so the
# resulting link survives moves of the repository.  We only install the
# link when the working directory matches the git toplevel — direnv
# subshells entered from subdirectories are no-ops, which avoids
# repeated "Installed git hook" noise in unrelated terminals.
pkgs: ''
  _git_root=$(git rev-parse --show-toplevel 2>/dev/null)
  if [ -n "$_git_root" ] \
      && [ "$(pwd)" = "$_git_root" ] \
      && [ -d ".cargo-husky/hooks" ]; then
    for _hook in .cargo-husky/hooks/*; do
      [ -x "$_hook" ] || continue
      _name=$(basename "$_hook")
      _dest="$_git_root/.git/hooks/$_name"
      _target=$(${pkgs.coreutils}/bin/realpath \
        --relative-to="$_git_root/.git/hooks" "$(pwd)/$_hook")
      if [ ! -L "$_dest" ] \
          || [ "$(readlink "$_dest")" != "$_target" ]; then
        ln -sf "$_target" "$_dest"
        echo "Installed git hook: $_name -> $_target"
      fi
    done
  fi
''
