{
  config,
  lib,
  pkgs,
  user,
  rtool,
  fpga-boss,
  ...
}:
let
  fpga_service = import ../fpga-service.nix {
    inherit
      pkgs
      rtool
      fpga-boss
      user
      ;
  };
in
{
  config = lib.mkMerge [
    (fpga_service.mkVckSubsystemDev "ttrippel" {
      ftdi = "1-1.2.1.3";
      sdwire = "1-1.2.1.4";
    })
    (fpga_service.mkVckSubsystemDev "amitkh" {
      ftdi = "1-1.2.1.2";
      sdwire = "1-1.2.1.1";
    })
    (fpga_service.mkVckSubsystemDev "cfrantz" {
      ftdi = "1-1.2.3";
      sdwire = "1-1.2.4";
    })
    (fpga_service.mkVckSubsystemDev "miguelosorio" {
      ftdi = "1-1.1";
      sdwire = "1-1.2.2";
    })
  ];
}
