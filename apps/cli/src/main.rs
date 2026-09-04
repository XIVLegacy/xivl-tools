//! `xivl`: the command line over the format libraries.
//!
//! File inspection and game-install sheet extraction.
//! The option exists because two of these formats have no signature: an
//! enable file and a row-offset array are bare arrays of 32-bit values, so
//! nothing in the bytes distinguishes them and a sniffing parser would be
//! guessing. Formats that do carry a signature are recognized.
//!
//! Every input path is supplied by the caller. There is no default
//! location, no client-install search, and no workspace-relative fallback.

use std::io::{Read, Write};
use std::process::ExitCode;

use xivl_formats::{extract_lpb, lua_path_document};
use xivl_formats::{inspect_named_bytes_as, to_canonical_json, validate_named_bytes_as, InspectAs};

mod batch_extract;
mod extract;
mod resource_export;
mod scan;
mod verify_extract;

const USAGE: &str = "\
xivl - Final Fantasy XIV 1.23b client file tools

usage:
  xivl inspect <file> [--as <format>] [--columns <list>]
  xivl validate <file> [--as <format>] [--columns <list>]
  xivl lua-path <path>
  xivl extract-lpb <file> --output <file>
  xivl extract <game-directory> --output <directory>
  xivl catalog <game-or-resource-directory> --output <directory> [--format json|jsonl]
  xivl extract-resource <file> --output <directory> [--format yaml|json] [--materialize-payloads] [--as <format>] [--columns <list>]
  xivl extract-catalog <catalog.json|catalog.jsonl> --root <directory> --output <directory> (--id <resource-id> | --path <catalog-path>)+ [--max-resources <count>] [--max-source-bytes <bytes>] [--max-output-bytes <bytes>] [--format yaml|json] [--materialize-payloads]
  xivl verify-extraction <directory> [--source <file> | --catalog <catalog.json|catalog.jsonl> --root <directory>] [--report json]
  xivl --help
  xivl --version

inspect prints the normalized structural report for one file. With no
--as it recognizes static-actor SAN tables, SEDB containers, SSD documents,
SQEX containers, and scrambled documents, the last by decoding them rather
than by one trailer byte.

validate reads the input the same way and reports the checks that
reading passed. For a format this tool can also write, that includes a
round trip: the model is written back and the bytes must reproduce the
input exactly.

extract discovers SSD sheet definitions under a 1.23b game directory,
reads their data, enable, and row-offset resources, and writes one
lossless CSV view per definition document.

catalog inventories DAT resources without changing them. It records known,
malformed, and unknown formats without guessing. extract-resource writes a
schema-versioned YAML document by default, or JSON, and keeps decoded opaque
payloads in separate files. --materialize-payloads explicitly writes exact
direct-root SEDB/RES payload spans when their boundaries are unambiguous.
extract-catalog plans and validates an explicit catalog selection before
writing isolated per-resource outputs; it never has an implicit extract-all.
verify-extraction checks an existing single or catalog extraction without
writing or repairing it. Source replay is explicit and optional.

lua-path applies the reversible ASCII resource-path transform. extract-lpb
removes an evidenced raw or XOR-0x73 LPB wrapper and writes the compiled Lua
5.1 chunk without interpreting it. The output path must not already exist.

  --as sedb | ssd | scrambled-xml | sqwt | lpb | lpb-bytecode | staticactor-san | gtex | pwib
     | enable-file | row-offsets
     | sheet-data | config-sys | config-pad | config-lng | config-rgn
      Read the input as this format. Needed for enable-file and
      row-offsets, which are unsigned 32-bit arrays with no signature,
      and for the four configuration files, which carry no signature
      either.
      --as ssd decodes a scrambled document first and then reads it with
      the same reader a plaintext one gets; --as scrambled-xml reports
      the container and a census of the document's shape instead.
      --as sqwt decodes a SQEX container, whose key is the file's own
      base name: renaming such a file makes it unreadable.
      --as gtex reports loader-backed texture fields, mapped formats,
      surface offset-size entries, and exact source spans. --as pwib reports
      its two loader-bounded segments and
      the fixed SEDB header at the start of the first.
      --as lpb-bytecode retains the LPB wrapper report and adds bounded
      Lua 5.1 header, prototype, constant, nesting, and validated opcode
      and operand structure. It does not decompile or execute code.
  --columns <type,...>
      Column types of a sheet-data file, from its schema document, for
      example 'str,s32,bool'. Without it the data is read as a stream of
      string values, which is how an all-string sheet is stored.

The report never carries payload bytes, sheet-row text, or a configuration
file's values. The SSD document view preserves sheet names and attribute
values. Use --as scrambled-xml for a redacted census of document shape.

exit status: 0 success, 1 usage or input/output failure, 2 parse failure
";

/// Exit code for a parse failure, distinct from a usage or I/O failure so
/// a caller can tell a malformed file from a mistyped command.
const EXIT_PARSE_FAILURE: u8 = 2;

/// Bounds allocation before parsing a caller-supplied path.
const MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            let mut stderr = std::io::stderr();
            let _ = writeln!(stderr, "xivl: {}", failure.message);
            ExitCode::from(failure.code)
        }
    }
}

#[derive(Debug)]
struct Failure {
    message: String,
    code: u8,
}

impl Failure {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 1,
        }
    }

    fn parse(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: EXIT_PARSE_FAILURE,
        }
    }
}

