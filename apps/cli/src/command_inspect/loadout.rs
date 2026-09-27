use super::input::{parse_write, ParsedWrite, SlotContextManifest, TraceKey, WriteCorpus};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use xivl_formats::digest::sha256_hex;

pub(super) const COMMAND_LOADOUT_PAYLOAD_SIZE: usize = 136;
pub(super) const COMMAND_LOADOUT_STREAM_OFFSET: usize = 1;
const COMMAND_LOADOUT_MAX_FRAGMENT_BYTES: usize = 128;

#[derive(Debug)]
pub(super) struct MaterializedCommandLoadout {
    pub(super) report: Value,
    pub(super) payload: Vec<u8>,
}

pub(super) fn materialize_command_loadout(
    manifest: &SlotContextManifest,
    trace_index: usize,
    record_range: Option<(u64, u64)>,
) -> Result<MaterializedCommandLoadout, String> {
    if manifest.schema_version != 2 {
        return Err("materialize-command-loadout requires slot context schema 2".to_owned());
    }
    let corpus = manifest
        .write_corpus
        .as_ref()
        .ok_or_else(|| "slot context schema 2 requires writeCorpus".to_owned())?;
    let traces = index_traces(corpus)?;
    let trace = traces.get(trace_index).ok_or_else(|| {
        format!(
            "trace index {trace_index} is out of range ({} traces)",
            traces.len()
        )
    })?;
    let first_record_index = trace.parsed_writes[0].record_index;
    let last_record_index = trace.parsed_writes.last().unwrap().record_index;
    let selected_range = record_range.unwrap_or((first_record_index, last_record_index));
    if selected_range.0 < first_record_index || selected_range.1 > last_record_index {
        return Err(format!(
            "record range {}:{} is outside trace record range {}:{}",
            selected_range.0, selected_range.1, first_record_index, last_record_index
        ));
    }

    let selected: Vec<&ParsedWrite> = trace
        .parsed_writes
        .iter()
        .filter(|write| {
            write.record_index >= selected_range.0 && write.record_index <= selected_range.1
        })
        .collect();
    if selected.is_empty() {
        return Err(format!(
            "record range {}:{} selects no records in trace {trace_index}",
            selected_range.0, selected_range.1
        ));
    }

    let stream_size = selected
        .iter()
        .map(|write| write.record_fragment.len())
        .sum::<usize>();
    if stream_size > COMMAND_LOADOUT_MAX_FRAGMENT_BYTES {
        return Err(format!(
            "selected record fragments total {stream_size} bytes, above the capture-observed {}-byte maximum",
            COMMAND_LOADOUT_MAX_FRAGMENT_BYTES
        ));
    }
    let mut payload = vec![0_u8; COMMAND_LOADOUT_PAYLOAD_SIZE];
    payload[0] = stream_size as u8;
    let mut offset = COMMAND_LOADOUT_STREAM_OFFSET;
    let mut fragments = Vec::with_capacity(selected.len());
    for write in &selected {
        let length = write.record_fragment.len();
        let end = offset + length;
        payload[offset..end].copy_from_slice(&write.record_fragment);
        fragments.push(json!({
            "recordIndex": write.record_index,
            "operation": write.operation,
            "payloadOffset": offset,
            "payloadLength": length,
        }));
        offset = end;
    }
    let padding_size = COMMAND_LOADOUT_PAYLOAD_SIZE - offset;
    let report = json!({
        "status": "planned",
        "syntheticProjection": true,
        "packetReplay": false,
        "serverAuthoritative": false,
        "targetMarkers": "omitted",
        "opcode": "0x0137",
        "manifestSchemaVersion": manifest.schema_version,
        "sourceSnapshots": manifest.source_snapshots,
        "derivation": manifest.derivation,
        "rowsSha256": manifest.rows_sha256,
        "writesSha256": corpus.writes_sha256,
        "partialState": corpus.partial_state,
        "initialState": corpus.initial_state,
        "finalState": corpus.final_state,
        "unresolved": manifest.unresolved,
        "trace": {
            "index": trace_index,
            "capture": trace.capture,
            "laneIndex": trace.lane_index,
            "sourceActorId": trace.source_actor_id,
            "firstRecordIndex": first_record_index,
            "lastRecordIndex": last_record_index,
            "writeCount": trace.writes.len(),
        },
        "recordRange": {
            "start": selected_range.0,
            "end": selected_range.1,
        },
        "fragments": fragments,
        "streamLength": stream_size,
        "paddingLength": padding_size,
        "payloadSize": payload.len(),
        "payloadSha256": sha256_hex(&payload),
    });
    Ok(MaterializedCommandLoadout { report, payload })
}

#[derive(Debug)]
struct Trace {
    capture: String,
    lane_index: u32,
    source_actor_id: u32,
    writes: Vec<Value>,
    parsed_writes: Vec<ParsedWrite>,
}

