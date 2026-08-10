{ self }:

{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.waybright.waydark;
  system = pkgs.stdenv.hostPlatform.system;
in
{
  options.services.waybright.waydark = {
    enable = lib.mkEnableOption "waydark software dimming daemon";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${system}.waydark;
      defaultText = lib.literalExpression "waybright.packages.\${pkgs.stdenv.hostPlatform.system}.waydark";
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

  config = lib.mkIf cfg.enable {
    systemd.user.sockets.waydark = {
      description = "waydark daemon socket";
      wantedBy = [ "sockets.target" ];

      socketConfig = {
        ListenStream = cfg.socketPath;
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
        ExecStart = "${cfg.package}/bin/waydark daemon";
        Environment = [ "WAYDARK_SOCKET=${cfg.socketPath}" ];
        Restart = "on-failure";
      };
    };
  };
}
