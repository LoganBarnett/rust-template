# NixOS module for the rust-template-web service.
# Exported from the flake as nixosModules.web.
#
# Minimal usage in a NixOS configuration:
#
#   inputs.rust-template.nixosModules.web
#
#   services.rust-template-web = {
#     enable = true;
#     port   = 8080;
#   };
{ self }:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.rust-template-web;
in
{
  options.services.rust-template-web = {
    enable = lib.mkEnableOption "rust-template-web web service";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.web;
      defaultText = lib.literalExpression "self.packages.\${system}.web";
      description = "Package providing the service binary.";
    };

    host = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1";
      description = "IP address to bind to.";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 3000;
      description = "TCP port to listen on.";
    };

    logLevel = lib.mkOption {
      type = lib.types.enum [ "trace" "debug" "info" "warn" "error" ];
      default = "info";
      description = "Tracing log verbosity level.";
    };

    logFormat = lib.mkOption {
      type = lib.types.enum [ "text" "json" ];
      default = "json";
      description = ''
        Log output format.  Use "text" for human-readable local logs and
        "json" for structured logs consumed by a log aggregator.
      '';
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = "rust-template-web";
      description = "System user account the service runs as.";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "rust-template-web";
      description = "System group the service runs as.";
    };
  };

  config = lib.mkIf cfg.enable {
    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      description = "rust-template-web service user";
    };

    users.groups.${cfg.group} = { };

    systemd.services.rust-template-web = {
      description = "rust-template-web web service";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];

      environment = {
        LOG_LEVEL = cfg.logLevel;
        LOG_FORMAT = cfg.logFormat;
      };

      serviceConfig = {
        # Type = notify causes systemd to wait for the binary to call
        # sd_notify(READY=1) before marking the unit active.  The binary
        # does this via the sd-notify crate immediately after the TCP
        # listener is bound.  NotifyAccess = main restricts who may send
        # notifications to the main process only.
        Type = "notify";
        NotifyAccess = "main";

        # Restart if no WATCHDOG=1 heartbeat arrives within 30 s.  The
        # binary reads WATCHDOG_USEC and pings at half this interval (15 s).
        # Override via systemd.services.rust-template-web.serviceConfig.WatchdogSec.
        WatchdogSec = lib.mkDefault "30s";

        ExecStart = "${cfg.package}/bin/rust-template-web"
          + " --host ${cfg.host}"
          + " --port ${toString cfg.port}";

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
