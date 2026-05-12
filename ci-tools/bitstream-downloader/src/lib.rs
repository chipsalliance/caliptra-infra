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

#[derive(Debug, Deserialize, Serialize)]
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
    pub xsa_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdi_hash: Option<String>,
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
        Ok(format!(
            "# Licensed under the Apache-2.0 license\n{}",
            toml::to_string(self).context("failed to serialize manifest to TOML")?
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
) -> Result<String> {
    let object_name = format!("v{MANIFEST_SCHEMA_VERSION}/{commit_hash}/{object_name}");
    let client = Storage::builder().build().await?;
    let response = client
        .write_object(
            format!("projects/_/buckets/{bucket}"),
            &object_name,
            content,
        )
        .send_buffered()
        .await?;
    let public_url = format!(
        "https://storage.googleapis.com/{}/{}",
        bucket, response.name
    );
    println!("Uploaded {} to: {}", &object_name, public_url);
    Ok(public_url)
}

pub async fn download_bitstream(manifest_path: &Path) -> Result<PathBuf> {
    let manifest = Manifest::load_from_path(manifest_path).await?;

    let bitstream_url = manifest
        .pdi_url
        .as_deref()
        .context("Manifest is missing 'pdi_url' field for download")?;
    let bitstream_hash = manifest
        .pdi_hash
        .as_deref()
        .context("Manifest is missing 'pdi_hash' field for download")?;

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

    let output_filename = format!("{}.pdi", manifest.caliptra_variant);
    let output_path = PathBuf::from(&output_filename);
    let mut file = fs::File::create(&output_path)
        .await
        .context("failed to create output file")?;

    use tokio::io::AsyncWriteExt;
    file.write_all(&bytes)
        .await
        .context("failed to write output file")?;
    println!("PDI saved to: {}", output_filename);
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
    output_dir: PathBuf,
) -> Result<PathBuf> {
    let mut manifest = manifest;
    let tmp_dir = tempfile::tempdir().context("Failed to create temporary directory")?;

    if !output_dir.exists() {
        anyhow::bail!("{} did not exist!", output_dir.display());
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
                manifest.xsa_hash = Some(add_file_to_tar(&mut tar, &xsa_path)?);
            }

            if let Some(pdi_path) = pdi_path {
                manifest.pdi_hash = Some(add_file_to_tar(&mut tar, &pdi_path)?);
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

async fn upload_component_to_gcs(path: &Path, bucket: &str, commit_hash: &str) -> Result<String> {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .context("Invalid file name")?;
    let file = File::open(path)
        .await
        .context("Failed to open file for upload")?;
    upload_content_to_gcs(file, bucket, &file_name, commit_hash).await
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

pub async fn upload_manifest_bundle(bundle_path: &Path, gcs_bucket: &str) -> Result<()> {
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

    if let Some(file) = &xsa_file {
        println!("Found XSA file in tarball: {}", file.display());
    }
    if let Some(file) = &pdi_file {
        println!("Found PDI file in tarball: {}", file.display());
    }

    if let Some(file) = xsa_file {
        manifest.xsa_url =
            Some(upload_component_to_gcs(&file, gcs_bucket, &manifest.commit_hash).await?);
    }

    if let Some(file) = pdi_file {
        manifest.pdi_url =
            Some(upload_component_to_gcs(&file, gcs_bucket, &manifest.commit_hash).await?);
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
    )
    .await?;

    println!("Successfully uploaded bundle to gs://{}", gcs_bucket);

    Ok(())
}
