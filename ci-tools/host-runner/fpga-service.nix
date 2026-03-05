{
  pkgs,
  rtool,
  fpga-boss,
  user,
  ...
}:
let
  scripts = import ./scripts.nix {
    inherit
      pkgs
      rtool
      fpga-boss
      user
      ;
  };
  mkFpgaJob =
    name: target: image:
    {
      ftdi,
      sdwire,
      enableService ? true,
    }:
    {
      systemd.user.services."${name}" = {
        enable = enableService;
        description = "${name} Service";
        after = [ "network.target" ];
        wantedBy = [ "multi-user.target" ];

        serviceConfig = {
          Type = "simple";
          ExecStart = "${scripts.fpga-boss-script}/bin/fpga.sh";
          Restart = "on-failure";
          RestartSec = "15s";
          Environment = [
            ''ZCU_FTDI="${ftdi}"''
            ''ZCU_SDWIRE="${sdwire}"''
            ''IDENTIFIER="${name}"''
            ''FPGA_TARGET=""${target}""''
            ''IMAGE="${image}"''
          ];
        };
      };
      environment.systemPackages = with pkgs; [
        (
          (pkgs.writeShellScriptBin "${name}-debug" ''
            #!${pkgs.bash}/bin/bash
            export ZCU_FTDI="${ftdi}"
            export ZCU_SDWIRE="${sdwire}"

            caliptra-fpga-boss --zcu104 $ZCU_FTDI --sdwire $ZCU_SDWIRE "$@"
          '')
        )
      ];
    };
in
rec {
  mkZcuJob =
    name:
    {
      ftdi,
      sdwire,
      enableService ? true,
    }:
    (mkFpgaJob name "caliptra-fpga,caliptra-fpga-nightly" "/home/${user}/ci-images/zcu104.img" {
      ftdi = ftdi;
      sdwire = sdwire;
      inherit enableService;
    });
  mkVckSubsystemJob =
    name:
    {
      ftdi,
      sdwire,
      enableService ? true,
    }:
    (mkFpgaJob name "vck190-subsystem" "/home/${user}/ci-images/caliptra-fpga-image-subsystem.img" {
      ftdi = ftdi;
      sdwire = sdwire;
      inherit enableService;
    });
  mkVckCoreJob =
    name:
    {
      ftdi,
      sdwire,
      enableService ? true,
    }:
    (mkFpgaJob name "vck190" "/home/${user}/ci-images/caliptra-fpga-image-core.img" {
      ftdi = ftdi;
      sdwire = sdwire;
      inherit enableService;
    });
  mkZcuJobDev =
    name:
    { ftdi, sdwire }:
    (
      let
        args = {
          ftdi = ftdi;
          sdwire = sdwire;
          enableService = false;
        };
      in
      mkZcuJob name args
    );
  mkVckSubsystemDev =
    name:
    { ftdi, sdwire }:
    (
      let
        args = {
          ftdi = ftdi;
          sdwire = sdwire;
          enableService = false;
        };
      in
      mkVckSubsystemJob name args
    );
  mkVckCoreDev =
    name:
    { ftdi, sdwire }:
    (
      let
        args = {
          ftdi = ftdi;
          sdwire = sdwire;
          enableService = false;
        };
      in
      mkVckCoreJob name args
    );
}
