// Licensed under the Apache-2.0 license

use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use google_cloud_storage::client::Storage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tar::Archive as TarArchive;
use tar::Builder as TarBuilder;
use tokio::fs::{self, File};

pub const MANIFEST_SCHEMA_VERSION: &str = "1";
pub const OUTPUT_BUNDLE_FILENAME: &str = "caliptra-bitstream.tar.gz";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub schema_version: String,
    pub repository: String,
    pub hw_major_version: String,
    pub target_branch: String,
    pub caliptra_variant: String,
    pub date: String,
    pub commit_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caliptra_ss_commit: Option<String>,
    pub job_id: String,
    pub segmented: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_pr: Option<u32>,
    // Fields for downloading a bitstream, optional when bundling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xsa_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdi_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bin_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xsa_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdi_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bin_hash: Option<String>,
}

impl Manifest {
    pub fn from_toml(content: &str) -> Result<Self> {
        let manifest: Self = toml::from_str(content).context("failed to parse manifest TOML")?;
        if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
            anyhow::bail!("Unsupported schema version: {}", manifest.schema_version);
        }
        Ok(manifest)
    }

    pub async fn load_from_path(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .await
            .context("failed to read manifest file")?;
        Self::from_toml(&content)
    }

    pub fn to_toml(&self) -> Result<String> {
        let mut manifest = self.clone();
        if let Some(url) = &mut manifest.xsa_url {
            *url = url.replace("/projects/_/buckets/", "/");
        }
        if let Some(url) = &mut manifest.pdi_url {
            *url = url.replace("/projects/_/buckets/", "/");
        }
        if let Some(url) = &mut manifest.bin_url {
            *url = url.replace("/projects/_/buckets/", "/");
        }
        Ok(format!(
            "# Licensed under the Apache-2.0 license\n{}",
            toml::to_string(&manifest).context("failed to serialize manifest to TOML")?
        ))
    }
}

fn calculate_hash<R: io::Read>(mut reader: R) -> Result<String> {
    let mut hasher = Sha256::new();
    io::copy(&mut reader, &mut hasher).context("failed to read content for hashing")?;
    Ok(hex::encode(hasher.finalize()))
}

// Upload file contents to cloud storage.
async fn upload_content_to_gcs(
    content: File,
    bucket: &str,
    object_name: &str,
    commit_hash: &str,
    namespace: &str,
) -> Result<String> {
    let object_name = if namespace.is_empty() {
        format!("v{MANIFEST_SCHEMA_VERSION}/{commit_hash}/{object_name}")
    } else {
        format!("v{MANIFEST_SCHEMA_VERSION}/{commit_hash}/{namespace}/{object_name}")
    };
    let client = Storage::builder().build().await?;
    client
        .write_object(
            format!("projects/_/buckets/{bucket}"),
            &object_name,
            content,
        )
        .send_buffered()
        .await?;
    let public_url = format!(
        "https://storage.googleapis.com/{}/{}",
        bucket, object_name
    );
    println!("Uploaded {} to: {}", &object_name, public_url);
    Ok(public_url)
}

pub async fn download_bitstream(manifest_path: &Path) -> Result<PathBuf> {
    let manifest = Manifest::load_from_path(manifest_path).await?;

    let (bitstream_url, bitstream_hash, extension) = if let (Some(url), Some(hash)) =
        (manifest.pdi_url.as_deref(), manifest.pdi_hash.as_deref())
    {
        (url, hash, "pdi")
    } else if let (Some(url), Some(hash)) =
        (manifest.bin_url.as_deref(), manifest.bin_hash.as_deref())
    {
        (url, hash, "bin")
    } else if let (Some(url), Some(hash)) =
        (manifest.xsa_url.as_deref(), manifest.xsa_hash.as_deref())
    {
        (url, hash, "xsa")
    } else {
        bail!("Manifest is missing 'pdi_url', 'bin_url', and 'xsa_url' fields for download");
    };

    // Use the name from the manifest if available, otherwise default to a generic name
    let bitstream_name = manifest.name.as_deref().unwrap_or("bitstream");

    println!("Downloading bitstream: {}", bitstream_name);
    println!("URL: {}", bitstream_url);

    let response = reqwest::get(bitstream_url)
        .await
        .context("failed to make request")?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.text().await.unwrap_or_default();
        anyhow::bail!("HTTP request failed with status {}: {}", status, error_body);
    }

    let bytes = response
        .bytes()
        .await
        .context("failed to read response bytes")?;

    let calculated_hash_hex = calculate_hash(&bytes[..])?;

    println!("Expected hash: {}", bitstream_hash);
    println!("Calculated hash: {}", calculated_hash_hex);

    if calculated_hash_hex != bitstream_hash {
        bail!(
            "hash mismatch expected: {}, got: {}",
            bitstream_hash,
            calculated_hash_hex
        );
    }
    println!("Hash verification successful!");

    let output_path = if extension == "bin" {
        PathBuf::from("./caliptra_build/caliptra_fpga.bin")
    } else {
        PathBuf::from(format!("{}.{}", manifest.caliptra_variant, extension))
    };

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)
                .await
                .context("failed to create output directory")?;
        }
    }

    let mut file = fs::File::create(&output_path)
        .await
        .context("failed to create output file")?;

    use tokio::io::AsyncWriteExt;
    file.write_all(&bytes)
        .await
        .context("failed to write output file")?;
    println!("File saved to: {}", output_path.display());
    Ok(output_path)
}

