# NixOS (Linux/systemd) module for the rust-template-daemon service.
# Exported from the flake as nixosModules.daemon.
# See darwin-daemon.nix for the macOS/launchd equivalent.
#
# Minimal usage (defaults to Unix domain socket):
#
#   inputs.rust-template.nixosModules.daemon
#
#   services.rust-template-daemon = {
#     enable = true;
#   };
#
# To use TCP instead:
#
#   services.rust-template-daemon = {
#     enable = true;
#     socket = null;
#     port   = 8080;
#   };
#
# To reference the socket from a reverse proxy (e.g. nginx):
#
#   locations."/".proxyPass =
#     "http://unix:${config.services.rust-template-daemon.socket}";
#
# Note: when using socket mode the reverse proxy user must be a member of
# the service group (cfg.group) so it can connect to the socket.
{self}: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.rust-template-daemon;
in {
  options.services.rust-template-daemon = {
    enable = lib.mkEnableOption "rust-template-daemon service";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.daemon;
      defaultText = lib.literalExpression "self.packages.\${system}.daemon";
      description = "Package providing the service binary.";
    };

    socket = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = "/run/rust-template-daemon/rust-template-daemon.sock";
      description = ''
        Path for the Unix domain socket used by the service.  When set,
        systemd socket activation is used and the host/port options are
        ignored.  Set to null to use TCP instead.

        Other services (e.g. nginx) that proxy to this socket must be
        members of the service group to connect.
      '';
    };

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
      default = "${cfg.package}/share/rust-template-daemon/frontend";
      defaultText =
        lib.literalExpression
        ''"''${cfg.package}/share/rust-template-daemon/frontend"'';
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
        Path to a file containing the OIDC client secret.  The module
        loads this via systemd's LoadCredential, so the service user
        does not need direct read access to the file.  Set all three
        OIDC options or leave all three null for unauthenticated admin
        mode.
      '';
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = "rust-template-daemon";
      description = "System user account the service runs as.";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "rust-template-daemon";
      description = "System group the service runs as.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [{
      assertion =
        let
          oidcFields = [ cfg.oidcIssuer cfg.oidcClientId cfg.oidcClientSecretFile ];
          setCount = lib.count (x: x != null) oidcFields;
        in
          setCount == 0 || setCount == 3;
      message = ''
        services.rust-template-daemon: OIDC configuration is partial.
        Set all three of oidcIssuer, oidcClientId, and oidcClientSecretFile,
        or leave all three null for unauthenticated admin mode.
      '';
    }];

    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      description = "rust-template-daemon service user";
    };

    users.groups.${cfg.group} = {};

    # Create the socket directory before the socket unit tries to bind.
    systemd.tmpfiles.rules = lib.mkIf (cfg.socket != null) [
      "d ${dirOf cfg.socket} 0750 ${cfg.user} ${cfg.group} -"
    ];

    # Socket unit: systemd creates and holds the Unix domain socket, then
    # passes the open file descriptor to the service on first activation.
    systemd.sockets.rust-template-daemon = lib.mkIf (cfg.socket != null) {
      description = "rust-template-daemon Unix domain socket";
      wantedBy = ["sockets.target"];
      socketConfig = {
        ListenStream = cfg.socket;
        SocketUser = cfg.user;
        SocketGroup = cfg.group;
        # 0660: accessible to the service user and group only.  Add the
        # reverse proxy user to cfg.group to grant it access.
        SocketMode = "0660";
        Accept = false;
      };
    };

    systemd.services.rust-template-daemon = {
      description = "rust-template-daemon service";
      wantedBy = ["multi-user.target"];
      after =
        ["network.target"]
        ++ lib.optional (cfg.socket != null) "rust-template-daemon.socket";
      requires =
        lib.optional (cfg.socket != null) "rust-template-daemon.socket";

      environment = {
        LOG_LEVEL = cfg.logLevel;
        LOG_FORMAT = cfg.logFormat;
        BASE_URL = cfg.baseUrl;
      } // lib.optionalAttrs (cfg.oidcIssuer != null) {
        OIDC_ISSUER = cfg.oidcIssuer;
        OIDC_CLIENT_ID = cfg.oidcClientId;
      };

      serviceConfig = {
        # Type = notify causes systemd to wait for the binary to call
        # sd_notify(READY=1) before marking the unit active.  The binary
        # does this via the sd-notify crate immediately after the listener
        # is bound.  NotifyAccess = main restricts who may send
        # notifications to the main process only.
        Type = "notify";
        NotifyAccess = "main";

        # Restart if no WATCHDOG=1 heartbeat arrives within 30 s.  The
        # binary reads WATCHDOG_USEC and pings at half this interval (15 s).
        # Override via systemd.services.rust-template-daemon.serviceConfig.WatchdogSec.
        WatchdogSec = lib.mkDefault "30s";

        ExecStart =
          "${cfg.package}/bin/rust-template-daemon"
          + (
            if cfg.socket != null
            then " --listen sd-listen"
            else " --listen ${cfg.host}:${toString cfg.port}"
          )
          + " --frontend-path ${cfg.frontendPath}";

        LoadCredential = lib.mkIf (cfg.oidcClientSecretFile != null)
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
