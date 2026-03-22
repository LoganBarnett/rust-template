# Crane-based derivation for the rust-template-web binary.
# Called from flake.nix with: import ./web.nix { inherit craneLib commonArgs; }
{ craneLib, commonArgs }:
craneLib.buildPackage (commonArgs // {
  pname = "rust-template-web";
  cargoExtraArgs = "-p rust-template-web";
})
