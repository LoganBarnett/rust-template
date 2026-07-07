# mkDarwinService — generate a Darwin (launchd) module for a service.
#
# Usage in a spawned project's flake.nix:
#
#   darwinModules.server = inputs.foundation.lib.mkDarwinService {
#     name = "my-app-server";
#     self = self;
#   };
#
# Then in a nix-darwin configuration:
#
#   imports = [ inputs.my-app.darwinModules.server ];
#
#   services.my-app-server = {
#     enable = true;
#     baseUrl = "https://my-app.example.com";
#   };
#
# Generates: launchd service, optional health check agent, activation
# scripts for socket/log directories, newsyslog rotation, user/group
# with static UID/GID.
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

  # Env-var prefix derived from the service name, matching the Rust
  # MergeConfig macro's derivation: lowercase, with hyphens turned into
  # underscores (POSIX env-var names are restricted to letters, digits,
  # and underscore).  See crates/foundation/USAGE.org → "Environment
  # variables" for the naming rule and POSIX §8.1 citation.
  envPrefix = lib.replaceStrings ["-"] ["_"] (lib.toLower name);

  listenArg =
    if cfg.socket != null
    then "--listen ${cfg.socket}"
    else "--listen ${cfg.host}:${toString cfg.port}";

  execLine =
    "${cfg.package}/bin/${name}"
    + " ${listenArg}";

  logDir = "/var/log/${name}";

  sharedOptions = import ./service-options.nix {
    inherit name self cfg lib pkgs;
  };
