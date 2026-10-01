//! Experimental OBJ-to-native WRB blockout importer.
//!
//! The command requires an explicit native RES model, OBJ source, and new
//! output path. It never searches for a client install and never overwrites
//! an existing file.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;

use xivl_formats::digest::sha256_hex;
use xivl_formats::model_import::import_obj_to_model;
use xivl_formats::zone::parse_model;

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
        .unwrap_or_else(|| "model_obj_import".into());
    let native = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage(&program))?;
    let obj = arguments
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

    let native_absolute = fs::canonicalize(&native)
        .map_err(|error| format!("cannot resolve native input {}: {error}", native.display()))?;
    let obj_absolute = fs::canonicalize(&obj)
        .map_err(|error| format!("cannot resolve OBJ input {}: {error}", obj.display()))?;
    let output_absolute = output_path_for_compare(&output)?;
    if output_absolute == native_absolute || output_absolute == obj_absolute {
        return Err("refusing to overwrite an input file".into());
    }
    if output.exists() {
        return Err(format!(
            "refusing to overwrite existing output {}",
            output.display()
        ));
    }

    let native_bytes = fs::read(&native)
        .map_err(|error| format!("cannot read native input {}: {error}", native.display()))?;
    let obj_text = fs::read_to_string(&obj)
        .map_err(|error| format!("cannot read OBJ input {}: {error}", obj.display()))?;
    let output_bytes = import_obj_to_model(&native_bytes, &obj_text)
        .map_err(|error| format!("OBJ import rejected: {error}"))?;
    let model = parse_model(&output_bytes)
        .map_err(|error| format!("generated native model is invalid: {error}"))?;
    write_new_output(&output, &output_bytes)?;

    let vertices = model
        .parts
        .iter()
        .map(|part| part.vertices.len())
        .sum::<usize>();
    let indices = model
        .parts
        .iter()
        .map(|part| part.indices.len())
        .sum::<usize>();
    println!("native={}", native.display());
    println!("obj={}", obj.display());
    println!("output={}", output.display());
    println!("output_length={}", output_bytes.len());
    println!("output_sha256={}", sha256_hex(&output_bytes));
    println!("parts={}", model.parts.len());
    println!("vertices={vertices}");
    println!("indices={indices}");
    Ok(())
}

fn usage(program: &std::ffi::OsStr) -> String {
    format!(
        "usage: {} <native-input> <obj-input> <new-output>",
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

    #[test]
    fn bare_output_name_uses_the_current_directory() {
        let expected = fs::canonicalize(".").unwrap().join("model-obj-test.bin");
        assert_eq!(
            output_path_for_compare(Path::new("model-obj-test.bin")).unwrap(),
            expected
        );
    }

    #[test]
    fn exclusive_output_creation_preserves_a_preexisting_sentinel() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("xivl-model-obj-{}-{unique}.bin", process::id()));
        fs::write(&path, b"sentinel").unwrap();
        let error = write_new_output(&path, b"replacement").unwrap_err();
        assert!(error.contains("cannot create output"));
        assert_eq!(fs::read(&path).unwrap(), b"sentinel");
        fs::remove_file(path).unwrap();
    }
}
