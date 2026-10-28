{ self }:

{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.waybright;
  system = pkgs.stdenv.hostPlatform.system;
in
{
  options.services.waybright = {
    enable = lib.mkEnableOption "waybright brightness daemon";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${system}.waybright;
      description = "The waybright package to use.";
    };

    hardwareMin = lib.mkOption {
      type = lib.types.ints.between 1 99;
      default = 50;
      description = "Virtual brightness level where hardware dimming begins.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];

    systemd.user.sockets.waybright = {
      description = "waybright daemon socket";
      wantedBy = [ "graphical-session.target" ];
      partOf = [ "graphical-session.target" ];
      socketConfig = {
        ListenStream = "%t/waybright.sock";
        RemoveOnStop = true;
      };
    };

    systemd.user.services.waybright = {
      description = "Wayland brightness daemon";
      wantedBy = [ "graphical-session.target" ];
      requires = [ "waybright.socket" ];
      after = [
        "graphical-session.target"
        "waybright.socket"
      ];
      partOf = [ "graphical-session.target" ];
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/waybright daemon --hardware-min ${toString cfg.hardwareMin}";
        Environment = [ "WAYBRIGHT_SOCKET=%t/waybright.sock" ];
        Restart = "on-failure";
      };
    };
  };
}
