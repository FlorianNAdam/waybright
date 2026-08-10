{
  naersk-lib,
  pkgs,
}:

let
  inherit (pkgs) lib;
  root = ../.;

  cargoToml = fromTOML (builtins.readFile (root + "/Cargo.toml"));
  crates = cargoToml.workspace.members;

  rustSource = lib.cleanSourceWith {
    src = root;
    filter =
      path: type:
      let
        rel = lib.removePrefix "${toString root}/" (toString path);
      in
      rel == "Cargo.toml"
      || rel == "Cargo.lock"
      || lib.any (crate: lib.hasPrefix "${crate}/" rel) crates
      || (type == "directory" && builtins.elem rel crates);
  };
in
naersk-lib.buildPackage {
  name = "waybright-workspace";
  pname = "waybright-workspace";
  version = cargoToml.workspace.package.version;
  src = rustSource;

  nativeBuildInputs = with pkgs; [
    pkg-config
  ];

  buildInputs = with pkgs; [
    dbus
    eudev
    libxkbcommon
    wayland
  ];
}
