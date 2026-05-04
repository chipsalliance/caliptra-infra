// Licensed under the Apache-2.0 license

use std::path::PathBuf;

use anyhow::Result;
use caliptra_bitstream_downloader::{MANIFEST_SCHEMA_VERSION, Manifest};
use chrono::Utc;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Download a bitstream using a manifest file.
    Download(DownloadArgs),
    /// Create a bitstream manifest bundle.
    BundleManifest(BundleManifestArgs),
    /// Upload a bitstream bundle to Google Cloud Storage.
    UploadBundle(UploadBundleArgs),
}

#[derive(Parser, Debug)]
struct DownloadArgs {
    /// Path to the bitstream manifest
    #[arg(long = "bitstream-manifest", value_name = "FILE")]
    bitstream_manifest: PathBuf,
}

#[derive(Parser, Debug)]
struct BundleManifestArgs {
    /// The repository name (e.g., "chipsalliance/caliptra-mcu-sw")
    #[arg(long)]
    repository: String,

    /// Hardware major version (e.g., "2.0")
    #[arg(long)]
    hw_major_version: String,

    /// Target branch (e.g., "main")
    #[arg(long)]
    target_branch: String,

    /// Caliptra variant (e.g., "subsystem")
    #[arg(long)]
    caliptra_variant: String,

    /// Commit hash of the repository
    #[arg(long)]
    commit_hash: String,

    /// Caliptra SS commit hash
    #[arg(long)]
    caliptra_ss_commit: Option<String>,

    /// GitHub Actions job ID
    #[arg(long)]
    job_id: String,

    /// Whether the bitstream is segmented
    #[arg(long)]
    segmented: bool,

    /// Optional GitHub Pull Request number
    #[arg(long)]
    github_pr: Option<u32>,

    /// Path to the XSA file (optional)
    #[arg(long)]
    xsa_file: Option<PathBuf>,

    /// Path to the PDI file (optional)
    #[arg(long)]
    pdi_file: Option<PathBuf>,

    /// Output directory for the bundled .tar.gz
    #[arg(long)]
    output_dir: PathBuf,
}

#[derive(Parser, Debug)]
struct UploadBundleArgs {
    /// Path to the bitstream bundle (.tar.gz) to upload.
    #[arg(long = "bundle-path", value_name = "FILE")]
    bundle_path: PathBuf,

    /// Name of the GCS bucket to upload to.
    #[arg(long = "gcs-bucket", value_name = "BUCKET")]
    gcs_bucket: String,

    /// Path to the XSA file (optional)
    #[arg(long)]
    xsa_file: Option<PathBuf>,

    /// Path to the PDI file (optional)
    #[arg(long)]
    pdi_file: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Download(args) => {
            caliptra_bitstream_downloader::download_bitstream(&args.bitstream_manifest).await?;
        }
        Commands::BundleManifest(args) => {
            let date = Utc::now().to_rfc3339();
            let manifest = Manifest {
                schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
                repository: args.repository,
                hw_major_version: args.hw_major_version,
                target_branch: args.target_branch,
                caliptra_variant: args.caliptra_variant,
                date,
                commit_hash: args.commit_hash,
                caliptra_ss_commit: args.caliptra_ss_commit,
                job_id: args.job_id,
                segmented: args.segmented,
                github_pr: args.github_pr,
                name: None,
                xsa_url: None,
                pdi_url: None,
                xsa_hash: None,
                pdi_hash: None,
            };
            caliptra_bitstream_downloader::create_manifest_bundle(
                manifest,
                args.xsa_file,
                args.pdi_file,
                args.output_dir,
            )
            .await?;
        }
        Commands::UploadBundle(args) => {
            caliptra_bitstream_downloader::upload_manifest_bundle(
                &args.bundle_path,
                &args.gcs_bucket,
                args.xsa_file,
                args.pdi_file,
            )
            .await?;
        }
    }
    Ok(())
}
