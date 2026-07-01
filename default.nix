{
  pkgs ? import <nixpkgs> { },
  naersk,
}:
let
  cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
  naersk-lib = pkgs.callPackage naersk { };
in
naersk-lib.buildPackage {
  pname = "waybright";
  version = cargoToml.workspace.package.version;
  src = ./.;
}
