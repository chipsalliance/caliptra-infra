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
    (fpga_service.mkVckSubsystemJob "caliptra-kir-vck-6" {
      ftdi = "1-1.3.1.4";
      sdwire = "1-1.3.1.3";
    })
    (fpga_service.mkVckSubsystemJob "caliptra-kir-vck-7" {
      ftdi = "1-1.3.1.2";
      sdwire = "1-1.3.1.1";
    })
  ];
}
