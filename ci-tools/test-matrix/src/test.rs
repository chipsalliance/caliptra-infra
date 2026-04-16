// Licensed under the Apache-2.0 license

use nextest_metadata::TestListSummary;

use std::{
    collections::BTreeMap,
    error::Error,
    io::{Cursor, Read},
};

use octocrab::{models::workflows::Run, Octocrab, Page, etag::Etagged};
use serde::Serialize;
use zip::result::ZipError;

use crate::junit;


pub async fn all_items<T: for<'de> serde::de::Deserialize<'de>>(
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

#[derive(Debug, serde::Serialize)]
pub struct TestMatrix {
    pub rows: Vec<TestSuite>,
    pub columns: Vec<String>,
}

fn get_at_index_mut<T: Default>(vec: &mut Vec<T>, index: usize) -> &mut T {
    if index >= vec.len() {
        vec.resize_with(index + 1, Default::default);
    }
    &mut vec[index]
}

#[derive(Debug, serde::Serialize)]
pub struct TestSuite {
    pub name: String,
    pub rows: Vec<TestCaseRow>,
}

impl TestMatrix {
    pub (crate) fn new(mut runs: Vec<TestRun>) -> Result<Self, Box<dyn Error + 'static>> {
        runs.sort_by(|a, b| a.name.cmp(&b.name));
        let mut columns = vec![];
        let mut row_map: BTreeMap<String, BTreeMap<String, TestCaseRow>> = BTreeMap::new();
        let runs_len = runs.len();
        for (run_index, run) in runs.into_iter().enumerate() {
            columns.push(run.name);
            for (suite_name, suite) in run.test_list.rust_suites {
                let suite_map = row_map.entry(suite_name.to_string()).or_default();
                for (test_case_name, test_case) in suite.test_cases {
                    let row =
                        suite_map
                            .entry(test_case_name.clone())
                            .or_insert_with(|| TestCaseRow {
                                name: test_case_name,
                                cells: vec![None; runs_len],
                            });
                    *get_at_index_mut(&mut row.cells, run_index) = Some(TestCaseCell {
                        status: if test_case.ignored {
                            TestCaseStatus::Ignored
                        } else {
                            TestCaseStatus::Unknown
                        },
                        output: Default::default(),
                        duration: Default::default(),
                    });
                }
            }
            for suite in run.junit_suites.test_suites {
                let suite_map = row_map
                    .get_mut(&suite.name)
                    .ok_or_else(|| format!("Unknown suite in junit file: {}", suite.name))?;
                for test_case in suite.test_cases {
                    let row = suite_map.get_mut(&test_case.name).ok_or_else(|| {
                        format!("Unknown test-case in junit file: {}", test_case.name)
                    })?;
                    let cell = get_at_index_mut(&mut row.cells, run_index)
                        .as_mut()
                        .ok_or_else(|| {
                            format!(
                                "Unknown test-case for this run in junit file: {}",
                                test_case.name
                            )
                        })?;
                    cell.status = test_case.status();
                    cell.output = test_case.output_truncated();
                    cell.duration = test_case.time;
                }
            }
        }
        let rows: Vec<_> = row_map
            .into_iter()
            .map(|(name, rows)| TestSuite {
                name,
                rows: rows.into_values().collect(),
            })
            .collect();
        Ok(Self { rows, columns })
    }
}

#[derive(Debug, Default, serde::Serialize)]
pub (crate)struct TestCaseRow {
    pub name: String,
    pub cells: Vec<Option<TestCaseCell>>,
}

#[derive(Copy, Clone, Debug, serde::Serialize)]
pub (crate) enum TestCaseStatus {
    Passed,
    Failed,
    Ignored,
    Unknown,
}

#[derive(Clone, Debug, serde::Serialize)]
pub (crate) struct TestCaseCell {
    pub status: TestCaseStatus,
    pub output: String,
    pub duration: f64,
}

pub (crate) struct TestRun {
    name: String,

    // The metadata about the tests to run (includes tests that were ignored)
    test_list: TestListSummary,

    // The results of the tests that were run (doesn't include ignored tests)
    junit_suites: junit::TestSuites,
}

impl TestRun {
    pub (crate) fn from_zip_bytes(name: String, bytes: &[u8]) -> Result<TestRun, Box<dyn Error>> {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
        let json = zip_extract_file(&mut archive, "nextest-list.json")?;
        let json = String::from_utf8(json)?;
        let test_list = nextest_metadata::TestListSummary::parse_json(json)?;
        let junit_xml = String::from_utf8(zip_extract_file(&mut archive, "junit.xml")?)?;
        let junit_suites = junit::TestSuites::from_xml(&junit_xml)?;

        Ok(TestRun {
            name,
            test_list,
            junit_suites,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RunInfo {
    pub id: String,
    pub display_name: String,
}

impl RunInfo {
    pub (crate) fn from_run(run: &Run) -> Self {
        RunInfo {
            id: run.created_at.format("%F-%H%M%S").to_string(),
            display_name: run.created_at.format("%F %H:%M:%S").to_string(),
        }
    }
}