fn index_traces(corpus: &WriteCorpus) -> Result<Vec<Trace>, String> {
    let mut grouped: BTreeMap<TraceKey, (Vec<Value>, Vec<ParsedWrite>)> = BTreeMap::new();
    for (index, value) in corpus.writes.iter().enumerate() {
        let parsed = parse_write(value, index)?;
        let key = (
            parsed.capture.clone(),
            parsed.lane_index,
            parsed.source_actor_id,
        );
        let entry = grouped.entry(key).or_default();
        entry.0.push(value.clone());
        entry.1.push(parsed);
    }
    let mut traces: Vec<Trace> = grouped
        .into_iter()
        .map(
            |((capture, lane_index, source_actor_id), (writes, parsed_writes))| Trace {
                capture,
                lane_index,
                source_actor_id,
                writes,
                parsed_writes,
            },
        )
        .collect();
    traces.sort_by(|left, right| {
        let left_first = left.parsed_writes[0].record_index;
        let right_first = right.parsed_writes[0].record_index;
        left_first
            .cmp(&right_first)
            .then_with(|| left.capture.cmp(&right.capture))
            .then_with(|| left.lane_index.cmp(&right.lane_index))
            .then_with(|| left.source_actor_id.cmp(&right.source_actor_id))
    });
    Ok(traces)
}

fn trace_inventory(traces: &[Trace]) -> Vec<Value> {
    traces
        .iter()
        .enumerate()
        .map(|(index, trace)| {
            let mut operation_counts = BTreeMap::new();
            for write in &trace.parsed_writes {
                *operation_counts
                    .entry(write.operation.as_str())
                    .or_insert(0_u64) += 1;
            }
            json!({
                "index": index,
                "capture": trace.capture,
                "laneIndex": trace.lane_index,
                "sourceActorId": trace.source_actor_id,
                "firstRecordIndex": trace.parsed_writes[0].record_index,
                "lastRecordIndex": trace.parsed_writes.last().unwrap().record_index,
                "writeCount": trace.writes.len(),
                "operationCounts": operation_counts,
            })
        })
        .collect()
}

fn loadout_metadata(
    manifest: &SlotContextManifest,
    corpus: &WriteCorpus,
    trace_count: usize,
) -> Value {
    json!({
        "scope": corpus.scope,
        "recordEncoding": corpus.record_encoding,
        "propertyHashEncoding": corpus.property_hash_encoding,
        "statePartition": corpus.state_partition,
        "stateOrder": corpus.state_order,
        "partialState": corpus.partial_state,
        "initialState": corpus.initial_state,
        "finalState": corpus.final_state,
        "serverAuthoritative": corpus.server_authoritative,
        "packetReplay": corpus.packet_replay,
        "writesSha256": corpus.writes_sha256,
        "writeCount": corpus.writes.len(),
        "traceCount": trace_count,
        "manifestSchemaVersion": manifest.schema_version,
    })
}

pub(super) fn build_loadout_report(
    manifest: &SlotContextManifest,
    trace_index: Option<usize>,
) -> Result<Value, String> {
    if manifest.schema_version != 2 {
        return Err("inspect-command-loadout requires slot context schema 2".to_owned());
    }
    let corpus = manifest.write_corpus.as_ref().unwrap();
    let traces = index_traces(corpus)?;
    let metadata = loadout_metadata(manifest, corpus, traces.len());
    let mut report = json!({
        "status": "available",
        "schemaVersion": manifest.schema_version,
        "kind": manifest.kind,
        "gameVersion": manifest.game_version,
        "manifestStatus": manifest.status,
        "sourceSnapshots": manifest.source_snapshots,
        "derivation": manifest.derivation,
        "coverage": manifest.coverage,
        "rowsSha256": manifest.rows_sha256,
        "writeCorpus": metadata,
        "partialState": true,
        "initialState": "unknown",
        "finalState": "unasserted",
        "packetReplay": false,
        "serverAuthoritative": false,
        "validation": {
            "input": "bounded-canonical-json",
            "writesSha256": "matched",
            "writeCorpus": "matched",
            "traceIndex": "firstRecordIndex then capture, laneIndex, sourceActorId",
        },
        "unresolved": manifest.unresolved,
    });
    if let Some(index) = trace_index {
        let trace = traces.get(index).ok_or_else(|| {
            format!(
                "trace index {index} is out of range ({} traces)",
                traces.len()
            )
        })?;
        report["trace"] = json!({
            "index": index,
            "capture": trace.capture,
            "laneIndex": trace.lane_index,
            "sourceActorId": trace.source_actor_id,
            "firstRecordIndex": trace.parsed_writes[0].record_index,
            "lastRecordIndex": trace.parsed_writes.last().unwrap().record_index,
            "writeCount": trace.writes.len(),
            "writes": trace.writes,
        });
    } else {
        report["traces"] = json!(trace_inventory(&traces));
    }
    Ok(report)
}
