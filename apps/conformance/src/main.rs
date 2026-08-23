//! `conformance`: run the conformance cases in this checkout.
//!
//! Command shape is fixed by `docs/conformance-tests.md`:
//!
//! ```text
//! conformance run [--case <id>]... [--format <id>]...
//!                 [--fixture-root <dir>] [--require-private]
//!                 [--update-expected]
//! ```
//!
//! `--repo-root` is the one addition: it names the checkout to run against
//! and defaults to the working directory. There is no sibling default and
//! no search.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use xivl_conformance::{root_variable, Options, Outcome, DEFAULT_FIXTURE_ROOT};

const USAGE: &str = "\
conformance - run the xivl-tools conformance cases

usage:
  conformance run [--case <id>]... [--format <id>]...
                   [--fixture-root [<root-id>=]<dir>]... [--require-private]
                   [--update-expected]
                   [--repo-root <dir>]

Default run: every case, public fixtures only. Private cases without a
fixture root are reported as skipped with their reason.

A private fixture names the root it lives under, defaulting to
'client-install'. The bare --fixture-root <dir> form sets that one; the
configuration files live under 'user-config', which the client keeps
outside the install, so they take --fixture-root user-config=<dir> or
XIVL_TOOLS_FIXTURE_ROOT_USER_CONFIG.
";

/// Root ids the runner reads from the environment when the option is
/// absent. A root that is not listed here is supplied by option only.
const ENVIRONMENT_ROOTS: [&str; 2] = [DEFAULT_FIXTURE_ROOT, "user-config"];

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match execute(&arguments) {
        Ok(success) => {
            if success {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(message) => {
            eprintln!("conformance: {message}");
            ExitCode::FAILURE
        }
    }
}

fn execute(arguments: &[String]) -> Result<bool, String> {
    match arguments.first().map(String::as_str) {
        None | Some("--help") | Some("-h") | Some("help") => {
            print!("{USAGE}");
            Ok(true)
        }
        Some("run") => run(&arguments[1..]),
        Some(other) => Err(format!(
            "unknown command '{other}'; run 'conformance --help'"
        )),
    }
}

fn run(arguments: &[String]) -> Result<bool, String> {
    let options = parse_options(arguments)?;
    let report = xivl_conformance::run(&options).map_err(|error| error.to_string())?;

    for result in &report.results {
        match &result.outcome {
            Outcome::Passed => println!("PASS {} [{}]", result.id, result.format_id),
            Outcome::Failed(reason) => {
                println!("FAIL {} [{}]: {}", result.id, result.format_id, reason)
            }
            Outcome::Skipped(reason) => {
                println!("SKIP {} [{}]: {}", result.id, result.format_id, reason)
            }
        }
    }
    println!(
        "conformance: {} passed, {} failed, {} skipped",
        report.passed(),
        report.failed(),
        report.skipped()
    );
    Ok(report.is_success())
}

fn parse_options(arguments: &[String]) -> Result<Options, String> {
    let mut fixture_roots = BTreeMap::new();
    for root_id in ENVIRONMENT_ROOTS {
        if let Some(value) = std::env::var_os(root_variable(root_id)) {
            fixture_roots.insert(root_id.to_string(), PathBuf::from(value));
        }
    }
    let mut options = Options {
        repo_root: std::env::current_dir().map_err(|error| error.to_string())?,
        fixture_roots,
        ..Options::default()
    };

    let mut index = 0;
    while index < arguments.len() {
        let flag = arguments[index].as_str();
        let value = || -> Result<String, String> {
            arguments
                .get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag {
            "--case" => {
                options.cases.push(value()?);
                index += 2;
            }
            "--format" => {
                options.formats.push(value()?);
                index += 2;
            }
            "--fixture-root" => {
                // '<id>=<dir>' names a root. A bare directory is the
                // default one. A Windows drive letter is not an id, so the
                // split looks for an id shaped like the manifest's.
                let argument = value()?;
                let (id, directory) = match argument.split_once('=') {
                    Some((id, directory)) if is_root_id(id) => (id.to_string(), directory),
                    _ => (DEFAULT_FIXTURE_ROOT.to_string(), argument.as_str()),
                };
                options.fixture_roots.insert(id, PathBuf::from(directory));
                index += 2;
            }
            "--repo-root" => {
                options.repo_root = PathBuf::from(value()?);
                index += 2;
            }
            "--require-private" => {
                options.require_private = true;
                index += 1;
            }
            "--update-expected" => {
                options.update_expected = true;
                index += 1;
            }
            other => return Err(format!("unknown option '{other}'")),
        }
    }
    Ok(options)
}

/// The manifest's root-id shape: lower-case words joined by hyphens. It is
/// deliberately narrow so `--fixture-root C:\path` cannot be mistaken for a
/// named root.
fn is_root_id(text: &str) -> bool {
    !text.is_empty()
        && text
            .split('-')
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_directory_and_a_named_root_are_told_apart() {
        assert!(is_root_id("user-config"));
        assert!(is_root_id(DEFAULT_FIXTURE_ROOT));
        for not_an_id in ["C:\\FFXIV", "/srv/x", "User-Config", "user_config", ""] {
            assert!(!is_root_id(not_an_id), "{not_an_id}");
        }

        let parse = |parts: &[&str]| -> BTreeMap<String, PathBuf> {
            let arguments: Vec<String> = parts.iter().map(|part| part.to_string()).collect();
            parse_options(&arguments).unwrap().fixture_roots
        };
        let roots = parse(&[
            "--fixture-root",
            "C:\\FFXIV",
            "--fixture-root",
            "user-config=D:\\My Games\\x",
        ]);
        assert_eq!(
            roots.get(DEFAULT_FIXTURE_ROOT),
            Some(&PathBuf::from("C:\\FFXIV"))
        );
        assert_eq!(
            roots.get("user-config"),
            Some(&PathBuf::from("D:\\My Games\\x"))
        );
    }
}
