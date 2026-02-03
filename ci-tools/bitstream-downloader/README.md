# Caliptra bitstream downloader

A utility for downloading Caliptra bitstreams from a TOML manifest file.

## Usage

### Download latest core bitstream

``bash
$ # This example uses the bitstream toml from the "caliptra-sw" root directory.
$ cargo r  -- --bitstream-manifest ~/caliptra-sw/hw/fpga/bitstream_manifests/core.toml
```

## Schema

The schema for the bitstream manifest is simple but will likely evolve as we build out the Caliptra infrastructure. 

Currently it looks like this:

```toml
[bitstream]
name = "subsystem-2.1" # The name for the manifest
url = "" # URL to the hosted bitstream file. Generally a GCP bucket.
hash = "ae9097dc22c10bf919599573c8fb25a074d926d8e922df9bf74ccdbca05b07e8" # A SHA-256 hash of the bitstream file.
caliptra_variant = "subsystem" # Metadata describing the bitstream variant, for example "core" or "subsystem".
caliptra_rtl_commit = "33b959ea4990783e79cd5d463adc0c1e4dad0298" # The Caliptra RTL commit used to build the bitstream.
```