in {
  options.services.${name} =
    sharedOptions
    // {
      socket = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = "/var/run/${name}/${name}.sock";
        description = ''
          Path for the Unix domain socket used by the service.  When
          set, the server binds its own socket (no launchd socket
          activation) and the host/port options are ignored.  Set to
          null to use TCP instead.
        '';
      };

      user = lib.mkOption {
        type = lib.types.str;
        default = "_${name}";
        description = ''
          System user account the service runs as.  The leading
          underscore follows the macOS convention for daemon accounts.
        '';
      };

      group = lib.mkOption {
        type = lib.types.str;
        default = "_${name}";
        description = ''
          System group the service runs as.  The leading underscore
          follows the macOS convention for daemon groups.
        '';
      };

      uid = lib.mkOption {
        type = lib.types.int;
        default = 401;
        description = ''
          UID for the service user.  nix-darwin requires a static UID
          for user creation.  The default (401) sits above macOS
          Sequoia's claimed 300-304 range and below the 501
          normal-user boundary.
        '';
      };

      gid = lib.mkOption {
        type = lib.types.int;
        default = 401;
        description = ''
          GID for the service group.  nix-darwin requires a static GID
          for group creation.  The default (401) mirrors the UID
          choice.
        '';
      };

      healthCheck = {
        enable =
          lib.mkEnableOption
          "periodic health-check agent for the server";

        url = lib.mkOption {
          type = lib.types.str;
          default = "http://127.0.0.1:${toString cfg.port}/health";
          defaultText =
            lib.literalExpression
            ''"http://127.0.0.1:''${toString cfg.port}/health"'';
          example = "http://127.0.0.1:3000/health";
          description = ''
            URL to probe for health.  The agent runs curl against this
            endpoint every 30 seconds and kills the server if it
            fails, letting launchd's KeepAlive restart it.
          '';
        };
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
      uid = cfg.uid;
      gid = cfg.gid;
      home = "/var/empty";
      shell = "/usr/bin/false";
      description = "${name} service user";
      isHidden = true;
    };

    users.groups.${cfg.group} = {
      gid = cfg.gid;
      members = [cfg.user];
    };

    users.knownUsers = [cfg.user];
    users.knownGroups = [cfg.group];

    # Create the log directory.  The socket directory is created by the
    # service itself (see ProgramArguments) to avoid coupling with
    # activation.  mkdir is qualified to GNU coreutils because BSD mkdir
    # (/bin/mkdir) lacks --parents — only -p — and global convention is
    # to use long-form CLI arguments wherever they exist.
    system.activationScripts.postActivation.text = ''
      ${pkgs.coreutils}/bin/mkdir --parents ${logDir}
      chown ${cfg.user}:${cfg.group} ${logDir}
      chmod 0750 ${logDir}
    '';

    # Rotate launchd-captured logs via newsyslog.  Without this, stdout
    # and stderr grow without bound — launchd does not rotate the files
    # it opens for StandardOutPath / StandardErrorPath.  Flags:
    #   N — no signal.  The service is not syslogd and does not handle
    #       SIGHUP for log re-open; launchd owns the file descriptors
    #       and continues writing to the rotated file's inode until the
    #       service restarts.  Acceptable for low-volume launchd logs;
    #       high-volume services should restart on rotation or write
    #       their own logs via a rotating sink.
    #   J — bzip2-compress archived rotations.
    # size is in KB (10240 = 10 MB); count is archives retained.
    environment.etc."newsyslog.d/${name}.conf".text = let
      rotateLine = file: "${file} ${cfg.user}:${cfg.group} 640 5 10240 * NJ";
    in
      lib.concatStringsSep "\n" (
        [
          "# logfilename [owner:group] mode count size when flags"
          (rotateLine "${logDir}/stdout.log")
          (rotateLine "${logDir}/stderr.log")
        ]
        ++ lib.optionals cfg.healthCheck.enable [
          (rotateLine "${logDir}/healthcheck-stdout.log")
          (rotateLine "${logDir}/healthcheck-stderr.log")
        ]
      )
      + "\n";

    # Every scalar serviceConfig value is wrapped in lib.mkDefault so
    # downstream modules can override individual fields with plain
    # assignment.  ProgramArguments and KeepAlive are also wrapped:
    # they semantically represent "the launch command" and "the restart
    # policy" — one value each, not lists of contributions — so a
    # downstream replacement is the natural override shape.
    # EnvironmentVariables is per-key mkDefault'd via mapAttrs so the
    # attrset itself can still accept additive contributions.
    launchd.servers.${name} = {
      serviceConfig = {
        ProgramArguments = lib.mkDefault (let
          sockSetup =
            lib.optionalString (cfg.socket != null)
            ("/bin/mkdir -p ${dirOf cfg.socket}"
              + " && /usr/sbin/chown ${cfg.user}:${cfg.group} ${dirOf cfg.socket}"
              + " && /bin/chmod 0750 ${dirOf cfg.socket}"
              + " && ");
        in [
          "/bin/sh"
          "-c"
          # Runs as root (no UserName/GroupName) so it can create the
          # socket directory, then drops to the service user via
          # sudo(8).
          (sockSetup
            + "/bin/wait4path ${cfg.package}"
            + " && exec /usr/bin/sudo -E -u ${cfg.user} ${execLine}")
        ]);
        RunAtLoad = lib.mkDefault true;
        KeepAlive = lib.mkDefault {
          Crashed = true;
          SuccessfulExit = false;
        };
        ThrottleInterval = lib.mkDefault 30;
        ProcessType = lib.mkDefault "Background";
        EnvironmentVariables = lib.mapAttrs (_: lib.mkDefault) (
          {
            "${envPrefix}_log_level" = cfg.logLevel;
            "${envPrefix}_log_format" = cfg.logFormat;
            "${envPrefix}_base_url" = cfg.baseUrl;
          }
          // lib.optionalAttrs (cfg.oidcIssuer != null) {
            "${envPrefix}_oidc_issuer" = cfg.oidcIssuer;
            "${envPrefix}_oidc_client_id" = cfg.oidcClientId;
            "${envPrefix}_oidc_client_secret_file" = cfg.oidcClientSecretFile;
          }
        );
        StandardOutPath = lib.mkDefault "${logDir}/stdout.log";
        StandardErrorPath = lib.mkDefault "${logDir}/stderr.log";
      };
    };

    # Optional health-check agent.  Probes the server's health endpoint
    # every 30 seconds and kills the server process on failure, letting
    # launchd's KeepAlive trigger a restart.
    launchd.servers."${name}-healthcheck" =
      lib.mkIf cfg.healthCheck.enable
      {
        serviceConfig = {
          ProgramArguments = lib.mkDefault [
            "/bin/sh"
            "-c"
            ''/usr/bin/curl -sf ${cfg.healthCheck.url} || /bin/kill $(/bin/cat /var/run/${name}/pid) 2>/dev/null''
          ];
          StartInterval = lib.mkDefault 30;
          RunAtLoad = lib.mkDefault false;
          ProcessType = lib.mkDefault "Background";
          StandardOutPath = lib.mkDefault "${logDir}/healthcheck-stdout.log";
          StandardErrorPath = lib.mkDefault "${logDir}/healthcheck-stderr.log";
        };
      };
  };
}
