# Licensed under the Apache-2.0 license
#
# Declarative NixOS configuration for Caliptra FPGA images (VCK-190 2.x & ZCU104 1.x).

{
  config,
  lib,
  pkgs,
  modulesPath,
  buildPkgs ? pkgs.buildPackages,
  targetPkgs ? pkgs,
  ...
}:

let
  cfg = config.fpga;
  isZcu104 = cfg.variant == "zcu104";
  serialTty = if isZcu104 then "ttyPS0" else "ttyAMA0";
  sshKeys = import ./ssh-keys.nix;

  vck190KernelBundle = import ./kernel.nix {
    pkgs = targetPkgs;
    inherit (targetPkgs) lib;
  };

  zcu104Boot = import ./zcu104-boot.nix {
    inherit buildPkgs;
    inherit (buildPkgs) lib;
  };

  fpgaHealthd = import ./healthd.nix {
    inherit buildPkgs pkgs targetPkgs;
    inherit (pkgs) lib;
  };

  # Deterministic, board-unique MAC address generator reading `/sys/block/mmcblk0/device/cid`
  setMacAddressScript = pkgs.writeShellApplication {
    name = "set-mac-address";
    runtimeInputs = with pkgs; [
      coreutils
      findutils
      gnugrep
      gawk
      iproute2
    ];
    text = ''
      REQUESTED_IFACE="''${1:-}"

      get_hardware_seed() {
        # SD Card CID (128-bit unique hardware card identification register on the boot SD card)
        if [ -f /sys/block/mmcblk0/device/cid ]; then
          local cid
          cid=$(tr -d '\n\r\0' < /sys/block/mmcblk0/device/cid 2>/dev/null | xargs)
          if [ -n "$cid" ]; then
            echo "$cid"
            return 0
          fi
        fi

        # Fallback seed if SD card CID is unavailable (e.g. QEMU or container)
        echo "caliptra-fpga-default-seed"
      }

      SEED=$(get_hardware_seed)
      HASH=$(echo -n "$SEED" | sha256sum | awk '{print $1}')

      # Construct locally administered unicast MAC address (02:xx:xx:xx:xx:xx)
      MAC="02:''${HASH:0:2}:''${HASH:2:2}:''${HASH:4:2}:''${HASH:6:2}:''${HASH:8:2}"

      if [ "$REQUESTED_IFACE" = "--print" ]; then
        echo "$MAC"
        exit 0
      fi

      IFACE=""
      for candidate in "$REQUESTED_IFACE" end0 eth0; do
        if [ -n "$candidate" ] && ip link show "$candidate" >/dev/null 2>&1; then
          IFACE="$candidate"
          break
        fi
      done

      if [ -z "$IFACE" ]; then
        for netdev in /sys/class/net/*; do
          devname=$(basename "$netdev")
          if [ "$devname" != "lo" ] && [ -f "$netdev/type" ] && [ "$(cat "$netdev/type" 2>/dev/null)" = "1" ]; then
            IFACE="$devname"
            break
          fi
        done
      fi

      if [ -n "$IFACE" ] && ip link show "$IFACE" >/dev/null 2>&1; then
        CURRENT_MAC=$(ip link show "$IFACE" | grep -oE 'link/ether [0-9a-fA-F:]+' | awk '{print $2}')
        if [ "$CURRENT_MAC" != "$MAC" ]; then
          echo "Setting MAC address for $IFACE to $MAC (seed: $SEED)"
          ip link set dev "$IFACE" down 2>/dev/null || true
          ip link set dev "$IFACE" address "$MAC"
          ip link set dev "$IFACE" up 2>/dev/null || true
        fi
      fi
    '';
  };

  # Helper executed inside a `runner` login shell to populate the GitHub Actions
  # runner environment (`PATH`, `NIX_LD`, `NIX_LD_LIBRARY_PATH`, TLS certs) before
  # launching `Runner.Listener` with a one-time JIT configuration.
  runGhaJit = pkgs.writeShellScript "run-gha-jit" ''
    set -euo pipefail
    if [ -f /etc/set-environment ]; then
      # shellcheck disable=SC1091
      . /etc/set-environment
    fi
    export NIX_LD="/run/current-system/sw/share/nix-ld/lib/ld.so"
    export NIX_LD_LIBRARY_PATH="/run/current-system/sw/share/nix-ld/lib"
    export SSL_CERT_FILE="/etc/ssl/certs/ca-certificates.crt"
    export NIX_SSL_CERT_FILE="/etc/ssl/certs/ca-certificates.crt"
    export PATH="/run/wrappers/bin:/run/current-system/sw/bin:/usr/bin:/bin:/usr/sbin:/sbin:''${PATH:-}"
    export RUNNER_ROOT="/home/runner/.github-runner"

    mkdir -p "$RUNNER_ROOT"
    printf '%s\n' "$PATH" > "$RUNNER_ROOT/.path"
    cat > "$RUNNER_ROOT/.env" <<EOF
    NIX_LD=$NIX_LD
    NIX_LD_LIBRARY_PATH=$NIX_LD_LIBRARY_PATH
    SSL_CERT_FILE=$SSL_CERT_FILE
    NIX_SSL_CERT_FILE=$NIX_SSL_CERT_FILE
    LANG=''${LANG:-en_US.UTF-8}
    LOCALE_ARCHIVE=''${LOCALE_ARCHIVE:-/run/current-system/sw/lib/locale/locale-archive}
    EOF

    exec /home/runner/run.sh --jitconfig "''${1:-}"
  '';

  # Startup script executed on the board UART that communicates with `fpga-boss`
  startupScript = pkgs.writeShellApplication {
    name = "startup-script";
    runtimeInputs = with pkgs; [
      coreutils
      gawk
      iproute2
      systemd
      util-linux
      su
      bashInteractive
      github-runner
    ];
    text =
      lib.optionalString isZcu104 ''
        echo 3 > /proc/sys/kernel/printk || true
      ''
      + (
        if cfg.isDev then
          ''
            echo "Developer mode: Skipping CI startup logic."
            systemctl start resize-rootfs.service || true
            exec login -f root
          ''
        else
          ''
            echo "CI mode: Starting command loop..."

            runner_jitconfig() {
              echo "Executing GHA runner"
              su - runner -c "${runGhaJit} \"''${cmd_array[1]:-}\"" || true
              echo "GHA runner complete"
            }

            # Wait up to 15s for DHCPv4 on primary wired NIC (`end0` or `eth0`) and announce `/healthz` endpoint to fpga-boss
            FPGA_IP=""
            for _ in $(seq 1 15); do
              FPGA_IP=$(ip -4 -o addr show scope global 2>/dev/null | awk '$2 != "lo" { split($4, a, "/"); print a[1]; exit }' || true)
              if [[ -n "$FPGA_IP" ]]; then
                break
              fi
              sleep 1
            done
            if [[ -n "$FPGA_IP" ]]; then
              echo "CALIPTRA_FPGA_INFO {\"ip\":\"$FPGA_IP\",\"health_port\":8080,\"variant\":\"${cfg.variant}\"}"
            fi

            # Emit sentinel telling fpga-boss (listening via UART) we are ready for input
            echo "36668aa492b1c83cdd3ade8466a0153d --- Command input"
            echo "Available commands:"
            echo "  runner-jitconfig <base64>"
            echo "  login"
            read -r -e -p "> " cmd || true

            read -r -a cmd_array <<< "$cmd"
            if [[ "$cmd" == "login" ]]; then
              login -f root
            elif [[ "''${cmd_array[0]:-}" == "runner-jitconfig" ]]; then
              runner_jitconfig
            else
              echo "Unknown command $cmd"
            fi

            # Emit sentinel telling fpga-boss we are done and can be reset
            echo "3297327285280f1ffb8b57222e0a5033 --- ACTION IS COMPLETE"

            login -f root
            shutdown -h now
          ''
      );
  };

  # Partition 3 online resize script for `dev` images
  resizeRootfsScript = pkgs.writeShellApplication {
    name = "resize-rootfs";
    runtimeInputs = with pkgs; [
      cloud-utils
      e2fsprogs
    ];
    text = ''
      set -x
      growpart /dev/mmcblk0 3 || true
      resize2fs /dev/mmcblk0p3
    '';
  };

  # Stub `podman` binary so `cargo xtask` host checks pass when run directly on the FPGA
  podmanStub = pkgs.writeShellScriptBin "podman" ''
    echo "stub"
  '';
in
{
  imports = [
    "${modulesPath}/profiles/minimal.nix"
    "${modulesPath}/profiles/headless.nix"
    ./sd-image.nix
  ];

  system.stateVersion = "25.11";

  # Disable unused desktop/server storage and container subsystems to keep closure small
  boot.bcache.enable = false;
  boot.swraid.enable = false;
  boot.enableContainers = false;
  services.lvm.enable = false;

  # Boot & Kernel configuration:
  # The Versal boot chain (BOOT.BIN -> U-Boot -> boot.scr -> Image) boots directly into
  # partition 3 without a NixOS stage-1 initrd; UIO devices (`/dev/uio*`) are bound
  # automatically by `uio_pdrv_genirq` (`generic-uio`) from the variant `system.dtb` overlay.
  boot.loader.grub.enable = false;
  boot.initrd.enable = false;
  system.build.initialRamdisk =
    buildPkgs.runCommand "dummy-initrd" { }
      "mkdir -p $out; touch $out/${config.system.boot.loader.initrdFile}";
  boot.kernel.enable = !isZcu104;
  boot.kernelPackages = lib.mkIf (!isZcu104) (pkgs.linuxPackagesFor vck190KernelBundle.linuxXlnx);
  system.systemBuilderCommands = lib.mkIf isZcu104 ''
    ln -s ${zcu104Boot.kernelModules} $out/kernel-modules
  '';

  # Arm the Cadence hardware watchdog so even unattended `dev` boards recover from hard hangs
  systemd.settings.Manager = {
    RuntimeWatchdogSec = "30s";
    RebootWatchdogSec = "2m";
  };

  services.udev.extraRules = ''
    SUBSYSTEM=="uio", MODE="0666"
    KERNEL=="caliptra-rom-backdoor", MODE="0666"
  '';

  # Root filesystem declaration:
  # - `dev` mode: mutable ext4 partition on `/dev/mmcblk0p3`
  # - `ci` mode: ephemeral RAM `tmpfs` at `/` (with `/nix/store` bind-mounted from read-only SquashFS `p3`)
  fileSystems."/" =
    if cfg.isDev then
      {
        device = "/dev/mmcblk0p3";
        fsType = "ext4";
      }
    else
      {
        device = "tmpfs";
        fsType = "tmpfs";
        options = [
          "mode=0755"
          "size=75%"
        ];
      };

  # Kernel sysctls matching `build.sh` lines 46-49
  boot.kernel.sysctl = {
    "kernel.softlockup_panic" = 1;
    "kernel.softlockup_all_cpu_backtrace" = 1;
    "kernel.panic_print" = 127;
    "kernel.sysrq" = 1;
  };

  # Networking: DHCPv4 + IPv6 RA on `end0`, Google DNS, NTP, deterministic MAC address
  networking = {
    hostName = "caliptra-fpga";
    useDHCP = false;
    useNetworkd = true;
    firewall.enable = false;
    nameservers = [
      "8.8.8.8"
      "2001:4860:4860::6464"
      "2001:4860:4860::64"
    ];
    timeServers = [ "time.google.com" ];
  };

  systemd.network = {
    enable = true;
    networks."10-wired" = {
      matchConfig.Name = "end* eth*";
      networkConfig = {
        DHCP = "yes";
        IPv6AcceptRA = true;
      };
    };
  };

  systemd.services.set-mac-address = {
    description = "Set deterministic board-unique MAC address from SD Card CID";
    before = [
      "systemd-networkd.service"
      "network-pre.target"
    ];
    wants = [ "network-pre.target" ];
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      ExecStart = "${setMacAddressScript}/bin/set-mac-address";
    };
  };

  services.timesyncd.enable = true;

  # SSH configuration (enabled with developer keys in `dev` mode)
  services.openssh = {
    enable = cfg.isDev;
    settings = {
      PubkeyAuthentication = true;
      PasswordAuthentication = false;
    };
  };

  # Users & passwordless sudo (`runner ALL=(ALL) NOPASSWD: SETENV: ALL`)
  users.mutableUsers = false;
  users.allowNoPasswordLogin = true;
  users.users.runner = {
    isNormalUser = true;
    uid = 1000;
    home = "/home/runner";
    createHome = true;
    linger = true;
    shell = pkgs.bashInteractive;
    extraGroups = [
      "wheel"
      "dialout"
      "tty"
    ];
    openssh.authorizedKeys.keys = lib.optionals cfg.isDev sshKeys;
  };

  services.logind.settings.Login.RemoveIPC = false;

  security.sudo = {
    enable = true;
    wheelNeedsPassword = false;
    extraRules = [
      {
        users = [ "runner" ];
        commands = [
          {
            command = "ALL";
            options = [
              "NOPASSWD"
              "SETENV"
            ];
          }
        ];
      }
    ];
    extraConfig = ''
      Defaults env_keep += "NIX_LD NIX_LD_LIBRARY_PATH PATH SSL_CERT_FILE NIX_SSL_CERT_FILE"
    '';
  };

  # Enable `nix-ld` so unpatched Ubuntu/Debian aarch64 test binaries from `caliptra-builder`
  # and any downloaded tools execute natively without patchelf.
  programs.nix-ld = {
    enable = true;
    libraries = with pkgs; [
      stdenv.cc.cc
      zlib
      zstd
      xz
      bzip2
      openssl
      libusb1
      libffi
      elfutils
      icu
      curl
      util-linux
      systemd
    ];
  };

  environment.variables = {
    NIX_LD = "/run/current-system/sw/share/nix-ld/lib/ld.so";
    NIX_LD_LIBRARY_PATH = "/run/current-system/sw/share/nix-ld/lib";
    SSL_CERT_FILE = "/etc/ssl/certs/ca-certificates.crt";
    NIX_SSL_CERT_FILE = "/etc/ssl/certs/ca-certificates.crt";
  };

  environment.extraInit = ''
    export PATH="$PATH:/usr/bin:/bin:/usr/sbin:/sbin"
  '';

  # Ensure standard FHS paths (`/bin/bash`, `/usr/bin/env`, `/lib/ld-linux-aarch64.so.1`,
  # `/lib/firmware`, `/usr/bin/cargo-nextest`) exist in both `ci` (tmpfs) and `dev` (ext4) modes.
  systemd.tmpfiles.rules = [
    "d /lib/firmware 0755 root root -"
    "d /lib64 0755 root root -"
    "L+ /bin/bash - - - - ${pkgs.bashInteractive}/bin/bash"
    "L+ /usr/bin/env - - - - ${pkgs.coreutils}/bin/env"
    "L+ /lib/ld-linux-aarch64.so.1 - - - - ${config.programs.nix-ld.package}/libexec/nix-ld"
    "L+ /lib64/ld-linux-aarch64.so.1 - - - - ${config.programs.nix-ld.package}/libexec/nix-ld"
    "L+ /usr/bin/cargo-nextest - - - - ${pkgs.cargo-nextest}/bin/cargo-nextest"
    "L+ /usr/bin/rsync - - - - ${pkgs.rsync}/bin/rsync"
    "L+ /usr/bin/podman - - - - ${podmanStub}/bin/podman"
    "L+ /home/runner/run.sh - - - - ${pkgs.github-runner}/bin/run.sh"
  ];

  # System packages matching `build.sh`
  environment.systemPackages =
    with pkgs;
    [
      git
      curl
      cacert
      sudo
      vmtouch
      fping
      openocd
      gdb
      cargo-nextest
      rsync
      unzip
      jq
      kmod
      pciutils
      usbutils
      github-runner
      fpgaHealthd
      podmanStub
      setMacAddressScript
      startupScript
    ]
    ++ lib.optionals cfg.isDev [
      openssh
      tmux
      cloud-utils
      e2fsprogs
      resizeRootfsScript
    ];

  # Build metadata file `/etc/caliptra-build-info`
  environment.etc."caliptra-build-info".text = ''
    Build system: nixos-${config.system.nixos.release}
    Image env : ${if cfg.isDev then "dev" else "non-dev"}
    Image variant : ${cfg.variant}
    Runner version : ${pkgs.github-runner.version}
    Kernel source : ${if isZcu104 then "canonical-zcu-5.15" else "nix-linux-xlnx"}
    Workflow name : ${cfg.workflowInfo.workflow}
    Workflow branch : ${cfg.workflowInfo.headRef}
    Workflow build commit : ${cfg.workflowInfo.workflowSha}
  '';

  # HTTP health endpoint (`:8080/healthz`) polled by `fpga-boss` and hostrunner status server
  systemd.services.caliptra-fpga-healthd = {
    description = "Caliptra FPGA HTTP Health Agent (:8080/healthz)";
    after = [ "network.target" ];
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "simple";
      DynamicUser = true;
      Environment = [ "CALIPTRA_HEALTHD_PORT=8080" ];
      ExecStart = "${fpgaHealthd}/bin/caliptra-fpga-healthd";
      Restart = "always";
      RestartSec = "2s";
    };
  };

  # Disable default serial-getty on the board's primary UART because `startup-script.service` owns it
  systemd.services."serial-getty@${serialTty}".enable = false;

  # UART startup service for `fpga-boss`
  systemd.services.startup-script = {
    description = "Caliptra FPGA UART Startup Script (fpga-boss interface)";
    after = [
      "serial-getty.target"
      "time-sync.target"
      "network-online.target"
    ];
    wants = [
      "serial-getty.target"
      "time-sync.target"
      "network-online.target"
    ];
    conflicts = [ "serial-getty@${serialTty}.service" ];
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "idle";
      Restart = "no";
      StandardInput = "tty-force";
      StandardOutput = "tty";
      StandardError = "tty";
      TTYPath = "/dev/${serialTty}";
      User = "root";
      WorkingDirectory = "/home/runner";
      ExecStart = "${startupScript}/bin/startup-script";
    };
  };

  # One-shot rootfs expansion service for `dev` images
  systemd.services.resize-rootfs = lib.mkIf cfg.isDev {
    description = "Resize root partition to fill SD card on first boot";
    after = [ "local-fs.target" ];
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      ExecStart = "${resizeRootfsScript}/bin/resize-rootfs";
    };
  };

  # Keep Nix store footprint minimal on the FPGA image
  documentation.enable = false;
  documentation.man.enable = false;
  documentation.nixos.enable = false;
}
