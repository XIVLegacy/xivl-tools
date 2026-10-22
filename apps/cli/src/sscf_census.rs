//! Aggregate SSCF header statistics over an explicit client root.
//!
//! The report carries counts, distributions, ranges, and resource ids of
//! failures. It carries no file bytes and no per-file content.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde_json::{json, Map, Value};
use xivl_formats::sscf::{self, CodecDetail, Sscf};
use xivl_formats::{parse_dat_path, to_canonical_json};

use crate::scan::{collect_dat_paths, relative_path};
use crate::{read_capped, Failure};

pub const CENSUS_SCHEMA_VERSION: u64 = 1;

const MAGIC_PREFIX: &[u8; 8] = b"SEDBSSCF";
const FAILURE_EXAMPLES: usize = 5;
const UNREADABLE: &str = "unreadable";
const TOP_RATES: usize = 10;

pub fn run(arguments: &[String]) -> Result<String, Failure> {
    let [root] = arguments else {
        return Err(Failure::usage("usage: xivl sscf-census <client-root>"));
    };
    let root = Path::new(root);
    if !root.is_dir() {
        return Err(Failure::usage(format!(
            "census root '{}' is not a directory",
            root.display()
        )));
    }
    let scan_root = if root.join("data").is_dir() {
        root.join("data")
    } else {
        root.to_path_buf()
    };
    let mut paths = Vec::new();
    collect_dat_paths(&scan_root, &mut paths)?;
    paths.sort();
    let mut census = Census::default();
    for path in &paths {
        census.scanned += 1;
        let mut prefix = [0u8; 8];
        let is_sscf = File::open(path)
            .and_then(|mut file| file.read_exact(&mut prefix))
            .is_ok()
            && &prefix == MAGIC_PREFIX;
        if !is_sscf {
            continue;
        }
        let relative = relative_path(root, path);
        let label = parse_dat_path(&relative, 0)
            .map(|id| id.to_hex())
            .unwrap_or(relative);
        match read_capped(&path.display().to_string()) {
            Ok(bytes) => census.add(&label, &bytes),
            Err(_) => {
                census.sscf += 1;
                census.add_failure(UNREADABLE, &label);
            }
        }
    }
    Ok(to_canonical_json(&census.report()))
}

#[derive(Default)]
struct StreamStats {
    count: u64,
    channels: BTreeMap<u64, u64>,
    rates: BTreeMap<u64, u64>,
    data_sizes: Range,
    extra_sizes: Range,
    aux_chunks: BTreeMap<u64, u64>,
    unknown_1e: BTreeMap<u64, u64>,
    loops: BTreeMap<&'static str, u64>,
    vorbis_heads: BTreeMap<String, u64>,
    wave_head_absent: u64,
    wave_tags: BTreeMap<String, u64>,
    wave_channels_agree: u64,
    wave_rate_agrees: u64,
}

#[derive(Default)]
struct Range {
    min: Option<u64>,
    max: Option<u64>,
}

impl Range {
    fn add(&mut self, value: u64) {
        self.min = Some(self.min.map_or(value, |min| min.min(value)));
        self.max = Some(self.max.map_or(value, |max| max.max(value)));
    }

    fn to_json(&self) -> Value {
        json!({ "min": self.min, "max": self.max })
    }
}

#[derive(Default)]
struct Census {
    scanned: u64,
    sscf: u64,
    parsed: u64,
    failures: BTreeMap<&'static str, (u64, Vec<String>)>,
    anomalies: BTreeMap<&'static str, u64>,
    files_with_anomalies: u64,
    revisions: BTreeMap<u64, u64>,
    flags: BTreeMap<String, u64>,
    header_sizes: BTreeMap<String, u64>,
    declared_size_equal: u64,
    sizes: Vec<u64>,
    table_a_counts: BTreeMap<u64, u64>,
    stream_counts: BTreeMap<u64, u64>,
    table_c_counts: BTreeMap<u64, u64>,
    unknown_06: BTreeMap<u64, u64>,
    block_d: [u64; 2],
    block_e: [u64; 2],
    rates: BTreeMap<u64, u64>,
    aux_tags: BTreeMap<String, u64>,
    codecs: BTreeMap<u32, StreamStats>,
}

impl Census {
    fn add(&mut self, label: &str, bytes: &[u8]) {
        self.sscf += 1;
        self.sizes.push(bytes.len() as u64);
        match sscf::parse(bytes) {
            Ok(file) => {
                self.parsed += 1;
                self.add_file(&file);
            }
            Err(error) => self.add_failure(error.kind().as_str(), label),
        }
    }

