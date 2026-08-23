//! The client's own configuration files.
//!
//! Four files sit in the client's user directory rather than under the
//! install: `config.sys`, `config.pad`, `config.lng`, and `config.rgn`.
//! Three of them are a grid of little-endian 32-bit words, two of those
//! behind a leading word that is a compiled-in format stamp. The fourth is
//! five bytes that are not a grid at all.
//!
//! This reading stops at structure. There is exactly one sample of each
//! file, so no field's meaning is claimed: a word is carried with its
//! offset and value, and a run of bytes that reads as printable text is
//! counted rather than named. What that buys is the thing this reading is
//! for - [`ConfigFile::encode`] reproduces the input byte for byte, so
//! nothing has to be understood to be preserved.
//!
//! These files are the owner's own settings rather than client assets, so
//! nothing here goes into a report: the values reach a caller through this
//! type, and `inspect` carries spans, counts, and digests only.
//!
//! Byte-layout evidence and its retail citation: `docs/formats/configuration.md`.

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::Span;

/// Bytes in one word of the grid.
pub const WORD_SIZE: usize = 4;

/// Shortest run of printable units the census reports.
///
/// Below four, ordinary binary fields qualify constantly: the `config.sys`
/// stamp alone reads as two printable UTF-16 units. Four is short enough to
/// catch the four-byte tags these files carry and long enough that the
/// census says something.
pub const MIN_RUN_UNITS: usize = 4;

/// Which configuration file an input is being read as.
///
/// Nothing in the bytes distinguishes them: `config.sys` and `config.pad`
/// open with different stamps but neither is a signature this project has
/// more than one sample of, and `config.lng` and `config.rgn` carry no
/// leading word at all. The caller names the file, as it does for an enable
/// file and a row-offset array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigKind {
    /// `config.sys`: a stamp followed by an unresolved word grid.
    Sys,
    /// `config.pad`: a stamp followed by an unresolved word grid.
    Pad,
    /// `config.lng`: a word grid with no stamp.
    Lng,
    /// `config.rgn`: five bytes, carried whole. Not a word grid.
    Rgn,
}

impl ConfigKind {
    /// The support-matrix row this reading claims against.
    pub const fn format_id(self) -> &'static str {
        match self {
            ConfigKind::Sys => "config-sys",
            ConfigKind::Pad => "config-pad",
            ConfigKind::Lng => "config-lng",
            ConfigKind::Rgn => "config-rgn",
        }
    }

    /// Does this file open with the compiled-in stamp word?
    pub const fn has_stamp(self) -> bool {
        matches!(self, ConfigKind::Sys | ConfigKind::Pad)
    }

    /// Is the body a grid of 32-bit words?
    pub const fn is_word_grid(self) -> bool {
        !matches!(self, ConfigKind::Rgn)
    }

    /// The `--as` name, which is the format id: one row, one reading.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "config-sys" => Some(ConfigKind::Sys),
            "config-pad" => Some(ConfigKind::Pad),
            "config-lng" => Some(ConfigKind::Lng),
            "config-rgn" => Some(ConfigKind::Rgn),
            _ => None,
        }
    }
}

/// The leading word, which is a compiled-in constant rather than anything
/// this install wrote: both values occur as immediate operands in the
/// client executables. See the evidence document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub span: Span,
    pub value: u32,
}

/// How a printable run was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEncoding {
    Ascii,
    Utf16Le,
}

impl RunEncoding {
    pub const fn name(self) -> &'static str {
        match self {
            RunEncoding::Ascii => "ascii",
            RunEncoding::Utf16Le => "utf16le",
        }
    }
}

/// A run of bytes that reads as printable text under one encoding.
///
/// A census entry, not a field. The two encodings are scanned
/// independently, so one span may appear under both, and a run of ordinary
/// binary bytes that happens to be printable is reported the same as real
/// text: the pad file's first device identifier reads as nine printable
/// UTF-16 units and is not text at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextRun {
    pub span: Span,
    pub encoding: RunEncoding,
    /// Bytes for `ascii`, 16-bit units for `utf16le`.
    pub units: u64,
}

/// A parsed configuration file.
///
/// The parse is total over what it accepts: every byte of the input is in
/// exactly one of `stamp`, `words`, and `body`, which is what makes
/// [`ConfigFile::encode`] exact rather than approximate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFile {
    pub kind: ConfigKind,
    /// The leading word, for the two files that carry one.
    pub stamp: Option<Stamp>,
    /// The word grid past the stamp. Empty for a file that has no grid.
    pub grid: Span,
    pub words: Vec<u32>,
    /// The whole input, for a file that is not a grid. Empty otherwise.
    pub body: Vec<u8>,
    /// Printable runs, in offset order and then encoding order.
    pub runs: Vec<TextRun>,
}