fn run(arguments: &[String]) -> Result<(), Failure> {
    let first = arguments.first().map(String::as_str);
    match first {
        None | Some("--help") | Some("-h") | Some("help") => {
            print!("{USAGE}");
            Ok(())
        }
        Some("--version") => {
            println!("xivl {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("inspect") => read(&arguments[1..], Operation::Inspect),
        Some("validate") => read(&arguments[1..], Operation::Validate),
        Some("lua-path") => lua_path(&arguments[1..]),
        Some("extract-lpb") => extract_lpb_command(&arguments[1..]),
        Some("extract") => {
            let summary = extract::run(&arguments[1..])?;
            println!(
                "wrote {} CSV files from {} sheet documents and {} rows; {} missing trailing values; {} absent blocks; {} conflicting values",
                summary.files,
                summary.documents,
                summary.rows,
                summary.missing_trailing_values,
                summary.absent_blocks,
                summary.conflicting_values
            );
            Ok(())
        }
        Some("catalog") => {
            let summary = scan::run(&arguments[1..])?;
            println!(
                "cataloged {} resources to {}",
                summary.resources, summary.output
            );
            Ok(())
        }
        Some("extract-resource") => {
            let summary = resource_export::run(&arguments[1..])?;
            println!("wrote {}", summary.output);
            Ok(())
        }
        Some("extract-catalog") => {
            let summary = batch_extract::run(&arguments[1..])?;
            println!(
                "extracted {} resources, {} source bytes, {} output bytes to {}",
                summary.resources, summary.source_bytes, summary.output_bytes, summary.output
            );
            Ok(())
        }
        Some("verify-extraction") => {
            let summary = verify_extract::run(&arguments[1..])?;
            println!("{}", summary.text);
            Ok(())
        }
        Some(other) => Err(Failure::usage(format!(
            "unknown command '{other}'; run 'xivl --help'"
        ))),
    }
}

fn lua_path(arguments: &[String]) -> Result<(), Failure> {
    let [path] = arguments else {
        return Err(Failure::usage("usage: xivl lua-path <path>"));
    };
    let document = lua_path_document(path).map_err(|error| Failure::parse(error.to_string()))?;
    std::io::stdout()
        .write_all(to_canonical_json(&document).as_bytes())
        .map_err(|error| Failure::usage(format!("cannot write output: {error}")))
}

fn extract_lpb_command(arguments: &[String]) -> Result<(), Failure> {
    let [input, flag, output] = arguments else {
        return Err(Failure::usage(
            "usage: xivl extract-lpb <file> --output <file>",
        ));
    };
    if flag != "--output" {
        return Err(Failure::usage(
            "usage: xivl extract-lpb <file> --output <file>",
        ));
    }
    let data = read_capped(input)?;
    let file = extract_lpb(&data).map_err(|error| Failure {
        message: format!("{input}: {error}"),
        code: EXIT_PARSE_FAILURE,
    })?;
    let mut destination = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|error| Failure::usage(format!("cannot create '{output}': {error}")))?;
    destination
        .write_all(&file.decoded)
        .map_err(|error| Failure::usage(format!("cannot write '{output}': {error}")))?;
    println!("wrote {} bytes to {}", file.decoded.len(), output);
    Ok(())
}

#[derive(Clone, Copy)]
enum Operation {
    Inspect,
    Validate,
}

impl Operation {
    fn name(self) -> &'static str {
        match self {
            Operation::Inspect => "inspect",
            Operation::Validate => "validate",
        }
    }
}

fn read(arguments: &[String], operation: Operation) -> Result<(), Failure> {
    let Some((path, rest)) = arguments.split_first() else {
        return Err(Failure::usage(format!(
            "usage: xivl {} <file> [--as <format>] [--columns <list>]",
            operation.name()
        )));
    };
    let how = InspectAs::from_arguments(rest).map_err(Failure::usage)?;
    let data = read_capped(path)?;
    // The SQEX container's key is the file's own base name, so the reading
    // needs it. Taking it from the path the caller typed is the whole of
    // the derivation. Nothing here searches for or resolves a path.
    let name = base_name(path);
    let produced = match operation {
        Operation::Inspect => inspect_named_bytes_as(&data, name, &how),
        Operation::Validate => validate_named_bytes_as(&data, name, &how),
    };
    let document = produced.map_err(|error| Failure {
        message: format!("{path}: {error}"),
        code: EXIT_PARSE_FAILURE,
    })?;
    let text = to_canonical_json(&document);
    std::io::stdout()
        .write_all(text.as_bytes())
        .map_err(|error| Failure::usage(format!("cannot write output: {error}")))
}

fn read_capped(path: &str) -> Result<Vec<u8>, Failure> {
    // The caller supplied the path, so echoing it back leaks nothing they
    // do not already know. It never reaches the JSON document.
    let file = std::fs::File::open(path).map_err(|error| read_failure(path, &error.to_string()))?;
    let mut data = Vec::new();
    file.take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|error| read_failure(path, &error.to_string()))?;
    if data.len() as u64 > MAX_INPUT_BYTES {
        return Err(read_failure(
            path,
            &format!("input is larger than the {MAX_INPUT_BYTES}-byte limit"),
        ));
    }
    Ok(data)
}

fn read_failure(path: &str, reason: &str) -> Failure {
    Failure::usage(format!("cannot read '{path}': {reason}"))
}

/// The part of a path after the last separator. Both separators are cut,
/// so a Windows path typed on any platform gives the same name and a
/// fixture reads the same everywhere.
fn base_name(path: &str) -> &str {
    match path.rfind(['/', '\\']) {
        Some(index) => &path[index + 1..],
        None => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_names_the_staticactor_reader() {
        assert!(USAGE.contains("static-actor SAN tables"));
        assert!(USAGE.contains("staticactor-san"));
    }
}
