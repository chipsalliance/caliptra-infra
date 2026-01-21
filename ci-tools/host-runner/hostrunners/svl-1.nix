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
    (fpga_service.mkVckSubsystemJob "caliptra-svl-vck-4" {
      ftdi = "1-1.2.2";
      sdwire = "1-1.2.3";
    })
    (fpga_service.mkVckSubsystemJob "caliptra-svl-vck-5" {
      ftdi = "1-1.2.4";
      sdwire = "1-1.2.1.1";
    })
    (fpga_service.mkVckCoreJob "caliptra-svl-vck-6" {
      ftdi = "1-1.2.1.2";
      sdwire = "1-1.2.1.3";
    })
    # Note don't start this one. It's a dev board.
    (fpga_service.mkVckSubsystemJob "caliptra-svl-vck-7" {
      ftdi = "1-1.2.1.4";
      sdwire = "1-1.1";
    })
  ];
}