    fn add_failure(&mut self, kind: &'static str, label: &str) {
        let entry = self.failures.entry(kind).or_default();
        entry.0 += 1;
        if entry.1.len() < FAILURE_EXAMPLES {
            entry.1.push(label.to_string());
        }
    }

    fn add_file(&mut self, file: &Sscf) {
        bump(&mut self.revisions, u64::from(file.revision));
        bump_text(&mut self.flags, format!("{:#06x}", file.flags));
        bump_text(&mut self.header_sizes, format!("{:#04x}", file.header_size));
        if u64::from(file.declared_size) == file.total_size {
            self.declared_size_equal += 1;
        }
        bump(&mut self.table_a_counts, u64::from(file.table_a.count));
        bump(&mut self.stream_counts, u64::from(file.stream_table.count));
        bump(&mut self.table_c_counts, u64::from(file.table_c.count));
        bump(&mut self.unknown_06, u64::from(file.unknown_06));
        self.block_d[usize::from(file.block_d_offset != 0)] += 1;
        self.block_e[usize::from(file.block_e_offset != 0)] += 1;
        if !file.anomalies.is_empty() {
            self.files_with_anomalies += 1;
        }
        for anomaly in &file.anomalies {
            *self.anomalies.entry(anomaly.kind).or_default() += 1;
        }
        for stream in &file.streams {
            bump(&mut self.rates, u64::from(stream.sample_rate));
            for chunk in &stream.aux_chunks {
                bump_text(&mut self.aux_tags, chunk.tag.clone());
            }
            let stats = self.codecs.entry(stream.codec).or_default();
            stats.count += 1;
            bump(&mut stats.channels, u64::from(stream.channels));
            bump(&mut stats.rates, u64::from(stream.sample_rate));
            stats.data_sizes.add(u64::from(stream.size));
            stats.extra_sizes.add(u64::from(stream.extra_size));
            bump(&mut stats.aux_chunks, u64::from(stream.aux_chunk_count));
            bump(&mut stats.unknown_1e, u64::from(stream.unknown_1e));
            *stats.loops.entry(loop_class(stream)).or_default() += 1;
            match stream.detail {
                CodecDetail::None => {}
                CodecDetail::Vorbis {
                    lead_u16,
                    ident_offset,
                } => {
                    let lead = lead_u16.map_or("absent".to_string(), |v| format!("{v:#06x}"));
                    let marker = ident_offset.map_or("absent".to_string(), |at| at.to_string());
                    bump_text(
                        &mut stats.vorbis_heads,
                        format!("lead {lead}, marker {marker}"),
                    );
                }
                CodecDetail::WaveFormat(None) => stats.wave_head_absent += 1,
                CodecDetail::WaveFormat(Some(head)) => {
                    bump_text(&mut stats.wave_tags, format!("{:#06x}", head.format_tag));
                    if u32::from(head.channels) == stream.channels {
                        stats.wave_channels_agree += 1;
                    }
                    if head.samples_per_sec == stream.sample_rate {
                        stats.wave_rate_agrees += 1;
                    }
                }
            }
        }
    }

    fn report(&mut self) -> Value {
        self.sizes.sort_unstable();
        let mut size_buckets: BTreeMap<String, u64> = BTreeMap::new();
        for size in &self.sizes {
            let bucket = 64 - size.leading_zeros();
            *size_buckets.entry(format!("{bucket:02}")).or_default() += 1;
        }
        let failed = self.sscf - self.parsed;
        let failures: Map<String, Value> = self
            .failures
            .iter()
            .map(|(kind, (count, examples))| {
                (
                    (*kind).to_string(),
                    json!({ "count": count, "examples": examples }),
                )
            })
            .collect();
        let mut codecs = Map::new();
        for (codec, stats) in &self.codecs {
            codecs.insert(codec.to_string(), codec_report(*codec, stats));
        }
        json!({
            "schemaVersion": CENSUS_SCHEMA_VERSION,
            "operation": "sscf-census",
            "files": {
                "datScanned": self.scanned,
                "sedbSscf": self.sscf,
                "parsed": self.parsed,
                "failed": failed,
                "parsedWithAnomalies": self.files_with_anomalies,
            },
            "failures": failures,
            "anomalies": self.anomalies,
            "prefix": {
                "revision": hist(&self.revisions),
                "flags": self.flags,
                "headerSize": self.header_sizes,
                "declaredSizeEqualsFileSize": self.declared_size_equal,
            },
            "fileSize": {
                "min": self.sizes.first(),
                "median": self.sizes.get(self.sizes.len() / 2),
                "max": self.sizes.last(),
                "totalBytes": self.sizes.iter().sum::<u64>(),
                "bitLengthBuckets": size_buckets,
            },
            "tables": {
                "tableACount": hist(&self.table_a_counts),
                "streamCount": hist(&self.stream_counts),
                "tableCCount": hist(&self.table_c_counts),
                "unknown06": distinct(&self.unknown_06),
                "blockDOffset": { "zero": self.block_d[0], "nonzero": self.block_d[1] },
                "blockEOffset": { "zero": self.block_e[0], "nonzero": self.block_e[1] },
            },
            "streams": {
                "total": self.codecs.values().map(|stats| stats.count).sum::<u64>(),
                "sampleRate": rate_report(&self.rates),
                "auxChunkTags": self.aux_tags,
                "byCodec": codecs,
            },
        })
    }
}

