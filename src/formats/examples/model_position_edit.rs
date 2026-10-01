//! Experimental count-preserving model position editor.
//!
//! This research example requires explicit input and output paths. It reports
//! structure and byte spans only; it does not search for a client install.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;

use xivl_formats::digest::sha256_hex;
use xivl_formats::zone::{edit_model_part_positions, parse_model, Model};

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
        .unwrap_or_else(|| "model_position_edit".into());
    let input = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage(&program))?;
    let output = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage(&program))?;
    let part_index = arguments
        .next()
        .ok_or_else(|| usage(&program))?
        .to_string_lossy()
        .parse::<usize>()
        .map_err(|_| "mesh part must be a zero-based integer".to_string())?;
    let factor = arguments
        .next()
        .ok_or_else(|| usage(&program))?
        .to_string_lossy()
        .parse::<f32>()
        .map_err(|_| "factor must be a finite decimal number".to_string())?;
    if arguments.next().is_some() {
        return Err(usage(&program));
    }
    if !(factor.is_finite() && 0.0 < factor && factor <= 1.0) {
        return Err("factor must be finite and in (0, 1]".into());
    }

    let input_absolute = fs::canonicalize(&input)
        .map_err(|error| format!("cannot resolve input {}: {error}", input.display()))?;
    let output_absolute = output_path_for_compare(&output)?;
    if input_absolute == output_absolute {
        return Err("refusing to overwrite the input file".into());
    }
    if output.exists() {
        return Err(format!(
            "refusing to overwrite existing output {}",
            output.display()
        ));
    }
    let input_bytes = fs::read(&input)
        .map_err(|error| format!("cannot read input {}: {error}", input.display()))?;

    let before = parse_model(&input_bytes).map_err(|error| format!("input is invalid: {error}"))?;
    let edited = edit_model_part_positions(&input_bytes, part_index, factor)
        .map_err(|error| format!("position edit rejected: {error}"))?;
    let after =
        parse_model(&edited).map_err(|error| format!("edited output is invalid: {error}"))?;
    if edited.len() != input_bytes.len() {
        return Err("position edit changed the input length".into());
    }
    let spans = changed_spans(&input_bytes, &edited);
    write_new_output(&output, &edited)?;

    let (before_parts, before_vertices, before_indices) = model_counts(&before);
    let (after_parts, after_vertices, after_indices) = model_counts(&after);
    println!("input={}", input.display());
    println!("output={}", output.display());
    println!("input_length={}", input_bytes.len());
    println!("output_length={}", edited.len());
    println!("input_sha256={}", sha256_hex(&input_bytes));
    println!("output_sha256={}", sha256_hex(&edited));
    println!(
        "input_counts=parts:{before_parts},vertices:{before_vertices},indices:{before_indices}"
    );
    println!("output_counts=parts:{after_parts},vertices:{after_vertices},indices:{after_indices}");
    println!("part_index={part_index}");
    println!("factor={factor}");
    println!("changed_byte_spans={}", format_spans(&spans));
    Ok(())
}

fn usage(program: &std::ffi::OsStr) -> String {
    format!(
        "usage: {} <input> <new-output> <zero-based-mesh-part> <factor>",
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

fn model_counts(model: &Model) -> (usize, usize, usize) {
    (
        model.parts.len(),
        model.parts.iter().map(|part| part.vertices.len()).sum(),
        model.parts.iter().map(|part| part.indices.len()).sum(),
    )
}

fn changed_spans(before: &[u8], after: &[u8]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = None;
    for (offset, (left, right)) in before.iter().zip(after).enumerate() {
        if left != right && start.is_none() {
            start = Some(offset);
        } else if left == right {
            if let Some(start) = start.take() {
                spans.push((start, offset - start));
            }
        }
    }
    if let Some(start) = start {
        spans.push((start, before.len() - start));
    }
    spans
}

fn format_spans(spans: &[(usize, usize)]) -> String {
    let mut output = String::from("[");
    for (index, (offset, length)) in spans.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push_str(&format!("{{\"offset\":{offset},\"length\":{length}}}"));
    }
    output.push(']');
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn bare_output_name_uses_the_current_directory() {
        let expected = fs::canonicalize(".")
            .unwrap()
            .join("model-position-test.bin");
        assert_eq!(
            output_path_for_compare(Path::new("model-position-test.bin")).unwrap(),
            expected
        );
    }

    #[test]
    fn exclusive_output_creation_preserves_a_preexisting_sentinel() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "xivl-model-position-{}-{unique}.bin",
            process::id()
        ));
        fs::write(&path, b"sentinel").unwrap();
        let error = write_new_output(&path, b"replacement").unwrap_err();
        assert!(error.contains("cannot create output"));
        assert_eq!(fs::read(&path).unwrap(), b"sentinel");
        fs::remove_file(path).unwrap();
    }
}
