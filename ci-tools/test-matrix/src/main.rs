// Licensed under the Apache-2.0 license

use clap::Parser;
use octocrab::{etag::Etagged, params::actions::ArchiveFormat, Octocrab, Page};
use zip::result::ZipError;

use std::{
    io::{Cursor, Read},
    path::Path,
};

mod cli;
mod html;
mod junit;
mod test;

const NUM_RUNS: usize = 6;

async fn all_items<T: for<'de> serde::de::Deserialize<'de>>(
    octocrab: &Octocrab,
    etagged: Etagged<Page<T>>,
) -> Result<Vec<T>, octocrab::Error> {
    let mut result = vec![];
    let Some(mut page) = etagged.value else {
        panic!("etagged.value was not set; using api incorrectly?");
    };
    loop {
        result.extend(page.items);
        page = match octocrab.get_page(&page.next).await? {
            Some(next_page) => next_page,
            None => break,
        }
    }
    Ok(result)
}

fn zip_extract_file(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
) -> Result<Vec<u8>, ZipError> {
    let mut result = vec![];
    zip.by_name(name)?.read_to_end(&mut result)?;
    Ok(result)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let cli = cli::Cli::parse();

    let org = cli.gh_org.clone();
    let repo = cli.gh_repo.clone();
    let workflow = cli.gh_workflow.clone();

    let octocrab = Octocrab::builder().personal_token(cli.gh_token).build()?;
    let release_runs = octocrab
        .workflows(&org, &repo)
        .list_runs(&workflow)
        .branch("main")
        .send()
        .await?;
    log::info!("{}/{}:{}", org, repo, workflow);

    let run_infos: Vec<test::RunInfo> = release_runs
        .items
        .iter()
        .take(NUM_RUNS)
        .map(test::RunInfo::from_run)
        .collect();

    for (index, run) in release_runs.into_iter().take(NUM_RUNS).enumerate() {
        let artifacts = test::all_items(
            &octocrab,
            octocrab
                .actions()
                .list_workflow_run_artifacts(&org, &repo, run.id)
                .send()
                .await?,
        )
        .await?;
        let mut test_runs = vec![];
        for artifact in artifacts {
            if artifact.name.starts_with("caliptra-test-results") {
                let test_run_name = &artifact.name["caliptra-test-results-".len()..];
                let artifact_zip = octocrab
                    .actions()
                    .download_artifact(&org, &repo, artifact.id, ArchiveFormat::Zip)
                    .await?;

                let t = test::TestRun::from_zip_bytes(test_run_name.into(), &artifact_zip);
                match t {
                    Ok(test_run) => test_runs.push(test_run),
                    Err(e) => log::error!(
                        "Error processing test results for run {} artifact {}: {}",
                        run.id,
                        artifact.name,
                        e
                    ),
                }
            }
        }
        let matrix = test::TestMatrix::new(test_runs).unwrap();
        let html = html::format(&run, &run_infos, &matrix);
        std::fs::write(
            Path::new(&cli.www_out).join(format!("run-{}.html", test::RunInfo::from_run(&run).id)),
            &html,
        )
        .unwrap();
        log::info!(
            "{}/run-{}.html",
            cli.www_out,
            test::RunInfo::from_run(&run).id
        );

        if index == 0 {
            std::fs::write(Path::new(&cli.www_out).join("index.html"), &html).unwrap();
        }
    }

    Ok(())
}