fn add_file_to_tar<W: io::Write>(tar: &mut TarBuilder<W>, path: &Path) -> Result<String> {
    let file_name = path.file_name().context("Invalid file path")?;
    tar.append_path_with_name(path, file_name)
        .context("Failed to add file to tar archive")?;

    let file = std::fs::File::open(path)?;
    calculate_hash(file)
}

pub async fn create_manifest_bundle(
    manifest: Manifest,
    xsa_path: Option<PathBuf>,
    pdi_path: Option<PathBuf>,
    bin_path: Option<PathBuf>,
    output_dir: PathBuf,
) -> Result<PathBuf> {
    let mut manifest = manifest;
    let tmp_dir = tempfile::tempdir().context("Failed to create temporary directory")?;

    if !output_dir.exists() {
        anyhow::bail!("{} did not exist!", output_dir.display());
    }

    if xsa_path.is_none() && pdi_path.is_none() && bin_path.is_none() {
        anyhow::bail!("At least one bitstream file (XSA, PDI, or BIN) must be provided");
    }

    let output_bundle_path = output_dir.join(OUTPUT_BUNDLE_FILENAME);

    {
        let output_bundle_path = output_bundle_path.clone();
        let res = tokio::task::spawn_blocking(move || -> Result<()> {
            let file = std::fs::File::create(&output_bundle_path)
                .context("Failed to create output tar.gz file")?;
            let enc = GzEncoder::new(file, Compression::default());
            let mut tar = TarBuilder::new(enc);

            if let Some(xsa_path) = xsa_path {
                if !xsa_path.exists() {
                    anyhow::bail!("XSA file specified but does not exist: {}", xsa_path.display());
                }
                manifest.xsa_hash = Some(add_file_to_tar(&mut tar, &xsa_path)?);
            }

            if let Some(pdi_path) = pdi_path {
                if !pdi_path.exists() {
                    anyhow::bail!("PDI file specified but does not exist: {}", pdi_path.display());
                }
                manifest.pdi_hash = Some(add_file_to_tar(&mut tar, &pdi_path)?);
            }

            if let Some(bin_path) = bin_path {
                if !bin_path.exists() {
                    anyhow::bail!("BIN file specified but does not exist: {}", bin_path.display());
                }
                manifest.bin_hash = Some(add_file_to_tar(&mut tar, &bin_path)?);
            }

            let manifest_toml = manifest.to_toml()?;
            let manifest_path = tmp_dir.path().join("manifest.toml");
            std::fs::write(&manifest_path, manifest_toml)
                .context("Failed to write manifest.toml to temporary directory")?;

            tar.append_path_with_name(&manifest_path, "manifest.toml")
                .context("Failed to add manifest.toml to tar archive")?;

            tar.finish().context("Failed to finish tar archive")?;
            Ok(())
        })
        .await
        .context("Blocking task for bundle creation failed")?;
        res?;
    }

    println!(
        "Manifest bundle created at: {}",
        output_bundle_path.display()
    );

    Ok(output_bundle_path)
}

async fn upload_component_to_gcs(path: &Path, bucket: &str, commit_hash: &str, namespace: &str) -> Result<String> {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .context("Invalid file name")?;
    let file = File::open(path)
        .await
        .context("Failed to open file for upload")?;
    upload_content_to_gcs(file, bucket, &file_name, commit_hash, namespace).await
}

async fn find_file_with_extension(dir: &Path, extension: &str) -> Result<Option<PathBuf>> {
    let mut match_path = None;
    let mut dir = fs::read_dir(dir).await?;
    while let Some(entry) = dir.next_entry().await? {
        let path = entry.path();
        if path.is_file() && path.extension().map_or(false, |ext| ext == extension) {
            if match_path.is_some() {
                anyhow::bail!(
                    "Found multiple files with extension .{} in tarball",
                    extension
                );
            }
            match_path = Some(path);
        }
    }
    Ok(match_path)
}