impl ConfigFile {
    /// Words whose value is zero.
    pub fn zero_word_count(&self) -> u64 {
        self.words.iter().filter(|word| **word == 0).count() as u64
    }

    /// Absolute offsets of the words that are not zero.
    ///
    /// The values are the owner's settings and stay out of every report.
    /// which slots an install has ever written is structure and is what
    /// this list carries.
    pub fn non_zero_word_offsets(&self) -> Vec<u64> {
        self.words
            .iter()
            .enumerate()
            .filter(|(_, word)| **word != 0)
            .map(|(index, _)| self.grid.offset + (index * WORD_SIZE) as u64)
            .collect()
    }

    /// Rebuild the input.
    ///
    /// This is the write half of the claim: parse then encode reproduces
    /// the bytes exactly, for every input the parser accepts.
    pub fn encode(&self) -> Vec<u8> {
        if !self.kind.is_word_grid() {
            return self.body.clone();
        }
        let mut out = Vec::with_capacity(
            self.words.len() * WORD_SIZE + if self.stamp.is_some() { WORD_SIZE } else { 0 },
        );
        if let Some(stamp) = self.stamp {
            out.extend_from_slice(&stamp.value.to_le_bytes());
        }
        for word in &self.words {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out
    }
}

/// Parse one configuration file.
pub fn parse(data: &[u8], kind: ConfigKind) -> Result<ConfigFile> {
    let runs = printable_runs(data);
    if !kind.is_word_grid() {
        return Ok(ConfigFile {
            kind,
            stamp: None,
            grid: Span::new(0, 0),
            words: Vec::new(),
            body: data.to_vec(),
            runs,
        });
    }

    if kind.has_stamp() && data.len() < WORD_SIZE {
        return Err(FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            data.len() as u64,
            format!(
                "{} opens with a {WORD_SIZE} byte stamp; this input has {}",
                kind.format_id(),
                data.len()
            ),
        ));
    }
    let remainder = data.len() % WORD_SIZE;
    if remainder != 0 {
        // The grid has no length field and no terminator, so a trailing run
        // shorter than a word is unaccountable rather than ignorable.
        return Err(FormatError::new(
            ErrorKind::TrailingPartialRecord,
            (data.len() - remainder) as u64,
            format!(
                "{} is a grid of {WORD_SIZE} byte words; {remainder} byte(s) are left over",
                kind.format_id()
            ),
        ));
    }

    let stamp_length = if kind.has_stamp() { WORD_SIZE } else { 0 };
    let stamp = kind.has_stamp().then(|| Stamp {
        span: Span::new(0, WORD_SIZE as u64),
        value: u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
    });
    let words = data[stamp_length..]
        .chunks_exact(WORD_SIZE)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    Ok(ConfigFile {
        kind,
        stamp,
        grid: Span::new(stamp_length as u64, (data.len() - stamp_length) as u64),
        words,
        body: Vec::new(),
        runs,
    })
}

/// Runs of printable units, under both encodings, over the whole input.
fn printable_runs(data: &[u8]) -> Vec<TextRun> {
    let mut runs = ascii_runs(data);
    runs.extend(utf16_runs(data));
    runs.sort_by_key(|run| (run.span.offset, run.encoding.name()));
    runs
}

fn ascii_runs(data: &[u8]) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut start: Option<usize> = None;
    for index in 0..=data.len() {
        let printable = data
            .get(index)
            .is_some_and(|byte| (0x20..0x7F).contains(byte));
        match (printable, start) {
            (true, None) => start = Some(index),
            (false, Some(begin)) => {
                push_run(&mut runs, begin, index - begin, 1, RunEncoding::Ascii);
                start = None;
            }
            _ => {}
        }
    }
    runs
}

fn utf16_runs(data: &[u8]) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut start: Option<usize> = None;
    let units = data.len() / 2;
    for index in 0..=units {
        // 0xFFFE and 0xFFFF are not characters, and a byte order mark inside
        // a run would mean the scan had found a different kind of thing.
        let printable = index < units && {
            let value = u16::from_le_bytes([data[index * 2], data[index * 2 + 1]]);
            (0x0020..=0xFFFD).contains(&value) && value != 0xFEFF
        };
        match (printable, start) {
            (true, None) => start = Some(index),
            (false, Some(begin)) => {
                push_run(&mut runs, begin * 2, index - begin, 2, RunEncoding::Utf16Le);
                start = None;
            }
            _ => {}
        }
    }
    runs
}