fn codec_report(codec: u32, stats: &StreamStats) -> Value {
    let mut report = json!({
        "name": sscf::codec_name(codec),
        "streams": stats.count,
        "channels": hist(&stats.channels),
        "sampleRate": rate_report(&stats.rates),
        "dataSize": stats.data_sizes.to_json(),
        "extraSize": stats.extra_sizes.to_json(),
        "auxChunkCount": hist(&stats.aux_chunks),
        "unknown1E": hist(&stats.unknown_1e),
        "loop": stats.loops,
    });
    match codec {
        sscf::CODEC_VORBIS => {
            report["vorbisHead"] = json!(stats.vorbis_heads);
        }
        sscf::CODEC_WAVE_FORMAT => {
            report["waveFormat"] = json!({
                "headAbsent": stats.wave_head_absent,
                "formatTag": stats.wave_tags,
                "channelsAgree": stats.wave_channels_agree,
                "sampleRateAgrees": stats.wave_rate_agrees,
            });
        }
        _ => {}
    }
    report
}

/// How the loop pair relates to the codec data length. The unit of the
/// loop fields is not established, so this only compares magnitudes.
fn loop_class(stream: &sscf::Stream) -> &'static str {
    let (start, end, size) = (stream.loop_start, stream.loop_end, stream.size);
    if start == 0 && end == 0 {
        "none"
    } else if start > end {
        "start-after-end"
    } else if end == size {
        "end-equals-size"
    } else if end < size {
        "end-within-size"
    } else {
        "end-past-size"
    }
}

fn bump(map: &mut BTreeMap<u64, u64>, key: u64) {
    *map.entry(key).or_default() += 1;
}

fn bump_text(map: &mut BTreeMap<String, u64>, key: String) {
    *map.entry(key).or_default() += 1;
}

fn hist(map: &BTreeMap<u64, u64>) -> Value {
    Value::Object(
        map.iter()
            .map(|(key, count)| (key.to_string(), json!(count)))
            .collect(),
    )
}

fn distinct(map: &BTreeMap<u64, u64>) -> Value {
    json!({
        "distinct": map.len() as u64,
        "min": map.keys().next(),
        "max": map.keys().next_back(),
    })
}