pub async fn upload_manifest_bundle(bundle_path: &Path, gcs_bucket: &str, namespace: &str) -> Result<()> {
    println!("Uploading manifest bundle from: {}", bundle_path.display());

    let tmp_dir = tempfile::tempdir()
        .context("Failed to create temporary directory for bundle extraction")?;
    let tmp_path = tmp_dir.path().to_path_buf();

    let bundle_path_clone = bundle_path.to_path_buf();
    let res = tokio::task::spawn_blocking(move || -> Result<()> {
        let tar_gz = std::fs::File::open(&bundle_path_clone).context(format!(
            "Failed to open bundle file: {}",
            bundle_path_clone.display()
        ))?;
        let tar = GzDecoder::new(tar_gz);
        let mut archive = TarArchive::new(tar);
        archive
            .unpack(&tmp_path)
            .context("Failed to unpack bundle archive")?;

        Ok(())
    })
    .await
    .context("Blocking task for bundle extraction failed")?;

    res?;

    let manifest_path = tmp_dir.path().join("manifest.toml");
    let mut manifest = Manifest::load_from_path(&manifest_path).await?;

    let xsa_file = find_file_with_extension(tmp_dir.path(), "xsa").await?;
    let pdi_file = find_file_with_extension(tmp_dir.path(), "pdi").await?;
    let bin_file = find_file_with_extension(tmp_dir.path(), "bin").await?;

    if xsa_file.is_none() && pdi_file.is_none() && bin_file.is_none() {
        anyhow::bail!("Manifest bundle is missing required bitstream file (XSA, PDI, or BIN)");
    }

    if let Some(file) = &xsa_file {
        println!("Found XSA file in tarball: {}", file.display());
        manifest.xsa_url =
            Some(upload_component_to_gcs(file, gcs_bucket, &manifest.commit_hash, namespace).await?);
    }

    if let Some(file) = &pdi_file {
        println!("Found PDI file in tarball: {}", file.display());
        manifest.pdi_url =
            Some(upload_component_to_gcs(file, gcs_bucket, &manifest.commit_hash, namespace).await?);
    }

    if let Some(file) = &bin_file {
        println!("Found BIN file in tarball: {}", file.display());
        manifest.bin_url =
            Some(upload_component_to_gcs(file, gcs_bucket, &manifest.commit_hash, namespace).await?);
    }

    manifest.name = Some(format!("{}-bitstream", manifest.caliptra_variant));

    fs::write(&manifest_path, manifest.to_toml()?)
        .await
        .context("Failed to write updated manifest to temporary directory")?;

    upload_content_to_gcs(
        File::open(&manifest_path)
            .await
            .context("Failed to read updated manifest file content")?,
        gcs_bucket,
        "manifest.toml",
        &manifest.commit_hash,
        namespace,
    )
    .await?;

    println!("Successfully uploaded bundle to gs://{}", gcs_bucket);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn test_manifest() -> Manifest {
        Manifest {
            schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
            repository: "chipsalliance/caliptra-infra".to_string(),
            hw_major_version: "2.0".to_string(),
            target_branch: "main".to_string(),
            caliptra_variant: "subsystem".to_string(),
            date: "2026-07-13T00:00:00Z".to_string(),
            commit_hash: "1234567890abcdef".to_string(),
            caliptra_ss_commit: None,
            job_id: "100".to_string(),
            segmented: true,
            github_pr: None,
            name: None,
            xsa_url: None,
            pdi_url: None,
            bin_url: None,
            xsa_hash: None,
            pdi_hash: None,
            bin_hash: None,
        }
    }

    #[tokio::test]
    async fn test_create_manifest_bundle_xsa_only() -> Result<()> {
        let tmp_dir = tempfile::tempdir()?;
        let xsa_path = tmp_dir.path().join("system.xsa");
        std::fs::File::create(&xsa_path)?.write_all(b"dummy xsa content")?;

        let output_dir = tmp_dir.path().join("output");
        std::fs::create_dir(&output_dir)?;

        let bundle_path = create_manifest_bundle(
            test_manifest(),
            Some(xsa_path),
            None,
            None,
            output_dir.clone(),
        )
        .await?;

        assert!(bundle_path.exists());

        // Extract tar.gz and verify contents
        let extract_dir = tmp_dir.path().join("extract");
        let tar_gz = std::fs::File::open(&bundle_path)?;
        let tar = GzDecoder::new(tar_gz);
        let mut archive = TarArchive::new(tar);
        archive.unpack(&extract_dir)?;

        let manifest_path = extract_dir.join("manifest.toml");
        assert!(manifest_path.exists());
        let unpacked_manifest = Manifest::load_from_path(&manifest_path).await?;

        assert!(unpacked_manifest.xsa_hash.is_some());
        assert!(unpacked_manifest.pdi_hash.is_none());
        assert!(extract_dir.join("system.xsa").exists());
        assert!(!extract_dir.join("system.pdi").exists());

        Ok(())
    }

    #[tokio::test]
    async fn test_create_manifest_bundle_xsa_and_pdi() -> Result<()> {
        let tmp_dir = tempfile::tempdir()?;
        let xsa_path = tmp_dir.path().join("system.xsa");
        let pdi_path = tmp_dir.path().join("subsystem.pdi");
        std::fs::File::create(&xsa_path)?.write_all(b"dummy xsa content")?;
        std::fs::File::create(&pdi_path)?.write_all(b"dummy pdi content")?;

        let output_dir = tmp_dir.path().join("output");
        std::fs::create_dir(&output_dir)?;

        let bundle_path = create_manifest_bundle(
            test_manifest(),
            Some(xsa_path),
            Some(pdi_path),
            None,
            output_dir.clone(),
        )
        .await?;

        assert!(bundle_path.exists());

        // Extract tar.gz and verify contents
        let extract_dir = tmp_dir.path().join("extract");
        let tar_gz = std::fs::File::open(&bundle_path)?;
        let tar = GzDecoder::new(tar_gz);
        let mut archive = TarArchive::new(tar);
        archive.unpack(&extract_dir)?;

        let manifest_path = extract_dir.join("manifest.toml");
        assert!(manifest_path.exists());
        let unpacked_manifest = Manifest::load_from_path(&manifest_path).await?;

        assert!(unpacked_manifest.xsa_hash.is_some());
        assert!(unpacked_manifest.pdi_hash.is_some());
        assert!(extract_dir.join("system.xsa").exists());
        assert!(extract_dir.join("subsystem.pdi").exists());

        Ok(())
    }

    #[tokio::test]
    async fn test_create_manifest_bundle_bin_only() -> Result<()> {
        let tmp_dir = tempfile::tempdir()?;
        let bin_path = tmp_dir.path().join("caliptra_fpga.bin");
        std::fs::File::create(&bin_path)?.write_all(b"dummy bin content")?;

        let output_dir = tmp_dir.path().join("output");
        std::fs::create_dir(&output_dir)?;

        let bundle_path = create_manifest_bundle(
            test_manifest(),
            None,
            None,
            Some(bin_path),
            output_dir.clone(),
        )
        .await?;

        assert!(bundle_path.exists());

        // Extract tar.gz and verify contents
        let extract_dir = tmp_dir.path().join("extract");
        let tar_gz = std::fs::File::open(&bundle_path)?;
        let tar = GzDecoder::new(tar_gz);
        let mut archive = TarArchive::new(tar);
        archive.unpack(&extract_dir)?;

        let manifest_path = extract_dir.join("manifest.toml");
        assert!(manifest_path.exists());
        let unpacked_manifest = Manifest::load_from_path(&manifest_path).await?;

        assert!(unpacked_manifest.bin_hash.is_some());
        assert!(unpacked_manifest.xsa_hash.is_none());
        assert!(unpacked_manifest.pdi_hash.is_none());
        assert!(extract_dir.join("caliptra_fpga.bin").exists());

        Ok(())
    }

    #[test]
    fn test_manifest_to_toml_sanitizes_urls() -> Result<()> {
        let mut manifest = test_manifest();
        manifest.xsa_url = Some("https://storage.googleapis.com/my-bucket/projects/_/buckets/v1/hash/system.xsa".to_string());
        manifest.pdi_url = Some("https://storage.googleapis.com/my-bucket/projects/_/buckets/v1/hash/subsystem.pdi".to_string());
        manifest.bin_url = Some("https://storage.googleapis.com/my-bucket/projects/_/buckets/v1/hash/caliptra_fpga.bin".to_string());

        let toml_str = manifest.to_toml()?;
        assert!(!toml_str.contains("/_/buckets/"));
        assert!(toml_str.contains("https://storage.googleapis.com/my-bucket/v1/hash/system.xsa"));
        assert!(toml_str.contains("https://storage.googleapis.com/my-bucket/v1/hash/subsystem.pdi"));
        assert!(toml_str.contains("https://storage.googleapis.com/my-bucket/v1/hash/caliptra_fpga.bin"));
        Ok(())
    }
}

