//! Read the SSCF sound container: SEDB prefix, offset tables, stream headers.
//!
//! The reader resolves every offset and extent it reports against the input
//! and refuses one that runs past it. It never decodes audio, and it reports
//! the bytes it does not interpret as spans rather than values.
//!
//! See `docs/formats/sscf.md` for byte-layout evidence and unknown fields.

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::{Reader, Span};
use crate::sedb;
use crate::Anomaly;

/// SEDB subtype of a sound container.
pub const SUBTYPE: &[u8; 4] = b"SSCF";

/// Bytes of the table header that starts at the SEDB `headerSize`.
pub const TABLE_HEADER_SIZE: usize = 0x30;

/// Bytes of a stream header, before its auxiliary chunks and extra data.
pub const STREAM_HEADER_SIZE: u64 = 0x20;

/// Bytes of an auxiliary chunk's tag and size fields.
pub const AUX_CHUNK_HEADER_SIZE: u64 = 8;

/// Codec id whose extra data holds Vorbis header packets.
pub const CODEC_VORBIS: u32 = 6;

/// Codec id whose extra data begins with a WAVEFORMATEX-shaped head.
pub const CODEC_WAVE_FORMAT: u32 = 12;

const VORBIS_IDENT_MARKER: &[u8; 7] = b"\x01vorbis";

/// Name of a codec id this crate has evidence for, if any.
pub const fn codec_name(codec: u32) -> Option<&'static str> {
    match codec {
        CODEC_VORBIS => Some("vorbis"),
        CODEC_WAVE_FORMAT => Some("wave-format"),
        _ => None,
    }
}

/// One of the three u32 offset arrays in the table header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// The u16 count that governs this table.
    pub count: u16,
    /// Absolute file offset of the first entry.
    pub offset: u32,
    /// Absolute file offsets of the records the table points at.
    pub entries: Vec<u32>,
}

/// A chunk between a stream header and its codec data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuxChunk {
    pub tag: String,
    /// The whole chunk, tag and size fields included.
    pub span: Span,
}

/// The first fields of a WAVEFORMATEX-shaped extra block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaveFormatHead {
    pub format_tag: u16,
    pub channels: u16,
    pub samples_per_sec: u32,
}

