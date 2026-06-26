// Licensed under the Apache-2.0 license

use clap::Parser;
use serde::Deserialize;
use std::fs;
use serde_json::Value;
use std::collections::HashSet;

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// The path to the junit.xml file
    #[clap(short, long)]
    xml_path: String,
    /// The path to the list.json file
    #[clap(short, long)]
    json_path: Option<String>,
    /// Error if any test takes longer than 3 minutes
    #[clap(long)]
    check_test_time: bool,
}

#[derive(Debug, Deserialize, Default)]
struct TestSuites {
    #[serde(rename = "testsuite", default)]
    testsuites: Vec<TestSuite>,
}

#[derive(Debug, Deserialize, Default)]
struct TestSuite {
    #[serde(rename = "@name", default)]
    name: String,
    #[serde(rename = "testcase", default)]
    testcases: Vec<TestCase>,
}

#[derive(Debug, Deserialize, Default)]
struct TestCase {
    #[serde(rename = "@name", default)]
    name: String,
    #[serde(rename = "@time", default)]
    time: Option<f64>,
    #[serde(rename = "failure", default)]
    failure: Option<Failure>,
    #[serde(rename = "rerunFailure", default)]
    rerun_failures: Vec<RerunFailure>,
}

#[derive(Debug, Deserialize, Default)]
struct Failure {}

#[derive(Debug, Deserialize, Default)]
struct RerunFailure {}

#[derive(Clone, Copy)]
enum TestStatus {
    Failed,
    Retried,
    Passed,
}

struct TestResult {
    suite_name: String,
    case_name: String,
    status: TestStatus,
    status_icon: &'static str,
    time: f64,
}

fn parse_list_json(json_path: String) -> HashSet<String> {
    let list_json = fs::read_to_string(json_path).expect("Unable to read list.json");
    let test_list: Value = serde_json::from_str(&list_json).expect("Unable to parse JSON");
    let mut list_set: HashSet<String> = HashSet::new();

    if let Some(suites) = test_list["rust-suites"].as_object() {
        for suite in suites.keys() {
            if let Some(testcases) = test_list["rust-suites"][suite]["testcases"].as_object() {
                for case in testcases.keys() {
                    list_set.insert(format!("{}::{}", suite, case));
                }
            }
        }
    }

    list_set
}

fn validate_result_list(list_set: &HashSet<String>, run_set: &HashSet<String>) -> Result<(), String> {
    let mut diff: Vec<&String> = list_set.difference(&run_set).collect();
    diff.sort_unstable();
    if diff.len() > 0 {
        println!("Tests not executed: {:#?}", diff);
        return Err(format!("tests not executed = {}", diff.len()));
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let args = Args::parse();

    let junit_xml = fs::read_to_string(args.xml_path).expect("Unable to read junit.xml");
    let testsuites: TestSuites = quick_xml::de::from_str(&junit_xml).expect("Unable to parse XML");

    let mut run_set: HashSet<String> = HashSet::new();

    let mut results = Vec::new();

    for suite in testsuites.testsuites {
        for case in suite.testcases {
            let status = if case.failure.is_some() {
                TestStatus::Failed
            } else if !case.rerun_failures.is_empty() {
                TestStatus::Retried
            } else {
                TestStatus::Passed
            };

            let status_icon = match status {
                TestStatus::Passed => "✅",
                TestStatus::Failed => "❌",
                TestStatus::Retried => "🔁",
            };
            
            run_set.insert(format!("{}::{}", suite.name, case.name));

            results.push(TestResult {
                suite_name: suite.name.clone(),
                case_name: case.name,
                status,
                status_icon,
                time: case.time.unwrap_or(0.0),
            });
        }
    }

    if args.json_path.is_some() {
        let list_set: HashSet<String> = parse_list_json(args.json_path.expect("Missing json_path argument"));
        match validate_result_list(&list_set, &run_set) {
            Ok(()) => (),
            Err(e) => {
                eprintln!("Error validating list: {e:#}");
                std::process::exit(1);
            }
        }
    }

    // no validation list, or list matches : in either case, now print the results

    // Sort by status priority (failures first, then flaky, then slow, then the rest)
    results.sort_by(|a, b| {
        let status_ord = (a.status as u8).cmp(&(b.status as u8));
        if status_ord != std::cmp::Ordering::Equal {
            return status_ord;
        }
        b.time
            .partial_cmp(&a.time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    println!("| Test Suite | Test | Status | Time |");
    println!("|---|---|---|---|");

    for result in &results {
        println!(
            "| {} | {} | {} | {:.3}s |",
            result.suite_name, result.case_name, result.status_icon, result.time
        );
    }

    if args.check_test_time {
        const LIMIT: f64 = 180.0; // 3 minutes in seconds
        let slow_tests: Vec<String> = results
            .iter()
            .filter(|r| r.time > LIMIT)
            .map(|r| format!("{}::{} ({:.3}s)", r.suite_name, r.case_name, r.time))
            .collect();

        if !slow_tests.is_empty() {
            return Err(format!(
                "The following tests took longer than 3 minutes (limit {:.1}s):\n{}",
                LIMIT,
                slow_tests.join("\n")
            ));
        }
    }

    Ok(())
}
