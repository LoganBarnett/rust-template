# mkNixosService — generate a NixOS (systemd) module for a service.
#
# Usage in a spawned project's flake.nix:
#
#   nixosModules.server = inputs.foundation.lib.mkNixosService {
#     name = "my-app-server";
#     self = self;
#   };
#
# Then in a NixOS configuration:
#
#   imports = [ inputs.my-app.nixosModules.server ];
#
#   services.my-app-server = {
#     enable = true;
#     baseUrl = "https://my-app.example.com";
#   };
#
# Generates: systemd service (Type=notify, watchdog), socket unit
# (optional), tmpfiles rules, user/group, OIDC credential plumbing,
# and hardening.
{
  name,
  self,
}: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.${name};
  sharedOptions = import ./service-options.nix {
    inherit name self cfg lib pkgs;
  };
  # Env-var prefix derived from the service name, matching the Rust
  # MergeConfig macro's derivation: lowercase, with hyphens turned into
  # underscores (POSIX env-var names are restricted to letters, digits,
  # and underscore).  See crates/foundation/USAGE.org → "Environment
  # variables" for the naming rule and POSIX §8.1 citation.
  envPrefix = lib.replaceStrings ["-"] ["_"] (lib.toLower name);
in {
  options.services.${name} =
    sharedOptions
    // {
      socket = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = "/run/${name}/${name}.sock";
        description = ''
          Path for the Unix domain socket used by the service.  When set,
          systemd socket activation is used and the host/port options are
          ignored.  Set to null to use TCP instead.

          Other services (e.g. nginx) that proxy to this socket must be
          members of the service group to connect.
        '';
      };

      user = lib.mkOption {
        type = lib.types.str;
        default = name;
        description = "System user account the service runs as.";
      };

      group = lib.mkOption {
        type = lib.types.str;
        default = name;
        description = "System group the service runs as.";
      };

      oidcClientSecretFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = ''
          Path to a file containing the OIDC client secret.  The module
          loads this via systemd's LoadCredential, so the service user
          does not need direct read access to the file.  Set all three
          OIDC options or leave all three null for unauthenticated admin
          mode.
        '';
      };
    };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = let
          oidcFields = [cfg.oidcIssuer cfg.oidcClientId cfg.oidcClientSecretFile];
          setCount = lib.count (x: x != null) oidcFields;
        in
          setCount == 0 || setCount == 3;
        message = ''
          services.${name}: OIDC configuration is partial.
          Set all three of oidcIssuer, oidcClientId, and oidcClientSecretFile,
          or leave all three null for unauthenticated admin mode.
        '';
      }
    ];

    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      description = "${name} service user";
    };

    users.groups.${cfg.group} = {};

    # Create the socket directory before the socket unit tries to bind.
    systemd.tmpfiles.rules =
      lib.mkIf (cfg.socket != null)
      ["d ${dirOf cfg.socket} 0750 ${cfg.user} ${cfg.group} -"];

    # Socket unit: systemd creates and holds the Unix domain socket,
    # then passes the open file descriptor to the service on first
    # activation.
    systemd.sockets.${name} = lib.mkIf (cfg.socket != null) {
      description = "${name} Unix domain socket";
      wantedBy = ["sockets.target"];
      # Scalar socketConfig entries are wrapped in lib.mkDefault so a
      # downstream module can override individual fields with plain
      # assignment.  Lists (wantedBy) are intentionally left plain so
      # the NixOS module merger concatenates additions instead of
      # treating downstream's list as a replacement.
      socketConfig = {
        ListenStream = lib.mkDefault cfg.socket;
        SocketUser = lib.mkDefault cfg.user;
        SocketGroup = lib.mkDefault cfg.group;
        # 0660: accessible to the service user and group only.  Add
        # the reverse proxy user to cfg.group to grant it access.
        SocketMode = lib.mkDefault "0660";
        Accept = lib.mkDefault false;
      };
    };

    systemd.services.${name} = {
      description = "${name} service";
      wantedBy = ["multi-user.target"];
      after =
        ["network.target"]
        ++ lib.optional (cfg.socket != null) "${name}.socket";
      requires =
        lib.optional (cfg.socket != null) "${name}.socket";

      # Per-key lib.mkDefault on environment.  The attrset itself is
      # plain so downstream modules contributing new keys merge
      # additively; existing keys remain overridable without mkForce.
      environment = lib.mapAttrs (_: lib.mkDefault) (
        {
          "${envPrefix}_log_level" = cfg.logLevel;
          "${envPrefix}_log_format" = cfg.logFormat;
          "${envPrefix}_base_url" = cfg.baseUrl;
        }
        // lib.optionalAttrs (cfg.oidcIssuer != null) {
          "${envPrefix}_oidc_issuer" = cfg.oidcIssuer;
          "${envPrefix}_oidc_client_id" = cfg.oidcClientId;
        }
      );

      # Every scalar serviceConfig value is wrapped in lib.mkDefault so
      # downstream modules — for example, ones that need to inject a
      # --config flag into ExecStart or pin a different User — can
      # override with plain assignment instead of reaching for mkForce.
      serviceConfig = {
        # Type = notify causes systemd to wait for the binary to call
        # sd_notify(READY=1) before marking the unit active.  The
        # binary does this via the sd-notify crate immediately after
        # the listener is bound.  NotifyAccess = main restricts who
        # may send notifications to the main process only.
        Type = lib.mkDefault "notify";
        NotifyAccess = lib.mkDefault "main";

        # Restart if no WATCHDOG=1 heartbeat arrives within 30 s.
        # The binary reads WATCHDOG_USEC and pings at half this
        # interval (15 s).
        WatchdogSec = lib.mkDefault "30s";

        ExecStart = lib.mkDefault (
          "${cfg.package}/bin/${name}"
          + (
            if cfg.socket != null
            then " --listen sd-listen"
            else " --listen ${cfg.host}:${toString cfg.port}"
          )
          + " --frontend-path ${cfg.frontendPath}"
        );

        LoadCredential =
          lib.mkIf (cfg.oidcClientSecretFile != null)
          (lib.mkDefault "oidc-client-secret:${cfg.oidcClientSecretFile}");

        User = lib.mkDefault cfg.user;
        Group = lib.mkDefault cfg.group;
        Restart = lib.mkDefault "on-failure";
        RestartSec = lib.mkDefault "5s";

        # Harden the service environment.
        NoNewPrivileges = lib.mkDefault true;
        PrivateTmp = lib.mkDefault true;
        ProtectSystem = lib.mkDefault "strict";
        ProtectHome = lib.mkDefault true;
      };
    };
  };
}
