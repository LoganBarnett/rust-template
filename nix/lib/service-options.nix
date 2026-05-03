# Shared option definitions for NixOS and Darwin service modules.
#
# Both mkNixosService and mkDarwinService import this to declare the
# common option schema.  Platform-specific options (uid/gid, healthCheck)
# are added by each platform helper.
#
# Parameters:
#   name  — service name (e.g. "my-app-server"), used as the options
#           key under services.<name> and for default paths/descriptions.
#   self  — the flake's self reference, used for the default package.
#   cfg   — the resolved config for this service (config.services.<name>).
#   lib   — nixpkgs lib.
#   pkgs  — nixpkgs package set.
{
  name,
  self,
  cfg,
  lib,
  pkgs,
}: {
  enable = lib.mkEnableOption "${name} service";

  package = lib.mkOption {
    type = lib.types.package;
    default = self.packages.${pkgs.stdenv.hostPlatform.system}.server;
    defaultText = lib.literalExpression "self.packages.\${system}.server";
    description = "Package providing the service binary.";
  };

  socket = lib.mkOption {
    type = lib.types.nullOr lib.types.path;
    description = ''
      Path for the Unix domain socket used by the service.  When set,
      the host/port options are ignored.  Set to null to use TCP instead.

      Other services (e.g. nginx) that proxy to this socket must be
      members of the service group to connect.
    '';
  };

  # host and port are separate options (rather than a single "listen"
  # string) so that other Nix expressions can reference them
  # individually — e.g. firewall rules need the port, reverse proxy
  # configs need host:port, and health-check URLs need both.  The
  # module combines them into the --listen flag internally.
  host = lib.mkOption {
    type = lib.types.str;
    default = "127.0.0.1";
    description = "IP address to bind to.  Ignored when socket is set.";
  };

  port = lib.mkOption {
    type = lib.types.port;
    default = 3000;
    description = "TCP port to listen on.  Ignored when socket is set.";
  };

  logLevel = lib.mkOption {
    type = lib.types.enum ["trace" "debug" "info" "warn" "error"];
    default = "info";
    description = "Tracing log verbosity level.";
  };

  logFormat = lib.mkOption {
    type = lib.types.enum ["text" "json"];
    default = "json";
    description = ''
      Log output format.  Use "text" for human-readable local logs and
      "json" for structured logs consumed by a log aggregator.
    '';
  };

  frontendPath = lib.mkOption {
    type = lib.types.str;
    default = "${cfg.package}/share/${name}/frontend";
    defaultText =
      lib.literalExpression
      ''"''${cfg.package}/share/${name}/frontend"'';
    description = "Path to compiled frontend static assets.";
  };

  baseUrl = lib.mkOption {
    type = lib.types.str;
    example = "https://example.com";
    description = ''
      Public base URL of the service, used to construct the OIDC redirect
      URI ("<baseUrl>/auth/callback").
    '';
  };

  oidcIssuer = lib.mkOption {
    type = lib.types.nullOr lib.types.str;
    default = null;
    example = "https://sso.example.com/application/o/my-app";
    description = ''
      OIDC issuer URL used for provider discovery.  Set all three OIDC
      options or leave all three null for unauthenticated admin mode.
    '';
  };

  oidcClientId = lib.mkOption {
    type = lib.types.nullOr lib.types.str;
    default = null;
    description = ''
      OIDC client ID.  Set all three OIDC options or leave all three
      null for unauthenticated admin mode.
    '';
  };

  oidcClientSecretFile = lib.mkOption {
    type = lib.types.nullOr lib.types.path;
    default = null;
    description = ''
      Path to a file containing the OIDC client secret.  Set all three
      OIDC options or leave all three null for unauthenticated admin
      mode.
    '';
  };

  user = lib.mkOption {
    type = lib.types.str;
    description = "System user account the service runs as.";
  };

  group = lib.mkOption {
    type = lib.types.str;
    description = "System group the service runs as.";
  };
}