fn push_run(
    runs: &mut Vec<TextRun>,
    offset: usize,
    units: usize,
    unit_size: usize,
    encoding: RunEncoding,
) {
    if units >= MIN_RUN_UNITS {
        runs.push(TextRun {
            span: Span::new(offset as u64, (units * unit_size) as u64),
            encoding,
            units: units as u64,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid in the shape of the real files, with a stamp, zero words, a
    /// UTF-16 span, and a value with bytes in every position.
    fn synthetic_sys() -> Vec<u8> {
        let mut data = 0x2012_0419u32.to_le_bytes().to_vec();
        data.extend(1u32.to_le_bytes());
        data.extend([0u8; 8]);
        for unit in "Name".encode_utf16() {
            data.extend(unit.to_le_bytes());
        }
        data.extend([0u8; 8]);
        data.extend(0xDEAD_BEEFu32.to_le_bytes());
        data
    }

    #[test]
    fn a_grid_round_trips_byte_for_byte() {
        for (kind, data) in [
            (ConfigKind::Sys, synthetic_sys()),
            (ConfigKind::Pad, synthetic_sys()),
            (ConfigKind::Lng, vec![0, 0, 0, 0, 1, 0, 0, 0]),
            (ConfigKind::Rgn, vec![0, 0, 0, 0, 0]),
        ] {
            let parsed = parse(&data, kind).unwrap();
            assert_eq!(parsed.encode(), data, "{}", kind.format_id());
        }
    }

    #[test]
    fn the_stamp_and_the_grid_tile_the_input() {
        let data = synthetic_sys();
        let parsed = parse(&data, ConfigKind::Sys).unwrap();
        let stamp = parsed.stamp.unwrap();
        assert_eq!(stamp.value, 0x2012_0419);
        assert_eq!(stamp.span.end(), parsed.grid.offset);
        assert_eq!(parsed.grid.end(), data.len() as u64);
        assert_eq!(parsed.words.len(), data.len() / WORD_SIZE - 1);
        assert_eq!(parsed.zero_word_count(), 4);
        assert_eq!(parsed.non_zero_word_offsets(), vec![4, 16, 20, 32]);
    }

    #[test]
    fn a_file_with_no_stamp_is_all_grid() {
        let parsed = parse(&[0, 0, 0, 0, 1, 0, 0, 0], ConfigKind::Lng).unwrap();
        assert!(parsed.stamp.is_none());
        assert_eq!(parsed.grid, Span::new(0, 8));
        assert_eq!(parsed.words, vec![0, 1]);
    }

    #[test]
    fn a_leftover_run_shorter_than_a_word_is_refused() {
        let error = parse(&[0u8; 6], ConfigKind::Sys).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::TrailingPartialRecord);
        assert_eq!(error.offset(), 4);

        let error = parse(&[0u8; 3], ConfigKind::Sys).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEndOfInput);
        assert_eq!(error.offset(), 3);

        // The file that is not a grid takes any length, including one no
        // word boundary would allow.
        assert!(parse(&[0u8; 5], ConfigKind::Rgn).is_ok());
    }

    #[test]
    fn the_run_census_counts_both_encodings_and_admits_its_noise() {
        let data = synthetic_sys();
        let parsed = parse(&data, ConfigKind::Sys).unwrap();
        let utf16: Vec<&TextRun> = parsed
            .runs
            .iter()
            .filter(|run| run.encoding == RunEncoding::Utf16Le)
            .collect();
        assert_eq!(utf16.len(), 1);
        assert_eq!(utf16[0].span, Span::new(16, 8));
        assert_eq!(utf16[0].units, 4);
        // Three printable bytes in a row are below the floor, so the ASCII
        // scan finds nothing here even though "Name" is ASCII text: its
        // bytes alternate with the UTF-16 zero halves.
        assert!(parsed
            .runs
            .iter()
            .all(|run| run.encoding == RunEncoding::Utf16Le));

        // A run of ordinary binary bytes that happens to be printable is
        // reported all the same. That is what makes this a census.
        let noisy = parse(b"!\"#$%&'()*+,-./0", ConfigKind::Lng).unwrap();
        assert_eq!(noisy.runs.len(), 2);
        assert_eq!(noisy.runs[0].encoding, RunEncoding::Ascii);
        assert_eq!(noisy.runs[0].units, 16);
        assert_eq!(noisy.runs[1].encoding, RunEncoding::Utf16Le);
    }

    #[test]
    fn an_empty_input_is_an_empty_grid() {
        let parsed = parse(&[], ConfigKind::Lng).unwrap();
        assert!(parsed.words.is_empty());
        assert!(parsed.encode().is_empty());
        // The stamped files need their stamp, so the same input fails there.
        assert!(parse(&[], ConfigKind::Sys).is_err());
    }
}
