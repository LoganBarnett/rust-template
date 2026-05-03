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
      socketConfig = {
        ListenStream = cfg.socket;
        SocketUser = cfg.user;
        SocketGroup = cfg.group;
        # 0660: accessible to the service user and group only.  Add
        # the reverse proxy user to cfg.group to grant it access.
        SocketMode = "0660";
        Accept = false;
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

      environment =
        {
          LOG_LEVEL = cfg.logLevel;
          LOG_FORMAT = cfg.logFormat;
          BASE_URL = cfg.baseUrl;
        }
        // lib.optionalAttrs (cfg.oidcIssuer != null) {
          OIDC_ISSUER = cfg.oidcIssuer;
          OIDC_CLIENT_ID = cfg.oidcClientId;
        };

      serviceConfig = {
        # Type = notify causes systemd to wait for the binary to call
        # sd_notify(READY=1) before marking the unit active.  The
        # binary does this via the sd-notify crate immediately after
        # the listener is bound.  NotifyAccess = main restricts who
        # may send notifications to the main process only.
        Type = "notify";
        NotifyAccess = "main";

        # Restart if no WATCHDOG=1 heartbeat arrives within 30 s.
        # The binary reads WATCHDOG_USEC and pings at half this
        # interval (15 s).  Override via
        # systemd.services.<name>.serviceConfig.WatchdogSec.
        WatchdogSec = lib.mkDefault "30s";

        ExecStart =
          "${cfg.package}/bin/${name}"
          + (
            if cfg.socket != null
            then " --listen sd-listen"
            else " --listen ${cfg.host}:${toString cfg.port}"
          )
          + " --frontend-path ${cfg.frontendPath}";

        LoadCredential =
          lib.mkIf (cfg.oidcClientSecretFile != null)
          "oidc-client-secret:${cfg.oidcClientSecretFile}";

        User = cfg.user;
        Group = cfg.group;
        Restart = "on-failure";
        RestartSec = "5s";

        # Harden the service environment.
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
      };
    };
  };
}