fn rate_report(map: &BTreeMap<u64, u64>) -> Value {
    let mut ranked: Vec<(u64, u64)> = map.iter().map(|(rate, count)| (*rate, *count)).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let top: Vec<Value> = ranked
        .iter()
        .take(TOP_RATES)
        .map(|(rate, count)| json!({ "rate": rate, "streams": count }))
        .collect();
    let mut report = distinct(map);
    report["top"] = Value::Array(top);
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "xivl-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn put32(buffer: &mut [u8], at: usize, value: u32) {
        buffer[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Three stream-table entries onto one stream header with no auxiliary
    /// chunk, 0x10 extra bytes, 8 data bytes. Table C holds 1 entry, table A 2.
    fn synthetic(codec: u32, channels: u32, rate: u32, loops: (u32, u32)) -> Vec<u8> {
        let total = 0x80 + 0x20 + 0x10 + 8;
        let mut buffer = vec![0u8; total];
        buffer[..8].copy_from_slice(MAGIC_PREFIX);
        put32(&mut buffer, 0x08, 3);
        buffer[0x0C..0x0E].copy_from_slice(&0x0400u16.to_le_bytes());
        buffer[0x0E..0x10].copy_from_slice(&0x30u16.to_le_bytes());
        put32(&mut buffer, 0x10, total as u32);
        buffer[0x30..0x32].copy_from_slice(&1u16.to_le_bytes());
        buffer[0x32..0x34].copy_from_slice(&2u16.to_le_bytes());
        buffer[0x34..0x36].copy_from_slice(&3u16.to_le_bytes());
        put32(&mut buffer, 0x38, 0x64);
        put32(&mut buffer, 0x3C, 0x6C);
        put32(&mut buffer, 0x40, 0x60);
        put32(&mut buffer, 0x60, 0x44);
        put32(&mut buffer, 0x64, 0x48);
        put32(&mut buffer, 0x68, 0x4C);
        for slot in 0..3 {
            put32(&mut buffer, 0x6C + slot * 4, 0x80);
        }
        put32(&mut buffer, 0x80, 8);
        put32(&mut buffer, 0x84, channels);
        put32(&mut buffer, 0x88, rate);
        put32(&mut buffer, 0x8C, codec);
        put32(&mut buffer, 0x90, loops.0);
        put32(&mut buffer, 0x94, loops.1);
        put32(&mut buffer, 0x98, 0x10);
        buffer
    }

    #[test]
    fn counts_distributions_and_failures_without_file_content() {
        let root = temp_root("sscf-census");
        let directory = root.join("data/07/FE/00");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("01.DAT"),
            synthetic(sscf::CODEC_VORBIS, 2, 44100, (4, 8)),
        )
        .unwrap();
        fs::write(
            directory.join("02.DAT"),
            synthetic(sscf::CODEC_WAVE_FORMAT, 1, 22050, (0, 0)),
        )
        .unwrap();
        let mut truncated = synthetic(sscf::CODEC_WAVE_FORMAT, 1, 22050, (0, 0));
        truncated.truncate(0xA0);
        fs::write(directory.join("03.DAT"), truncated).unwrap();
        fs::write(directory.join("04.DAT"), b"GTEX0000").unwrap();

        let text = run(&[root.display().to_string()]).unwrap();
        let report: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(report["files"]["datScanned"], 4);
        assert_eq!(report["files"]["sedbSscf"], 3);
        assert_eq!(report["files"]["parsed"], 2);
        assert_eq!(report["files"]["failed"], 1);
        assert_eq!(
            report["failures"]["declared-size-out-of-range"]["examples"],
            json!(["0x07FE0003"])
        );
        assert_eq!(report["streams"]["total"], 6);
        assert_eq!(report["streams"]["byCodec"]["6"]["name"], "vorbis");
        assert_eq!(
            report["streams"]["byCodec"]["6"]["loop"]["end-equals-size"],
            3
        );
        assert_eq!(report["streams"]["byCodec"]["12"]["loop"]["none"], 3);
        assert_eq!(report["streams"]["byCodec"]["6"]["channels"]["2"], 3);
        assert_eq!(report["streams"]["sampleRate"]["distinct"], 2);
        assert_eq!(report["streams"]["sampleRate"]["max"], 44100);
        assert_eq!(report["tables"]["tableCCount"]["1"], 2);
        assert_eq!(report["tables"]["tableACount"]["2"], 2);
        assert_eq!(report["tables"]["streamCount"]["3"], 2);
        assert_eq!(
            report["streams"]["byCodec"]["12"]["waveFormat"]["headAbsent"],
            0
        );
        assert_eq!(text, run(&[root.display().to_string()]).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn oversized_file_counts_as_a_failure_and_the_walk_continues() {
        let root = temp_root("sscf-census-oversize");
        let directory = root.join("data/07/FE/00");
        fs::create_dir_all(&directory).unwrap();
        let big = fs::File::create(directory.join("01.DAT")).unwrap();
        big.set_len(crate::MAX_INPUT_BYTES + 1).unwrap();
        drop(big);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .open(directory.join("01.DAT"))
            .unwrap();
        std::io::Write::write_all(&mut file, MAGIC_PREFIX).unwrap();
        drop(file);
        fs::write(
            directory.join("02.DAT"),
            synthetic(sscf::CODEC_VORBIS, 2, 44100, (0, 0)),
        )
        .unwrap();

        let report: Value =
            serde_json::from_str(&run(&[root.display().to_string()]).unwrap()).unwrap();
        assert_eq!(report["files"]["sedbSscf"], 2);
        assert_eq!(report["files"]["parsed"], 1);
        assert_eq!(report["files"]["failed"], 1);
        assert_eq!(report["failures"]["unreadable"]["count"], 1);
        assert_eq!(
            report["failures"]["unreadable"]["examples"],
            json!(["0x07FE0001"])
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn requires_one_directory_argument() {
        assert!(run(&[]).is_err());
        assert!(run(&["a".into(), "b".into()]).is_err());
        assert!(run(&["/nonexistent-xivl-census-root".into()]).is_err());
    }
}
