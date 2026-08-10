{ self }:

{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.waybright;
  waydark = cfg.waydark;
  system = pkgs.stdenv.hostPlatform.system;
in
{
  options.services.waybright = {
    enable = lib.mkEnableOption "waybright brightness tools";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${system}.waybright;
      description = "The waybright package to install.";
    };

    waydark = {
      enable = lib.mkEnableOption "waydark software dimming daemon";

      package = lib.mkOption {
        type = lib.types.package;
        default = self.packages.${system}.waydark;
        description = "The waydark package to use.";
      };

      socketPath = lib.mkOption {
        type = lib.types.str;
        default = "%t/waydark.sock";
        description = ''
          Socket path for the waydark daemon. The default expands to
          $XDG_RUNTIME_DIR/waydark.sock in the user systemd manager.
        '';
      };
    };
  };

  config = lib.mkMerge [
    (lib.mkIf cfg.enable {
      environment.systemPackages = [ cfg.package ];
      services.waybright.waydark.enable = lib.mkDefault true;
    })

    (lib.mkIf waydark.enable {
      systemd.user.sockets.waydark = {
        description = "waydark daemon socket";
        wantedBy = [ "sockets.target" ];

        socketConfig = {
          ListenStream = waydark.socketPath;
          RemoveOnStop = true;
        };
      };

      systemd.user.services.waydark = {
        description = "waydark software dimming daemon";
        wantedBy = [ "graphical-session.target" ];
        requires = [ "waydark.socket" ];
        after = [
          "graphical-session.target"
          "waydark.socket"
        ];
        partOf = [ "graphical-session.target" ];

        serviceConfig = {
          ExecStart = "${waydark.package}/bin/waydark daemon";
          Environment = [ "WAYDARK_SOCKET=${waydark.socketPath}" ];
          Restart = "on-failure";
        };
      };
    })
  ];
}
