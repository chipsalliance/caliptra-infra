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
    (fpga_service.mkVckSubsystemJob "caliptra-svl-vck-8" {
      ftdi = "1-1.2.1.3";
      sdwire = "1-1.2.1.4";
    })
    (fpga_service.mkVckSubsystemJob "caliptra-svl-vck-9" {
      ftdi = "1-1.2.1.2";
      sdwire = "1-1.2.1.1";
    })
    (fpga_service.mkVckSubsystemJob "caliptra-svl-vck-10" {
      ftdi = "1-1.2.3";
      sdwire = "1-1.2.4";
    })
    (fpga_service.mkVckSubsystemJob "caliptra-svl-vck-11" {
      ftdi = "1-1.1";
      sdwire = "1-1.2.2";
    })
  ];
}
