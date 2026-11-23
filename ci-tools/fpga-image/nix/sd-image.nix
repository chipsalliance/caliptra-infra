# Licensed under the Apache-2.0 license
#
# Rootless FPGA SD Card Image Builder for NixOS (VCK-190 2.x & ZCU104 1.x).
#
# Produces an MBR-partitioned disk image matching the exact layout required by
# `fpga-boss` across VCK-190 (`core`, `subsystem`) and ZCU104 (`zcu104`):
#   - p1: FAT32 bootable partition containing board boot firmware, kernel, and U-Boot script
#   - p2: 8-sector partition containing the `CONFIG_PARTITION` sentinel
#   - p3: Root filesystem:
#         * `ci` mode: zstd-compressed read-only SquashFS (`/nix/store` mounted onto an ephemeral RAM `tmpfs` `/`)
#         * `dev` mode: mutable ext4 filesystem expanded to fill the SD card on first boot
#   - p4: (CI partition table entry only) persistent partition entry

{
  config,
  lib,
  pkgs,
  buildPkgs ? pkgs.buildPackages,
  targetPkgs ? (
    if buildPkgs.stdenv.hostPlatform.system == "aarch64-linux" then
      pkgs
    else
      buildPkgs.pkgsCross.aarch64-multiplatform
  ),
  githubRunnerPkg ? pkgs.github-runner,
  ...
}:

let
  cfg = config.fpga;
  isZcu104 = cfg.variant == "zcu104";
  imagePrefix = if isZcu104 then "zcu104" else "vck190-${cfg.variant}";
  toplevel = config.system.build.toplevel;

  vck190BootFirmware = import ./boot-firmware.nix {
    inherit buildPkgs targetPkgs;
    inherit (buildPkgs) lib;
  };

  vck190KernelBundle = import ./kernel.nix {
    pkgs = targetPkgs;
    inherit (targetPkgs) lib;
  };

  zcu104Boot = import ./zcu104-boot.nix {
    inherit buildPkgs;
    inherit (buildPkgs) lib;
  };

  vck190BootBin = vck190BootFirmware.mkBootBin (
    {
      inherit (cfg) variant;
    }
    // lib.optionalAttrs (cfg.xsa != null) {
      inherit (cfg) xsa;
    }
  );

  # Ephemeral Stage-1 PID 1 script for CI mode (`init=${ciStage1Init}`).
  # Mounts a fresh RAM disk (`tmpfs`) as `/`, bind-mounts `/nix/store` from the
  # read-only SquashFS partition (`/dev/mmcblk0p3`), detaches the SquashFS root,
  # and execs NixOS stage-2 init (`${toplevel}/init`). All runtime state lives
  # purely in RAM and vanishes on board reset.
  ciStage1Init = pkgs.writeScript "caliptra-ci-stage1-init" ''
    #!${pkgs.bashInteractive}/bin/sh
    # Licensed under the Apache-2.0 license

    export PATH=${
      lib.makeBinPath [
        pkgs.util-linux
        pkgs.coreutils
        pkgs.bashInteractive
      ]
    }

    echo "--- Starting Caliptra NixOS Ephemeral tmpfs Stage-1 ---"

    # Ensure private mount propagation before pivot_root (in case an initramfs marked `/` shared)
    mount -n --make-rprivate / 2>/dev/null || true

    # Mount ephemeral RAM rootfs (wiped completely on reset)
    mount -n -t tmpfs -o mode=0755,size=75% tmpfs /mnt/new_root

    mkdir -p /mnt/new_root/nix/store /mnt/new_root/old_root /mnt/new_root/bin /mnt/new_root/usr/bin
    mount -n --bind /nix/store /mnt/new_root/nix/store
    ln -sf ${pkgs.bashInteractive}/bin/sh /mnt/new_root/bin/sh
    ln -sf ${pkgs.bashInteractive}/bin/bash /mnt/new_root/bin/bash
    ln -sf ${pkgs.coreutils}/bin/env /mnt/new_root/usr/bin/env

    if [ -f /nix-path-registration ]; then
      cp /nix-path-registration /mnt/new_root/nix-path-registration
    fi

    ${lib.optionalString isZcu104 ''
      mkdir -p /mnt/new_root/lib
      ln -s ${zcu104Boot.kernelModules}/lib/modules /mnt/new_root/lib/modules
    ''}

    # If booted via an initramfs (e.g. ZCU104 `image.fit`), preserve its virtual mounts;
    # otherwise (VCK-190 direct boot), leave `/proc` unmounted so NixOS stage-2 `mounts.sh`
    # mounts `/proc`, `/sys`, `/dev`, `/dev/pts`, `/dev/shm`, and `/run` with standard options.
    if [ -e /proc/1 ]; then
      for mp in /dev /proc /sys /run; do
        if mountpoint -q "$mp"; then
          mkdir -p "/mnt/new_root$mp"
          mount -n --move "$mp" "/mnt/new_root$mp"
        fi
      done
    fi

    cd /mnt/new_root
    if ! pivot_root . old_root; then
      echo "pivot_root failed! Dropping to shell."
      exec ${pkgs.bashInteractive}/bin/sh
    fi

    umount -n -l /old_root
    rmdir /old_root 2>/dev/null || true

    exec ${toplevel}/init || {
      echo "Failed to exec NixOS stage-2 init! Dropping to shell."
      exec ${pkgs.bashInteractive}/bin/sh
    }
  '';

  initPath = if cfg.isDev then "${toplevel}/init" else "${ciStage1Init}";

  sdClosureInfo = buildPkgs.closureInfo {
    rootPaths = [ toplevel ] ++ lib.optional (!cfg.isDev) ciStage1Init;
  };

  # Builds the partition 3 rootfs image (SquashFS for `ci`, ext4 for `dev`)
  # using native host tools (`buildPkgs`) for fast image creation.
  rootfsImage = buildPkgs.stdenv.mkDerivation {
    name = "${imagePrefix}-${if cfg.isDev then "dev-ext4" else "ci-squashfs"}.img";

    nativeBuildInputs = with buildPkgs; [
      coreutils
      findutils
      gawk
      gnutar
      gzip
      squashfsTools
      e2fsprogs
      libfaketime
      fakeroot
    ];

    buildCommand = ''
      mkdir -p ./rootImage/proc ./rootImage/sys ./rootImage/dev ./rootImage/run
      cp ${sdClosureInfo}/registration ./rootImage/nix-path-registration

      ${lib.optionalString isZcu104 ''
        mkdir -p ./rootImage/lib
        cp -a ${zcu104Boot.kernelModules}/lib/modules ./rootImage/lib/
        chmod -R u+w ./rootImage/lib/modules
      ''}

      ${
        if cfg.isDev then
          ''
            mkdir -p ./rootImage/bin ./rootImage/sbin ./rootImage/usr/bin
            mkdir -p ./rootImage/etc ./rootImage/tmp ./rootImage/var ./rootImage/lib/firmware
            mkdir -p ./rootImage/home/runner

            # Bootstrap symlinks needed before NixOS stage-2 activation runs
            ln -sf ${pkgs.bashInteractive}/bin/sh ./rootImage/bin/sh
            ln -sf ${pkgs.bashInteractive}/bin/bash ./rootImage/bin/bash
            ln -sf ${pkgs.coreutils}/bin/env ./rootImage/usr/bin/env
            ln -sf ${toplevel}/init ./rootImage/sbin/init
            ln -sf ${githubRunnerPkg}/bin/run.sh ./rootImage/home/runner/run.sh

            echo "Copying Nix store closure into rootImage..."
            mkdir -p ./rootImage/nix/store
            xargs cp -a --reflink=auto -t ./rootImage/nix/store/ < ${sdClosureInfo}/store-paths

            numInodes=$(find ./rootImage | wc -l)
            numDataBlocks=$(du -s -c -B 4096 --apparent-size ./rootImage | tail -1 | awk '{ print int($1 * 1.20) }')
            bytes=$((2 * 4096 * $numInodes + 4096 * $numDataBlocks))
            mebibyte=$((1024 * 1024))
            if (( bytes % mebibyte )); then
              bytes=$(( (bytes / mebibyte + 1) * mebibyte ))
            fi
            echo "Building initial ext4 dev rootfs of $bytes bytes..."
            truncate -s $bytes $out
            faketime -f "1970-01-01 00:00:01" fakeroot mkfs.ext4 -F -L NIXOS_ROOT -d ./rootImage $out

            export EXT2FS_NO_MTAB_OK=yes
            fsck.ext4 -n -f $out
            resize2fs -M $out
            block_size=$(dumpe2fs -h $out 2>/dev/null | awk -F: '/^Block size:/ { gsub(/ /, "", $2); print $2 }')
            block_count=$(dumpe2fs -h $out 2>/dev/null | awk -F: '/^Block count:/ { gsub(/ /, "", $2); print $2 }')
            # Leave 64 MiB of free headroom for stage-2 activation before resize-rootfs.service runs
            extra_blocks=$(( (64 * 1024 * 1024) / block_size ))
            new_blocks=$(( block_count + extra_blocks ))
            resize2fs $out $new_blocks
            truncate -s $(( new_blocks * block_size )) $out
            fsck.ext4 -n -f $out
          ''
        else
          ''
            mkdir -p ./rootImage/mnt/new_root
            echo "Building zstd-compressed SquashFS CI rootfs directly from Nix store paths..."
            (
              cd ./rootImage
              SOURCE_DATE_EPOCH=0 mksquashfs * $(cat ${sdClosureInfo}/store-paths) $out \
                -comp zstd -all-root -no-strip -keep-as-directory -b 1048576 \
                -processors $NIX_BUILD_CORES -root-mode 0755
            )
          ''
      }
    '';
  };

  # Assembles the complete partitioned SD card `.img` file without root/losetup
  sdImage = buildPkgs.stdenv.mkDerivation {
    name = "caliptra-fpga-image-${cfg.variant}${lib.optionalString cfg.isDev "-dev"}.img";

    nativeBuildInputs = with buildPkgs; [
      coreutils
      dosfstools
      mtools
      libfaketime
      util-linux
    ];

    buildCommand = ''
      mkdir -p $out
      img="$out/${if cfg.isDev then "dev-image.img" else "image.img"}"

      bootfs_blocks=$((80000 * ${if isZcu104 then "2" else "4"}))
      rootfs_bytes=$(stat --printf="%s" ${rootfsImage})
      rootfs_blocks=$(( (rootfs_bytes + 511) / 512 ))
      total_blocks=$((2048 + 8 + bootfs_blocks + rootfs_blocks))

      truncate -s $((total_blocks * 512)) "$img"

      ${
        if cfg.isDev then
          ''
            sfdisk --no-reread --no-tell-kernel "$img" <<EOF
            label: dos
            label-id: 0x4effe30a
            unit: sectors
            sector-size: 512

            p1 : start=2048, size=''${bootfs_blocks}, type=c, bootable
            p2 : start=$((2048 + bootfs_blocks)), size=8, type=83
            p3 : start=$((2048 + 8 + bootfs_blocks)), size=''${rootfs_blocks}, type=83
            EOF
          ''
        else
          ''
            persistfs_blocks=14680064
            truncate -s $(( (total_blocks + persistfs_blocks) * 512 )) "$img"
            sfdisk --no-reread --no-tell-kernel "$img" <<EOF
            label: dos
            label-id: 0x4effe30a
            unit: sectors
            sector-size: 512

            p1 : start=2048, size=''${bootfs_blocks}, type=c, bootable
            p2 : start=$((2048 + bootfs_blocks)), size=8, type=83
            p3 : start=$((2048 + 8 + bootfs_blocks)), size=''${rootfs_blocks}, type=83
            p4 : start=$((2048 + 8 + bootfs_blocks + rootfs_blocks)), size=''${persistfs_blocks}, type=83
            EOF
            # Truncate image right after p3 while keeping p4 in the MBR partition table
            truncate -s $((total_blocks * 512)) "$img"
          ''
      }

      # Build and populate FAT32 boot partition (p1)
      truncate -s $((bootfs_blocks * 512)) bootfs.img
      mkfs.vfat --invariant -i 4effe30a -n BOOT bootfs.img

      mkdir -p boot_files
      ${
        if isZcu104 then
          ''
            tar -xzf ${zcu104Boot.systemBootTarball} -C boot_files
            chmod -R u+w boot_files
            cp -f ${
              zcu104Boot.mkBootScrUimg {
                inherit (cfg) isDev;
                inherit initPath;
              }
            } boot_files/boot.scr.uimg
          ''
        else
          ''
            cp ${vck190BootBin} boot_files/BOOT.BIN
            cp ${vck190BootFirmware.mkSystemDtb cfg.variant} boot_files/system.dtb
            cp ${
              vck190KernelBundle.mkBootScr {
                inherit (cfg) isDev;
                inherit initPath;
              }
            } boot_files/boot.scr
            cp ${vck190KernelBundle.linuxXlnx}/Image boot_files/Image
          ''
      }

      find boot_files -exec touch --date=2000-01-01 {} +
      (
        cd boot_files
        for d in $(find . -mindepth 1 -type d | sort); do
          mmd -i ../bootfs.img "::/$d"
        done
        for f in $(find . -type f | sort); do
          mcopy -pvm -i ../bootfs.img "$f" "::/$f"
        done
      )
      fsck.vfat -vn bootfs.img
      dd if=bootfs.img of="$img" bs=4M oflag=seek_bytes seek=$((2048 * 512)) conv=notrunc,sparse

      # Write CONFIG_PARTITION sentinel to p2
      printf "CONFIG_PARTITION\n" > config_part.bin
      dd conv=notrunc if=config_part.bin of="$img" bs=512 seek=$((2048 + bootfs_blocks))

      # Write RootFS image to p3
      dd if=${rootfsImage} of="$img" bs=4M oflag=seek_bytes seek=$(( (2048 + 8 + bootfs_blocks) * 512 )) conv=notrunc,sparse

      # Expose hydra build products for CI
      mkdir -p $out/nix-support
      echo "file sd-image $img" >> $out/nix-support/hydra-build-products
    '';
  };
in
{
  options.fpga = {
    variant = lib.mkOption {
      type = lib.types.enum [
        "core"
        "subsystem"
        "zcu104"
      ];
      default = "subsystem";
      description = "Caliptra FPGA image target (`core` or `subsystem` for VCK-190 2.x, or `zcu104` for ZCU104 1.x).";
    };

    isDev = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Whether to build a mutable developer image (`true`) or an immutable SquashFS+overlay CI image (`false`).";
    };

    xsa = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = ''
        Optional override path to a custom Vivado `.xsa` archive for assembling VCK-190 `BOOT.BIN`.
        When `null`, defaults to the pinned CI `.xsa` for `fpga.variant`. Unused on `zcu104`.
      '';
    };

    workflowInfo = {
      workflow = lib.mkOption {
        type = lib.types.str;
        default = "nix-flake";
      };
      headRef = lib.mkOption {
        type = lib.types.str;
        default = "main";
      };
      workflowSha = lib.mkOption {
        type = lib.types.str;
        default = "unknown";
      };
    };
  };

  config = {
    system.build.rootfsImage = rootfsImage;
    system.build.sdImage = sdImage;
  };
}