/// Codec-specific facts read from a stream's extra data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecDetail {
    None,
    Vorbis {
        /// The u16 at the start of the extra data, if it holds two bytes.
        lead_u16: Option<u16>,
        /// Offset of a plain Vorbis identification header marker from the
        /// start of the extra data.
        ident_offset: Option<u64>,
    },
    WaveFormat(Option<WaveFormatHead>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    /// Absolute offset of the stream header.
    pub header_offset: u64,
    /// Codec data length in bytes.
    pub size: u32,
    pub channels: u32,
    pub sample_rate: u32,
    pub codec: u32,
    pub loop_start: u32,
    pub loop_end: u32,
    /// Bytes between the stream header and the codec data, auxiliary chunks
    /// included.
    pub extra_size: u32,
    pub aux_chunk_count: u16,
    /// The u16 at stream-header offset 0x1E. Meaning unresolved.
    pub unknown_1e: u16,
    pub aux_chunks: Vec<AuxChunk>,
    /// Extra data after the auxiliary chunks.
    pub extra: Span,
    pub data: Span,
    pub detail: CodecDetail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sscf {
    /// The u32 at 0x08, which the SEDB header table calls `unknownA`.
    pub revision: u32,
    pub flags: u16,
    pub header_size: u16,
    pub declared_size: u32,
    pub total_size: u64,
    /// The u16 at table-header offset 0x06. Meaning unresolved.
    pub unknown_06: u16,
    /// Count at table-header offset 0x02, offset array at 0x08.
    pub table_a: Table,
    /// Count at table-header offset 0x04, offset array at 0x0C. Each entry
    /// is a stream header.
    pub stream_table: Table,
    /// Count at table-header offset 0x00, offset array at 0x10.
    pub table_c: Table,
    /// Offsets at table-header offsets 0x18 and 0x20. Targets unresolved.
    pub block_d_offset: u32,
    pub block_e_offset: u32,
    pub streams: Vec<Stream>,
    pub anomalies: Vec<Anomaly>,
}

/// Parse the SSCF container that fills `data`.
pub fn parse(data: &[u8]) -> Result<Sscf> {
    let mut reader = Reader::new(data);
    if reader.take(4)? != sedb::MAGIC {
        return Err(FormatError::new(
            ErrorKind::BadMagic,
            0,
            "input does not start with SEDB",
        ));
    }
    if reader.take(4)? != SUBTYPE {
        return Err(FormatError::new(
            ErrorKind::BadMagic,
            4,
            "SEDB subtype is not SSCF",
        ));
    }
    let revision = reader.u32_le()?;
    let flags = reader.u16_le()?;
    let header_size = reader.u16_le()?;
    let declared_size = reader.u32_le()?;
    let length = data.len() as u64;
    if header_size < sedb::FIXED_HEADER_SIZE {
        return Err(FormatError::new(
            ErrorKind::HeaderTooSmall,
            0x0E,
            format!("headerSize {header_size:#x} is below the fixed fields"),
        ));
    }
    let table_end = u64::from(header_size) + TABLE_HEADER_SIZE as u64;
    if table_end > length {
        return Err(FormatError::new(
            ErrorKind::HeaderSizeOutOfRange,
            0x0E,
            format!("headerSize {header_size:#x} leaves no room for the table header"),
        ));
    }
    if u64::from(declared_size) > length {
        return Err(FormatError::new(
            ErrorKind::DeclaredSizeOutOfRange,
            0x10,
            format!("declared size {declared_size} exceeds the {length}-byte input"),
        ));
    }

    let mut anomalies = Vec::new();
    if header_size != 0x30 {
        anomalies.push(Anomaly::new(
            "header-size-unexpected",
            Span::new(0x0E, 2),
            format!("headerSize is {header_size:#x}, not 0x30"),
        ));
    }
    if u64::from(declared_size) != length {
        anomalies.push(Anomaly::new(
            "declared-size-mismatch",
            Span::new(0x10, 4),
            format!("declared size {declared_size} differs from the {length}-byte input"),
        ));
    }
    if data[0x14..usize::from(header_size).min(0x30)]
        .iter()
        .any(|byte| *byte != 0)
    {
        anomalies.push(Anomaly::new(
            "prefix-field-nonzero",
            Span::new(0x14, u64::from(header_size.min(0x30)) - 0x14),
            "bytes after the declared size are not all zero",
        ));
    }

    let base = usize::from(header_size);
    let mut table = reader.sub_reader(base, TABLE_HEADER_SIZE)?;
    let count_c = table.u16_le()?;
    let count_a = table.u16_le()?;
    let count_stream = table.u16_le()?;
    let unknown_06 = table.u16_le()?;
    let offset_a = table.u32_le()?;
    let offset_stream = table.u32_le()?;
    let offset_c = table.u32_le()?;
    let reserved_14 = table.u32_le()?;
    let block_d_offset = table.u32_le()?;
    let reserved_1c = table.u32_le()?;
    let block_e_offset = table.u32_le()?;
    let reserved_tail = table.take(12)?;
    if reserved_14 != 0 || reserved_1c != 0 || reserved_tail.iter().any(|byte| *byte != 0) {
        anomalies.push(Anomaly::new(
            "reserved-header-word-nonzero",
            Span::new(base as u64, TABLE_HEADER_SIZE as u64),
            "a table-header word observed as zero is not zero",
        ));
    }
    for (name, offset) in [("d", block_d_offset), ("e", block_e_offset)] {
        if u64::from(offset) > length {
            anomalies.push(Anomaly::new(
                "block-offset-out-of-range",
                Span::new(base as u64, TABLE_HEADER_SIZE as u64),
                format!("block {name} offset {offset:#x} is past the {length}-byte input"),
            ));
        }
    }

    let table_a = read_unknown_table(data, count_a, offset_a, "a", &mut anomalies);
    let stream_table = read_table(data, count_stream, offset_stream)?;
    let table_c = read_unknown_table(data, count_c, offset_c, "c", &mut anomalies);

    let mut streams = Vec::with_capacity(stream_table.entries.len());
    for entry in &stream_table.entries {
        streams.push(read_stream(data, u64::from(*entry), &mut anomalies)?);
    }

    Ok(Sscf {
        revision,
        flags,
        header_size,
        declared_size,
        total_size: length,
        unknown_06,
        table_a,
        stream_table,
        table_c,
        block_d_offset,
        block_e_offset,
        streams,
        anomalies,
    })
}

/// Read a table whose records are uninterpreted. A range failure keeps the
/// entries read so far and reports an anomaly instead of rejecting the file.
fn read_unknown_table(
    data: &[u8],
    count: u16,
    offset: u32,
    name: &str,
    anomalies: &mut Vec<Anomaly>,
) -> Table {
    match read_table(data, count, offset) {
        Ok(table) => table,
        Err(error) => {
            anomalies.push(Anomaly::new(
                "unknown-table-out-of-range",
                Span::new(error.offset(), 4),
                format!("table {name}: {error}"),
            ));
            Table {
                count,
                offset,
                entries: Vec::new(),
            }
        }
    }
}

fn read_table(data: &[u8], count: u16, offset: u32) -> Result<Table> {
    let mut entries = Vec::with_capacity(usize::from(count));
    if count != 0 {
        let length = data.len() as u64;
        let end = u64::from(offset) + u64::from(count) * 4;
        if end > length {
            return Err(FormatError::new(
                ErrorKind::TableOutOfRange,
                u64::from(offset).min(length),
                format!("{count} entries at {offset:#x} run past the {length}-byte input"),
            ));
        }
        let mut reader = Reader::new(data);
        reader.seek(offset as usize)?;
        for _ in 0..count {
            let at = reader.offset();
            let entry = reader.u32_le()?;
            if u64::from(entry) >= length {
                return Err(FormatError::new(
                    ErrorKind::TableOutOfRange,
                    at,
                    format!("entry {entry:#x} is past the {length}-byte input"),
                ));
            }
            entries.push(entry);
        }
    }
    Ok(Table {
        count,
        offset,
        entries,
    })
}

fn read_stream(data: &[u8], header_offset: u64, anomalies: &mut Vec<Anomaly>) -> Result<Stream> {
    let length = data.len() as u64;
    if header_offset + STREAM_HEADER_SIZE > length {
        return Err(FormatError::new(
            ErrorKind::StreamOutOfRange,
            header_offset,
            "stream header runs past the input",
        ));
    }
    let mut reader = Reader::new(data);
    reader.seek(header_offset as usize)?;
    let size = reader.u32_le()?;
    let channels = reader.u32_le()?;
    let sample_rate = reader.u32_le()?;
    let codec = reader.u32_le()?;
    let loop_start = reader.u32_le()?;
    let loop_end = reader.u32_le()?;
    let extra_size = reader.u32_le()?;
    let aux_chunk_count = reader.u16_le()?;
    let unknown_1e = reader.u16_le()?;

    let extra_start = header_offset + STREAM_HEADER_SIZE;
    let extra_end = extra_start + u64::from(extra_size);
    let data_end = extra_end + u64::from(size);
    if data_end > length {
        return Err(FormatError::new(
            ErrorKind::StreamOutOfRange,
            header_offset,
            format!("stream data ends at {data_end}, past the {length}-byte input"),
        ));
    }

    let mut aux_chunks = Vec::new();
    let mut position = extra_start;
    for _ in 0..aux_chunk_count {
        if position + AUX_CHUNK_HEADER_SIZE > extra_end {
            return Err(FormatError::new(
                ErrorKind::InvalidAuxChunk,
                position,
                "auxiliary chunk header runs past the extra data",
            ));
        }
        let mut chunk = Reader::new(data);
        chunk.seek(position as usize)?;
        let tag = chunk.tag4()?;
        let chunk_size = u64::from(chunk.u32_le()?);
        if chunk_size < AUX_CHUNK_HEADER_SIZE || position + chunk_size > extra_end {
            return Err(FormatError::new(
                ErrorKind::InvalidAuxChunk,
                position + 4,
                format!("auxiliary chunk size {chunk_size} does not fit the extra data"),
            ));
        }
        aux_chunks.push(AuxChunk {
            tag,
            span: Span::new(position, chunk_size),
        });
        position += chunk_size;
    }
    let extra = Span::new(position, extra_end - position);
    let extra_bytes = &data[position as usize..extra_end as usize];

    let detail = match codec {
        CODEC_VORBIS => {
            let lead_u16 = extra_bytes
                .get(..2)
                .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]));
            let ident_offset = extra_bytes
                .windows(VORBIS_IDENT_MARKER.len())
                .position(|window| window == VORBIS_IDENT_MARKER)
                .map(|at| at as u64);
            CodecDetail::Vorbis {
                lead_u16,
                ident_offset,
            }
        }
        CODEC_WAVE_FORMAT => {
            let head = extra_bytes.get(..8).map(|bytes| WaveFormatHead {
                format_tag: u16::from_le_bytes([bytes[0], bytes[1]]),
                channels: u16::from_le_bytes([bytes[2], bytes[3]]),
                samples_per_sec: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            });
            if let Some(head) = head {
                if u32::from(head.channels) != channels || head.samples_per_sec != sample_rate {
                    anomalies.push(Anomaly::new(
                        "wave-format-mismatch",
                        Span::new(header_offset, STREAM_HEADER_SIZE),
                        "stream channels or rate differ from the extra-data head",
                    ));
                }
            }
            CodecDetail::WaveFormat(head)
        }
        _ => CodecDetail::None,
    };

    if loop_start > loop_end {
        anomalies.push(Anomaly::new(
            "loop-start-after-end",
            Span::new(header_offset + 0x10, 8),
            format!("loop start {loop_start} is after loop end {loop_end}"),
        ));
    }

    Ok(Stream {
        header_offset,
        size,
        channels,
        sample_rate,
        codec,
        loop_start,
        loop_end,
        extra_size,
        aux_chunk_count,
        unknown_1e,
        aux_chunks,
        extra,
        data: Span::new(extra_end, u64::from(size)),
        detail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_u16(buffer: &mut [u8], at: usize, value: u16) {
        buffer[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(buffer: &mut [u8], at: usize, value: u32) {
        buffer[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    const STREAM: usize = 0x80;
    const AUX: usize = STREAM + 0x20;
    const WFX: usize = AUX + 0x20;
    const DATA: usize = WFX + 0x14;

    /// One stream with a MARK chunk and a WAVEFORMATEX-shaped head. Table C
    /// holds 1 entry at 0x60, table A 2 at 0x64, and the stream table 3 at
    /// 0x6C, each with distinct targets, all offsets absolute.
    fn synthetic() -> Vec<u8> {
        let stream_header = STREAM;
        let aux = 0x20;
        let wave = 0x14;
        let extra = aux + wave;
        let data_len = 0x10;
        let total = stream_header + 0x20 + extra + data_len;
        let mut buffer = vec![0u8; total];
        buffer[..4].copy_from_slice(b"SEDB");
        buffer[4..8].copy_from_slice(b"SSCF");
        put_u32(&mut buffer, 0x08, 3);
        put_u16(&mut buffer, 0x0C, 0x0400);
        put_u16(&mut buffer, 0x0E, 0x30);
        put_u32(&mut buffer, 0x10, total as u32);
        put_u16(&mut buffer, 0x30, 1);
        put_u16(&mut buffer, 0x32, 2);
        put_u16(&mut buffer, 0x34, 3);
        put_u16(&mut buffer, 0x36, 4321);
        put_u32(&mut buffer, 0x38, 0x64);
        put_u32(&mut buffer, 0x3C, 0x6C);
        put_u32(&mut buffer, 0x40, 0x60);
        put_u32(&mut buffer, 0x60, 0x44);
        put_u32(&mut buffer, 0x64, 0x50);
        put_u32(&mut buffer, 0x68, 0x54);
        for slot in 0..3 {
            put_u32(&mut buffer, 0x6C + slot * 4, stream_header as u32);
        }
        let s = stream_header;
        put_u32(&mut buffer, s, data_len as u32);
        put_u32(&mut buffer, s + 4, 2);
        put_u32(&mut buffer, s + 8, 44100);
        put_u32(&mut buffer, s + 12, CODEC_WAVE_FORMAT);
        put_u32(&mut buffer, s + 16, 10);
        put_u32(&mut buffer, s + 20, 20);
        put_u32(&mut buffer, s + 24, extra as u32);
        put_u16(&mut buffer, s + 28, 1);
        let chunk = s + 0x20;
        buffer[chunk..chunk + 4].copy_from_slice(b"MARK");
        put_u32(&mut buffer, chunk + 4, aux as u32);
        let wfx = chunk + aux;
        put_u16(&mut buffer, wfx, 2);
        put_u16(&mut buffer, wfx + 2, 2);
        put_u32(&mut buffer, wfx + 4, 44100);
        buffer
    }

    #[test]
    fn reads_tables_streams_and_aux_chunks() {
        let buffer = synthetic();
        let file = parse(&buffer).unwrap();
        assert_eq!(file.revision, 3);
        assert_eq!(file.flags, 0x0400);
        assert_eq!(file.unknown_06, 4321);
        assert_eq!(file.table_c.count, 1);
        assert_eq!(
            (file.table_c.offset, &file.table_c.entries[..]),
            (0x60, &[0x44][..])
        );
        assert_eq!(file.table_a.count, 2);
        assert_eq!(
            (file.table_a.offset, &file.table_a.entries[..]),
            (0x64, &[0x50, 0x54][..])
        );
        assert_eq!(file.stream_table.count, 3);
        assert_eq!(file.stream_table.offset, 0x6C);
        assert_eq!(file.stream_table.entries, vec![0x80; 3]);
        assert_eq!(file.streams.len(), 3);
        assert!(file.anomalies.is_empty(), "{:?}", file.anomalies);
        let stream = &file.streams[0];
        assert_eq!(stream.codec, CODEC_WAVE_FORMAT);
        assert_eq!(stream.channels, 2);
        assert_eq!(stream.sample_rate, 44100);
        assert_eq!((stream.loop_start, stream.loop_end), (10, 20));
        assert_eq!(stream.aux_chunks.len(), 1);
        assert_eq!(stream.aux_chunks[0].tag, "MARK");
        assert_eq!(stream.aux_chunks[0].span, Span::new(AUX as u64, 0x20));
        assert_eq!(stream.extra, Span::new(WFX as u64, 0x14));
        assert_eq!(stream.data, Span::new(DATA as u64, 0x10));
        assert_eq!(
            stream.detail,
            CodecDetail::WaveFormat(Some(WaveFormatHead {
                format_tag: 2,
                channels: 2,
                samples_per_sec: 44100,
            }))
        );
    }

    #[test]
    fn stream_data_must_end_inside_the_input() {
        let mut buffer = synthetic();
        put_u32(&mut buffer, STREAM, 0x11);
        let error = parse(&buffer).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::StreamOutOfRange);
        assert_eq!(error.offset(), STREAM as u64);
    }

    #[test]
    fn stream_size_overflowing_a_u32_sum_is_refused() {
        let mut buffer = synthetic();
        put_u32(&mut buffer, STREAM, u32::MAX);
        put_u32(&mut buffer, STREAM + 24, u32::MAX);
        assert_eq!(
            parse(&buffer).unwrap_err().kind(),
            ErrorKind::StreamOutOfRange
        );
    }

    #[test]
    fn aux_chunk_must_fit_the_extra_data() {
        let mut buffer = synthetic();
        put_u32(&mut buffer, AUX + 4, 0x40);
        assert_eq!(
            parse(&buffer).unwrap_err().kind(),
            ErrorKind::InvalidAuxChunk
        );
        put_u32(&mut buffer, AUX + 4, 4);
        assert_eq!(
            parse(&buffer).unwrap_err().kind(),
            ErrorKind::InvalidAuxChunk
        );
        let mut buffer = synthetic();
        put_u16(&mut buffer, STREAM + 28, 3);
        assert_eq!(
            parse(&buffer).unwrap_err().kind(),
            ErrorKind::InvalidAuxChunk
        );
    }

    #[test]
    fn stream_table_entries_must_lie_inside_the_input() {
        let mut buffer = synthetic();
        put_u32(&mut buffer, 0x6C + 4, 0x1000);
        let error = parse(&buffer).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::TableOutOfRange);
        assert_eq!(error.offset(), 0x70);

        let mut buffer = synthetic();
        put_u32(&mut buffer, 0x3C, 0xFFFF_FFF0);
        assert_eq!(
            parse(&buffer).unwrap_err().kind(),
            ErrorKind::TableOutOfRange
        );
    }

    #[test]
    fn stream_header_running_past_the_input_is_a_stream_error() {
        let mut buffer = synthetic();
        let last = buffer.len() as u32 - 0x10;
        put_u32(&mut buffer, 0x6C + 8, last);
        let error = parse(&buffer).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::StreamOutOfRange);
        assert_eq!(error.offset(), u64::from(last));
    }

    #[test]
    fn unknown_tables_report_range_failures_as_anomalies() {
        let mut buffer = synthetic();
        put_u32(&mut buffer, 0x64, 0x1000);
        put_u32(&mut buffer, 0x40, 0xFFFF_FFF0);
        let file = parse(&buffer).unwrap();
        let kinds: Vec<_> = file.anomalies.iter().map(|a| a.kind).collect();
        assert_eq!(
            kinds,
            vec!["unknown-table-out-of-range", "unknown-table-out-of-range"]
        );
        assert_eq!(file.table_a.count, 2);
        assert!(file.table_a.entries.is_empty());
        assert!(file.table_c.entries.is_empty());
        assert_eq!(file.streams.len(), 3);
    }

    #[test]
    fn empty_table_ignores_its_offset() {
        let mut buffer = synthetic();
        put_u16(&mut buffer, 0x32, 0);
        put_u32(&mut buffer, 0x38, 0xFFFF_FFF0);
        assert!(parse(&buffer).unwrap().anomalies.is_empty());
        assert!(parse(&buffer).unwrap().table_a.entries.is_empty());
    }

    #[test]
    fn rejects_other_containers_and_bad_headers() {
        let good = synthetic();
        let mut buffer = good.clone();
        buffer[4..8].copy_from_slice(b"PHB ");
        let error = parse(&buffer).unwrap_err();
        assert_eq!((error.kind(), error.offset()), (ErrorKind::BadMagic, 4));
        assert_eq!(parse(b"GTEX").unwrap_err().kind(), ErrorKind::BadMagic);
        assert_eq!(
            parse(&good[..6]).unwrap_err().kind(),
            ErrorKind::UnexpectedEndOfInput
        );

        let mut buffer = good.clone();
        put_u16(&mut buffer, 0x0E, 0x10);
        assert_eq!(
            parse(&buffer).unwrap_err().kind(),
            ErrorKind::HeaderTooSmall
        );
        let mut buffer = good.clone();
        put_u16(&mut buffer, 0x0E, 0xFFFF);
        assert_eq!(
            parse(&buffer).unwrap_err().kind(),
            ErrorKind::HeaderSizeOutOfRange
        );
        let mut buffer = good;
        put_u32(&mut buffer, 0x10, 0x10000);
        assert_eq!(
            parse(&buffer).unwrap_err().kind(),
            ErrorKind::DeclaredSizeOutOfRange
        );
    }

    #[test]
    fn declared_size_below_the_input_is_an_anomaly() {
        let mut buffer = synthetic();
        buffer.extend_from_slice(&[0; 4]);
        let file = parse(&buffer).unwrap();
        assert_eq!(file.anomalies[0].kind, "declared-size-mismatch");
    }

    #[test]
    fn reports_anomalies_without_rejecting() {
        let mut buffer = synthetic();
        put_u32(&mut buffer, 0x44, 1);
        put_u32(&mut buffer, STREAM + 16, 30);
        put_u16(&mut buffer, WFX + 2, 1);
        let file = parse(&buffer).unwrap();
        let kinds: Vec<_> = file.anomalies.iter().map(|a| a.kind).collect();
        assert_eq!(
            kinds,
            vec![
                "reserved-header-word-nonzero",
                "wave-format-mismatch",
                "loop-start-after-end",
                "wave-format-mismatch",
                "loop-start-after-end",
                "wave-format-mismatch",
                "loop-start-after-end"
            ]
        );
    }

    #[test]
    fn vorbis_extra_reports_its_lead_and_marker() {
        let mut buffer = synthetic();
        let s = STREAM;
        put_u32(&mut buffer, s + 12, CODEC_VORBIS);
        put_u16(&mut buffer, s + 28, 0);
        // Extra data is 0x34 bytes at AUX: lead u16, marker at +0x10.
        put_u16(&mut buffer, AUX, 0x2002);
        buffer[AUX + 0x10..AUX + 0x17].copy_from_slice(b"\x01vorbis");
        let file = parse(&buffer).unwrap();
        assert_eq!(
            file.streams[0].detail,
            CodecDetail::Vorbis {
                lead_u16: Some(0x2002),
                ident_offset: Some(0x10),
            }
        );
        assert_eq!(file.streams[0].extra, Span::new(AUX as u64, 0x34));
    }

    #[test]
    fn unknown_codec_is_carried_without_detail() {
        let mut buffer = synthetic();
        put_u32(&mut buffer, STREAM + 12, 99);
        let file = parse(&buffer).unwrap();
        assert_eq!(file.streams[0].codec, 99);
        assert_eq!(file.streams[0].detail, CodecDetail::None);
        assert_eq!(codec_name(99), None);
        assert_eq!(codec_name(6), Some("vorbis"));
        assert_eq!(codec_name(12), Some("wave-format"));
    }

    #[test]
    fn every_truncation_and_byte_mutation_returns_without_panicking() {
        let good = synthetic();
        for cut in 0..good.len() {
            if let Err(error) = parse(&good[..cut]) {
                assert!(error.offset() <= cut as u64, "cut {cut}: {error}");
            }
        }
        for at in 0..good.len() {
            for value in [0x00, 0x01, 0x7F, 0xFF] {
                let mut buffer = good.clone();
                buffer[at] = value;
                let _ = parse(&buffer);
            }
        }
    }
}
