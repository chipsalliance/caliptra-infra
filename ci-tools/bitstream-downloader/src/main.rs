// Licensed under the Apache-2.0 license

use std::env;
use std::path::PathBuf;

use anyhow::{Result, bail};
use caliptra_bitstream_downloader::builder::{
    BuildOptions, BuildSelector, BundleOptions, Variant, format_variants_table, resolve_repo_root,
    run_build, run_bundle, run_ci_matrix,
};
use caliptra_bitstream_downloader::{MANIFEST_SCHEMA_VERSION, Manifest};
use chrono::Utc;
use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None, arg_required_else_help = true)]
struct Cli {
    /// Repository root directory (auto-detected if omitted)
    #[arg(long, global = true, value_name = "PATH")]
    repo_root: Option<PathBuf>,

    /// Print all supported FPGA variants, releases, and subtargets
    #[arg(long)]
    list: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Download a bitstream using a manifest file.
    Download(DownloadArgs),
    /// Create a bitstream manifest bundle.
    BundleManifest(BundleManifestArgs),
    /// Upload a bitstream bundle to Google Cloud Storage.
    UploadBundle(UploadBundleArgs),
    /// Print all supported FPGA variants, releases, and subtargets.
    List,
    /// Build one or more FPGA bitstreams with Vivado.
    Build(BuildArgs),
    /// Compute GitHub Actions build matrix and write outputs.
    CiMatrix,
    /// Validate staged Vivado outputs and create a manifest bundle.
    Bundle(BundleArgs),
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

    /// Path to the BIN file (optional)
    #[arg(long)]
    bin_file: Option<PathBuf>,

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

    /// Optional GCS path namespace (subfolder). When set, all objects for this bundle
    /// are uploaded under v1/{sha}/{namespace}/. Use the caliptra variant name to
    /// avoid collisions when uploading multiple variants from the same commit.
    /// Defaults to empty (no subfolder) for backwards compatibility.
    #[arg(long = "namespace", value_name = "NAMESPACE", default_value = "")]
    namespace: String,
}

#[derive(Args, Debug)]
struct BuildArgs {
    /// Enable the opt-in dual-iTRNG lab image (2.2/subsystem only)
    #[arg(long)]
    dual_itrng: bool,
    /// Print commands without initializing submodules or invoking Vivado
    #[arg(long)]
    dry_run: bool,

    /// Copy built .bin / .xsa / .pdi outputs from caliptra_build/ into <DIR>
    #[arg(long, value_name = "DIR")]
    staging_dir: Option<PathBuf>,

    /// Variant name, release (1.x/2.0/2.1/2.2), subtarget (e.g. 2.2/subsystem), or `all`
    selector: BuildSelector,
}

#[derive(Args, Debug)]
struct BundleArgs {
    /// Variant name from `list`
    #[arg(long, value_enum, value_name = "VARIANT")]
    variant: Variant,

    /// Directory containing staged Vivado outputs
    #[arg(long, value_name = "DIR")]
    staging_dir: PathBuf,

    /// Output directory for `caliptra-bitstream.tar.gz`
    #[arg(long, default_value = "manifest-bundle", value_name = "DIR")]
    output_dir: PathBuf,

    /// Repository name (defaults to $GITHUB_REPOSITORY)
    #[arg(long, value_name = "REPO")]
    repository: Option<String>,

    /// Target branch (defaults to $GITHUB_REF_NAME)
    #[arg(long, value_name = "BRANCH")]
    target_branch: Option<String>,

    /// Commit hash (defaults to $GITHUB_SHA)
    #[arg(long, value_name = "SHA")]
    commit_hash: Option<String>,

    /// GitHub Actions job ID (defaults to ${GITHUB_RUN_ID}-${GITHUB_JOB})
    #[arg(long, value_name = "ID")]
    job_id: Option<String>,
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|s| !s.is_empty())
}

fn resolve_bundle_options(args: BundleArgs) -> Result<BundleOptions> {
    let repository = args
        .repository
        .filter(|s| !s.is_empty())
        .or_else(|| non_empty_env("GITHUB_REPOSITORY"))
        .ok_or_else(|| anyhow::anyhow!("missing --repository or GITHUB_REPOSITORY"))?;
    let target_branch = args
        .target_branch
        .filter(|s| !s.is_empty())
        .or_else(|| non_empty_env("GITHUB_REF_NAME"))
        .ok_or_else(|| anyhow::anyhow!("missing --target-branch or GITHUB_REF_NAME"))?;
    let commit_hash = args
        .commit_hash
        .filter(|s| !s.is_empty())
        .or_else(|| non_empty_env("GITHUB_SHA"))
        .ok_or_else(|| anyhow::anyhow!("missing --commit-hash or GITHUB_SHA"))?;

    let job_id = if let Some(explicit) = args.job_id.filter(|s| !s.is_empty()) {
        explicit
    } else {
        let run_id = non_empty_env("GITHUB_RUN_ID");
        let job = non_empty_env("GITHUB_JOB");
        match (run_id, job) {
            (Some(r), Some(j)) => format!("{r}-{j}"),
            (Some(r), None) => format!("{r}-"),
            (None, Some(j)) => format!("-{j}"),
            (None, None) => bail!("missing --job-id or GITHUB_RUN_ID/GITHUB_JOB"),
        }
    };

    Ok(BundleOptions {
        variant: args.variant,
        staging_dir: args.staging_dir,
        output_dir: args.output_dir,
        repository,
        target_branch,
        commit_hash,
        job_id,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.list {
        if cli.command.is_some() {
            bail!("--list cannot be combined with a subcommand");
        }
        print!("{}", format_variants_table());
        return Ok(());
    }

    let Some(command) = cli.command else {
        unreachable!("clap arg_required_else_help handles missing --list/subcommand");
    };

    match command {
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
                bin_url: None,
                xsa_hash: None,
                pdi_hash: None,
                bin_hash: None,
            };
            caliptra_bitstream_downloader::create_manifest_bundle(
                manifest,
                args.xsa_file,
                args.pdi_file,
                args.bin_file,
                args.output_dir,
            )
            .await?;
        }
        Commands::UploadBundle(args) => {
            caliptra_bitstream_downloader::upload_manifest_bundle(
                &args.bundle_path,
                &args.gcs_bucket,
                &args.namespace,
            )
            .await?;
        }
        Commands::List => {
            print!("{}", format_variants_table());
        }
        Commands::Build(args) => {
            let repo_root = resolve_repo_root(cli.repo_root.as_deref())?;
            run_build(
                &repo_root,
                &BuildOptions {
                    selector: args.selector,
                    dry_run: args.dry_run,
                    dual_itrng: args.dual_itrng,
                    staging_dir: args.staging_dir,
                },
            )?;
        }
        Commands::CiMatrix => {
            let repo_root = resolve_repo_root(cli.repo_root.as_deref())?;
            run_ci_matrix(&repo_root)?;
        }
        Commands::Bundle(args) => {
            let repo_root = resolve_repo_root(cli.repo_root.as_deref())?;
            let opts = resolve_bundle_options(args)?;
            run_bundle(&repo_root, &opts).await?;
        }
    }
    Ok(())
}
