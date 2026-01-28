# Caliptra Infrastructure

This repository contains infrastructure, CI tools, and automation scripts for the Caliptra project.

## Tools

The `ci-tools` directory contains various utilities used across the project.

- **bitstream-downloader**: A utility for downloading Caliptra bitstreams based on a TOML manifest file.
- **build-image**: Contains the Dockerfile and configuration for the docker image used to cross-compile binaries for Caliptra FPGAs.
- **file-header-fix**: A tool to check and automatically fix file headers to ensure they contain the required Apache-2.0 license text.
- **fpga-boss**: A utility for controlling FPGA boards (flashing, resetting, and UART communication), specifically for testing Caliptra firmware on zcu104 boards.
- **fpga-image**: Scripts to generate SD card images that can boot on zcu104 Zynq FPGA development boards for testing.
 - VCK-190 images are supported in `chipsalliance/caliptra-sw` and will eventually be ported to this repository.
- **github-runner**: Infrastructure and code for launching self-hosted GitHub Actions runners within ephemeral Google Compute Engine VMs.
- **host-runner**: NixOS configuration for the Raspberry Pi "host runners" that manage and oversee clusters of FPGA boards.
- **release**: Scripts and tools for generating release artifacts, including calculating RTL hashes and packaging ROM/FMC binaries.
- **size-history**: A tool that tracks and reports the size history of firmware artifacts, useful for monitoring bloat over time.
- **test-matrix**: Generates HTML test matrix reports from JUnit XML test results to visualize test execution across different configurations.
-  **test-printer**: A simple CLI tool that parses JUnit XML files and prints a formatted summary table of test results to the console.
