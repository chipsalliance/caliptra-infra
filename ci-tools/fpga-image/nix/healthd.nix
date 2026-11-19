# Licensed under the Apache-2.0 license
#
# Builds the zero-dependency Rust `caliptra-fpga-healthd` HTTP `/healthz` daemon.
# Uses pre-built `rustc` binaries & `aarch64-unknown-linux-gnu` stdlibs from `cache.nixos.org`
# so cross-builds on `x86_64-linux` and native builds on `aarch64-linux` both finish in <1s
# without bootstrapping `rustc` from source.

{
  buildPkgs,
  pkgs,
  targetPkgs,
  lib ? buildPkgs.lib,
}:

let
  isNativeAarch64 = buildPkgs.stdenv.hostPlatform.system == "aarch64-linux";

  crossSysroot = buildPkgs.runCommand "rustc-aarch64-sysroot" { } ''
    mkdir -p $out/lib/rustlib
    ln -s ${buildPkgs.rustc.unwrapped}/lib/rustlib/${buildPkgs.stdenv.hostPlatform.rust.rustcTarget} $out/lib/rustlib/
    ln -s ${pkgs.rustc.unwrapped}/lib/rustlib/aarch64-unknown-linux-gnu $out/lib/rustlib/
  '';

  crossCc = targetPkgs.stdenv.cc;
  runtimeRpath = lib.makeLibraryPath [
    pkgs.glibc
    pkgs.stdenv.cc.cc.lib
  ];
in
if isNativeAarch64 then
  pkgs.stdenv.mkDerivation {
    pname = "caliptra-fpga-healthd";
    version = "0.1.0";
    dontUnpack = true;
    dontStrip = true;
    nativeBuildInputs = [ pkgs.rustc ];

    buildPhase = ''
      runHook preBuild
      rustc \
        --edition=2021 \
        -C opt-level=s \
        -C strip=symbols \
        -D warnings \
        ${./healthd.rs} \
        -o caliptra-fpga-healthd
      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall
      install -Dm755 caliptra-fpga-healthd $out/bin/caliptra-fpga-healthd
      runHook postInstall
    '';
  }
else
  buildPkgs.stdenv.mkDerivation {
    pname = "caliptra-fpga-healthd";
    version = "0.1.0";
    dontUnpack = true;
    dontStrip = true;
    nativeBuildInputs = [
      buildPkgs.rustc
      buildPkgs.patchelf
      crossCc
    ];

    buildPhase = ''
      runHook preBuild
      rustc \
        --edition=2021 \
        --sysroot=${crossSysroot} \
        --target=aarch64-unknown-linux-gnu \
        -C linker=${crossCc}/bin/${crossCc.targetPrefix}cc \
        -C opt-level=s \
        -C strip=symbols \
        -D warnings \
        ${./healthd.rs} \
        -o caliptra-fpga-healthd
      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall
      install -Dm755 caliptra-fpga-healthd $out/bin/caliptra-fpga-healthd
      # Re-point ELF interpreter & RPATH to target NixOS `pkgs.glibc` so we don't pull
      # a duplicate `pkgsCross` glibc into the FPGA SquashFS closure.
      patchelf \
        --set-interpreter "${pkgs.stdenv.cc.bintools.dynamicLinker}" \
        --set-rpath "${runtimeRpath}" \
        $out/bin/caliptra-fpga-healthd
      runHook postInstall
    '';
  }
