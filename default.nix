{
  pkgs ? import <nixpkgs> { },
  naersk,
}:
let
  cargoToml = fromTOML (builtins.readFile ./Cargo.toml);
  naersk-lib = pkgs.callPackage naersk { };
in
naersk-lib.buildPackage {
  pname = "waybright";
  version = cargoToml.workspace.package.version;
  src = ./.;
  nativeBuildInputs = with pkgs; [
    pkg-config
  ];
  buildInputs = with pkgs; [
    dbus
  ];
}
