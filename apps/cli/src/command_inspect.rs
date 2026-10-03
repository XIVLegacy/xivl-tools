//! Query the supplied command battle-parameter catalog.

mod input;
mod loadout;
mod report;

use crate::Failure;
use input::{read_catalog, read_monster_attack_profiles, read_slot_context};
use loadout::{build_loadout_report, materialize_command_loadout, MaterializedCommandLoadout};
use report::build_report_with_inputs;
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;

#[derive(Clone, Copy)]
enum OutputFormat {
    Yaml,
    Json,
}

type InspectArguments = (String, String, Option<String>, Option<String>, OutputFormat);

pub(crate) fn run(arguments: &[String]) -> Result<(), Failure> {
    let (query, catalog_path, slot_context_path, monster_attack_profiles_path, format) =
        parse_arguments(arguments)?;
    let data = read_catalog(&catalog_path)?;
    let slot_context = slot_context_path
        .as_deref()
        .map(read_slot_context)
        .transpose()?;
    let monster_attack_profiles = monster_attack_profiles_path
        .as_deref()
        .map(read_monster_attack_profiles)
        .transpose()?;
    let report = build_report_with_inputs(
        &data,
        &query,
        slot_context.as_ref(),
        monster_attack_profiles.as_ref(),
    )
    .map_err(Failure::usage)?;
    let text = match format {
        OutputFormat::Yaml => serde_yaml::to_string(&report)
            .map_err(|error| Failure::usage(format!("cannot encode YAML report: {error}")))?,
        OutputFormat::Json => {
            let mut text = serde_json::to_string_pretty(&report)
                .map_err(|error| Failure::usage(format!("cannot encode JSON report: {error}")))?;
            text.push('\n');
            text
        }
    };
    std::io::stdout()
        .write_all(text.as_bytes())
        .map_err(|error| Failure::usage(format!("cannot write output: {error}")))
}

pub(crate) fn run_loadout(arguments: &[String]) -> Result<(), Failure> {
    let (slot_context_path, trace_index, format) = parse_loadout_arguments(arguments)?;
    let manifest = read_slot_context(&slot_context_path)?;
    let report = build_loadout_report(&manifest, trace_index).map_err(Failure::usage)?;
    let text = match format {
        OutputFormat::Yaml => serde_yaml::to_string(&report)
            .map_err(|error| Failure::usage(format!("cannot encode YAML report: {error}")))?,
        OutputFormat::Json => {
            let mut text = serde_json::to_string_pretty(&report)
                .map_err(|error| Failure::usage(format!("cannot encode JSON report: {error}")))?;
            text.push('\n');
            text
        }
    };
    std::io::stdout()
        .write_all(text.as_bytes())
        .map_err(|error| Failure::usage(format!("cannot write output: {error}")))
}

pub(crate) fn run_materialize_loadout(arguments: &[String]) -> Result<(), Failure> {
    let (slot_context_path, trace_index, record_range, output, format) =
        parse_materialize_arguments(arguments)?;
    if let Some(path) = output.as_deref() {
        reject_existing_output(path).map_err(Failure::usage)?;
    }
    let manifest = read_slot_context(&slot_context_path)?;
    let materialized = materialize_command_loadout(&manifest, trace_index, record_range)
        .map_err(Failure::usage)?;

    if let Some(path) = output.as_deref() {
        let mut destination = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| Failure::usage(format!("cannot create '{path}': {error}")))?;
        destination
            .write_all(&materialized.payload)
            .map_err(|error| Failure::usage(format!("cannot write '{path}': {error}")))?;
    }

    let report = materialized_report(&materialized, output.as_deref());
    let text = match format {
        OutputFormat::Yaml => serde_yaml::to_string(&report)
            .map_err(|error| Failure::usage(format!("cannot encode YAML report: {error}")))?,
        OutputFormat::Json => {
            let mut text = serde_json::to_string_pretty(&report)
                .map_err(|error| Failure::usage(format!("cannot encode JSON report: {error}")))?;
            text.push('\n');
            text
        }
    };
    std::io::stdout()
        .write_all(text.as_bytes())
        .map_err(|error| Failure::usage(format!("cannot write output: {error}")))
}

fn materialized_report(materialized: &MaterializedCommandLoadout, output: Option<&str>) -> Value {
    let mut report = materialized.report.clone();
    report["status"] = json!(if output.is_some() {
        "written"
    } else {
        "planned"
    });
    if let Some(path) = output {
        report["outputPath"] = json!(path);
    }
    report
}

type MaterializeArguments = (
    String,
    usize,
    Option<(u64, u64)>,
    Option<String>,
    OutputFormat,
);

