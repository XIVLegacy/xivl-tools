//! The conformance suite as a cargo test, so `cargo test` proves the same
//! thing CI proves through the `conformance` binary.

use std::path::PathBuf;

use xivl_conformance::{run, Options, Outcome};

fn repo_root() -> PathBuf {
    // The crate sits at apps/conformance inside the checkout. Resolving
    // upward from the manifest keeps the test independent of the working
    // directory and of anything outside this repository.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("apps/conformance sits two levels below the checkout root")
        .to_path_buf()
}

fn default_options() -> Options {
    Options {
        repo_root: repo_root(),
        ..Options::default()
    }
}

#[test]
fn every_public_case_passes_with_no_fixture_root() {
    let report = run(&default_options()).expect("the runner reads this checkout");
    let failures: Vec<String> = report
        .results
        .iter()
        .filter_map(|result| match &result.outcome {
            Outcome::Failed(reason) => Some(format!("{}: {reason}", result.id)),
            _ => None,
        })
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(
        report.passed() >= 15,
        "expected the committed public cases to run, got {} passed",
        report.passed()
    );
}

#[test]
fn a_run_that_selects_nothing_is_visibly_empty_rather_than_quietly_green() {
    let options = Options {
        cases: vec!["no-such-case".into()],
        ..default_options()
    };
    let report = run(&options).expect("the runner reads this checkout");
    assert_eq!(report.results.len(), 0);
    assert_eq!(report.passed(), 0);
    assert!(!report.is_success());
}

#[test]
fn every_private_case_reports_a_skip_reason_without_a_fixture_root() {
    // Options::default() carries no fixture root, which is the state CI
    // runs in: the retail bytes are the owner's and are not in this
    // checkout. Every private case must say so rather than pass silently.
    let report = run(&default_options()).expect("the runner reads this checkout");
    let mut skipped = 0;
    for result in &report.results {
        if let Outcome::Skipped(reason) = &result.outcome {
            skipped += 1;
            assert!(
                reason.contains("fixture root"),
                "{} skipped without naming the missing root: {reason}",
                result.id
            );
        }
    }
    assert!(
        skipped >= 6,
        "expected the committed private cases to report themselves skipped, got {skipped}"
    );
}

#[test]
fn require_private_fails_the_run_when_the_bytes_are_absent() {
    let options = Options {
        require_private: true,
        ..default_options()
    };
    let report = run(&options).expect("the runner reads this checkout");
    assert!(
        !report.is_success(),
        "--require-private must fail when no fixture root is supplied"
    );
    assert_eq!(
        report.skipped(),
        0,
        "--require-private must not leave skips"
    );
}

#[test]
fn format_selection_narrows_the_run() {
    let options = Options {
        formats: vec!["resource-path".into()],
        ..default_options()
    };
    let report = run(&options).expect("the runner reads this checkout");
    assert!(report.results.len() >= 2);
    assert!(report
        .results
        .iter()
        .all(|result| result.format_id == "resource-path"));
}
