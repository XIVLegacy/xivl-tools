//! Explicit-path PHB collision authoring example.
//!
//! The command requires a native PHB template, a strict geometry JSON input,
//! and a new output path. It never searches for a client install and never
//! overwrites an existing file.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process;

use xivl_formats::collision_json::{parse_geometry_json, MAX_GEOMETRY_JSON_BYTES};
use xivl_formats::digest::sha256_hex;
use xivl_formats::phb_author::{author_phb, author_phb_branching};
use xivl_formats::phb_tree::validate_phb_tree;
use xivl_formats::zone::parse_phb;

const MAX_TEMPLATE_BYTES: u64 = 64 * 1024 * 1024;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args_os();
    let program = arguments
        .next()
        .unwrap_or_else(|| "phb_collision_author".into());
    let first = arguments.next().ok_or_else(|| usage(&program))?;
    let (profile, first) = if first == "--profile" {
        let profile = arguments.next().ok_or_else(|| usage(&program))?;
        let first = arguments.next().ok_or_else(|| usage(&program))?;
        (profile, first)
    } else {
        ("single-leaf".into(), first)
    };
    if profile != "single-leaf" && profile != "branching" {
        return Err("profile must be single-leaf or branching".into());
    }
    let template = PathBuf::from(first);
    let geometry_json = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage(&program))?;
    let output = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage(&program))?;
    if arguments.next().is_some() {
        return Err(usage(&program));
    }

    let template_absolute = fs::canonicalize(&template).map_err(|error| {
        format!(
            "cannot resolve native PHB input {}: {error}",
            template.display()
        )
    })?;
    let geometry_json_absolute = fs::canonicalize(&geometry_json).map_err(|error| {
        format!(
            "cannot resolve geometry JSON input {}: {error}",
            geometry_json.display()
        )
    })?;
    let output_absolute = output_path_for_compare(&output)?;
    if output_absolute == template_absolute || output_absolute == geometry_json_absolute {
        return Err("refusing to overwrite an input file".into());
    }
    if output.exists() {
        return Err(format!(
            "refusing to overwrite existing output {}",
            output.display()
        ));
    }

    let template_bytes = read_bounded(&template, "native PHB input", MAX_TEMPLATE_BYTES)?;
    let geometry_json_bytes = read_bounded(
        &geometry_json,
        "geometry JSON input",
        MAX_GEOMETRY_JSON_BYTES as u64,
    )?;
    let geometry =
        parse_geometry_json(&geometry_json_bytes).map_err(|error| error.detail().to_string())?;
    let output_bytes = if profile == "branching" {
        author_phb_branching(&template_bytes, &geometry)
    } else {
        author_phb(&template_bytes, &geometry)
    }
    .map_err(|error| format!("PHB authoring rejected: {error}"))?;
    if profile == "branching" {
        validate_phb_tree(&output_bytes)
            .map_err(|error| format!("generated PHB tree is invalid: {error}"))?;
    }
    let phb =
        parse_phb(&output_bytes).map_err(|error| format!("generated PHB is invalid: {error}"))?;
    write_new_output(&output, &output_bytes)?;

    let vertices = phb
        .hulls
        .iter()
        .map(|hull| hull.vertices.len())
        .sum::<usize>();
    let triangles = phb
        .hulls
        .iter()
        .map(|hull| hull.indices.len() / 3)
        .sum::<usize>();
    println!("template_length={}", template_bytes.len());
    println!("geometry_json_length={}", geometry_json_bytes.len());
    println!("output_length={}", output_bytes.len());
    println!("template_sha256={}", sha256_hex(&template_bytes));
    println!("geometry_json_sha256={}", sha256_hex(&geometry_json_bytes));
    println!("output_sha256={}", sha256_hex(&output_bytes));
    println!("hulls={}", phb.hulls.len());
    println!("vertices={vertices}");
    println!("triangles={triangles}");
    Ok(())
}

fn usage(program: &std::ffi::OsStr) -> String {
    format!(
        "usage: {} [--profile single-leaf|branching] <native-phb-input> <geometry-json-input> <new-output>",
        Path::new(program).display()
    )
}

fn output_path_for_compare(output: &Path) -> Result<PathBuf, String> {
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|error| {
        format!(
            "cannot resolve output directory {}: {error}",
            parent.display()
        )
    })?;
    let name = output
        .file_name()
        .ok_or_else(|| "output path has no file name".to_string())?;
    Ok(parent.join(name))
}

fn read_bounded(path: &Path, label: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("cannot read {label} {}: {error}", path.display()))?;
    let initial_length = file
        .metadata()
        .map_err(|error| format!("cannot inspect {label} {}: {error}", path.display()))?
        .len();
    if initial_length > limit {
        return Err(format!(
            "{label} {} exceeds the {limit}-byte limit",
            path.display()
        ));
    }
    let capacity = usize::try_from(initial_length)
        .map_err(|_| format!("{label} {} is too large for this platform", path.display()))?;
    let mut bytes = Vec::with_capacity(capacity);
    let read_limit = limit.saturating_add(1);
    Read::by_ref(&mut file)
        .take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read {label} {}: {error}", path.display()))?;
    let final_length = file
        .metadata()
        .map_err(|error| format!("cannot inspect {label} {}: {error}", path.display()))?
        .len();
    if bytes.len() as u64 > limit
        || final_length != initial_length
        || bytes.len() as u64 != initial_length
    {
        return Err(format!(
            "{label} {} changed while being read",
            path.display()
        ));
    }
    Ok(bytes)
}

fn write_new_output(output: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|error| format!("cannot create output {}: {error}", output.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("cannot write output {}: {error}", output.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_path(suffix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!(
            "xivl-phb-author-{}-{unique}.{suffix}",
            process::id()
        ))
    }

    #[test]
    fn bounded_reads_preserve_complete_input() {
        let path = unique_test_path("bin");
        fs::write(&path, b"template").unwrap();
        let bytes = read_bounded(&path, "test input", 64).unwrap();
        assert_eq!(bytes, b"template");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn bounded_reads_reject_oversized_input() {
        let path = unique_test_path("bin");
        fs::write(&path, b"12345").unwrap();
        let error = read_bounded(&path, "test input", 4).unwrap_err();
        assert!(error.contains("exceeds the 4-byte limit"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn exclusive_output_creation_preserves_a_preexisting_sentinel() {
        let path = unique_test_path("bin");
        fs::write(&path, b"sentinel").unwrap();
        let error = write_new_output(&path, b"replacement").unwrap_err();
        assert!(error.contains("cannot create output"));
        assert_eq!(fs::read(&path).unwrap(), b"sentinel");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn bare_output_name_uses_the_current_directory() {
        let expected = fs::canonicalize(".").unwrap().join("phb-test.bin");
        assert_eq!(
            output_path_for_compare(Path::new("phb-test.bin")).unwrap(),
            expected
        );
    }

    #[test]
    fn io_errors_keep_their_context() {
        let path = unique_test_path("missing");
        let error = read_bounded(&path, "test input", 64).unwrap_err();
        assert!(error.contains("cannot read test input"));
    }
}