fn reject_existing_output(path: &str) -> Result<(), String> {
    match std::fs::symlink_metadata(Path::new(path)) {
        Ok(_) => Err(format!("output path already exists: '{path}'")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot inspect output path '{path}': {error}")),
    }
}

fn parse_materialize_arguments(arguments: &[String]) -> Result<MaterializeArguments, Failure> {
    let mut slot_context = None;
    let mut trace = None;
    let mut record_range = None;
    let mut output = None;
    let mut format = OutputFormat::Yaml;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--slot-context" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return Err(materialize_usage());
                };
                if path.starts_with("--") {
                    return Err(materialize_usage());
                }
                if slot_context.replace(path.clone()).is_some() {
                    return Err(Failure::usage("--slot-context may be supplied only once"));
                }
            }
            "--trace" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return Err(materialize_usage());
                };
                let parsed = value
                    .parse::<usize>()
                    .map_err(|_| Failure::usage("--trace must be a nonnegative integer"))?;
                if trace.replace(parsed).is_some() {
                    return Err(Failure::usage("--trace may be supplied only once"));
                }
            }
            "--record-range" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return Err(materialize_usage());
                };
                let Some((first, last)) = value.split_once(':') else {
                    return Err(Failure::usage(
                        "--record-range must use inclusive START:END syntax",
                    ));
                };
                if first.is_empty() || last.is_empty() || last.contains(':') {
                    return Err(Failure::usage(
                        "--record-range must use inclusive START:END syntax",
                    ));
                }
                let first = first.parse::<u64>().map_err(|_| {
                    Failure::usage("--record-range START must be a nonnegative integer")
                })?;
                let last = last.parse::<u64>().map_err(|_| {
                    Failure::usage("--record-range END must be a nonnegative integer")
                })?;
                if first > last {
                    return Err(Failure::usage("--record-range START must not exceed END"));
                }
                if record_range.replace((first, last)).is_some() {
                    return Err(Failure::usage("--record-range may be supplied only once"));
                }
            }
            "--output" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return Err(materialize_usage());
                };
                if path.starts_with("--") {
                    return Err(materialize_usage());
                }
                if output.replace(path.clone()).is_some() {
                    return Err(Failure::usage("--output may be supplied only once"));
                }
            }
            "--format" => {
                index += 1;
                format = match arguments.get(index).map(String::as_str) {
                    Some("yaml") => OutputFormat::Yaml,
                    Some("json") => OutputFormat::Json,
                    _ => return Err(Failure::usage("--format must be yaml or json")),
                };
            }
            option => {
                return Err(Failure::usage(format!(
                    "unknown materialize-command-loadout option '{option}'"
                )))
            }
        }
        index += 1;
    }
    let slot_context = slot_context.ok_or_else(materialize_usage)?;
    let trace = trace.ok_or_else(materialize_usage)?;
    Ok((slot_context, trace, record_range, output, format))
}

fn materialize_usage() -> Failure {
    Failure::usage(
        "usage: xivl materialize-command-loadout --slot-context <command_slot_context.json> --trace <index> [--record-range <first>:<last>] [--output <new-file>] [--format yaml|json]",
    )
}

fn parse_loadout_arguments(
    arguments: &[String],
) -> Result<(String, Option<usize>, OutputFormat), Failure> {
    let mut slot_context = None;
    let mut trace = None;
    let mut format = OutputFormat::Yaml;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--slot-context" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return Err(loadout_usage());
                };
                if path.starts_with("--") {
                    return Err(loadout_usage());
                }
                if slot_context.replace(path.clone()).is_some() {
                    return Err(Failure::usage("--slot-context may be supplied only once"));
                }
            }
            "--trace" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return Err(loadout_usage());
                };
                let parsed = value
                    .parse::<usize>()
                    .map_err(|_| Failure::usage("--trace must be a nonnegative integer"))?;
                if trace.replace(parsed).is_some() {
                    return Err(Failure::usage("--trace may be supplied only once"));
                }
            }
            "--format" => {
                index += 1;
                format = match arguments.get(index).map(String::as_str) {
                    Some("yaml") => OutputFormat::Yaml,
                    Some("json") => OutputFormat::Json,
                    _ => return Err(Failure::usage("--format must be yaml or json")),
                };
            }
            option => {
                return Err(Failure::usage(format!(
                    "unknown inspect-command-loadout option '{option}'"
                )))
            }
        }
        index += 1;
    }
    let slot_context = slot_context.ok_or_else(loadout_usage)?;
    Ok((slot_context, trace, format))
}

fn loadout_usage() -> Failure {
    Failure::usage(
        "usage: xivl inspect-command-loadout --slot-context <command_slot_context.json> [--trace <index>] [--format yaml|json]",
    )
}

fn parse_arguments(arguments: &[String]) -> Result<InspectArguments, Failure> {
    let Some(query) = arguments.first() else {
        return Err(usage());
    };
    if query.is_empty() || query.starts_with("--") {
        return Err(usage());
    }

    let mut catalog = None;
    let mut slot_context = None;
    let mut monster_attack_profiles = None;
    let mut format = OutputFormat::Yaml;
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--catalog" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return Err(usage());
                };
                if catalog.replace(path.clone()).is_some() {
                    return Err(Failure::usage("--catalog may be supplied only once"));
                }
            }
            "--format" => {
                index += 1;
                format = match arguments.get(index).map(String::as_str) {
                    Some("yaml") => OutputFormat::Yaml,
                    Some("json") => OutputFormat::Json,
                    _ => return Err(Failure::usage("--format must be yaml or json")),
                };
            }
            "--slot-context" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return Err(usage());
                };
                if slot_context.replace(path.clone()).is_some() {
                    return Err(Failure::usage("--slot-context may be supplied only once"));
                }
            }
            "--monster-attack-profiles" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return Err(usage());
                };
                if path.starts_with("--") {
                    return Err(usage());
                }
                if monster_attack_profiles.replace(path.clone()).is_some() {
                    return Err(Failure::usage(
                        "--monster-attack-profiles may be supplied only once",
                    ));
                }
            }
            option => {
                return Err(Failure::usage(format!(
                    "unknown inspect-command option '{option}'"
                )))
            }
        }
        index += 1;
    }
    let catalog = catalog.ok_or_else(usage)?;
    Ok((
        query.clone(),
        catalog,
        slot_context,
        monster_attack_profiles,
        format,
    ))
}

fn usage() -> Failure {
    Failure::usage(
        "usage: xivl inspect-command <id-or-name> --catalog <command_battle_params.csv> [--slot-context <command_slot_context.json>] [--monster-attack-profiles <json>] [--format yaml|json]",
    )
}

#[cfg(test)]
mod tests;
