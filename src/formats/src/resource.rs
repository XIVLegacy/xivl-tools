//! Resource identifiers and the DAT path they name.
//!
//! Layout evidence and its retail citation: `docs/formats/sedb-res.md`,
//! "Resource ID to DAT path".

use crate::error::{ErrorKind, FormatError, Result};

/// Directory holding the resource tree inside a client install.
pub const RESOURCE_ROOT: &str = "data";

/// Extension of a resource file, uppercase as the client writes it.
pub const RESOURCE_EXTENSION: &str = "DAT";

/// A 32-bit client resource identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceId(u32);

impl ResourceId {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u32 {
        self.0
    }

    /// The install-relative path of this resource, using forward slashes.
    ///
    /// `0xAABBCCDD` maps to `data/AA/BB/CC/DD.DAT`. The mapping is total:
    /// every `u32` names a path, whether or not the file exists.
    pub fn dat_path(self) -> String {
        let [a, b, c, d] = self.0.to_be_bytes();
        format!("{RESOURCE_ROOT}/{a:02X}/{b:02X}/{c:02X}/{d:02X}.{RESOURCE_EXTENSION}")
    }

    /// Canonical text form of the identifier.
    pub fn to_hex(self) -> String {
        format!("0x{:08X}", self.0)
    }
}

/// Parse an identifier written as eight hexadecimal digits, with an
/// optional `0x` prefix. Case is not significant.
///
/// `offset` is the absolute offset of `text` in whatever the caller read
/// it from, so the error points at the input, not at the substring.
pub fn parse_resource_id(text: &str, offset: u64) -> Result<ResourceId> {
    let digits = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"));
    let (digits, digit_offset) = match digits {
        Some(rest) => (rest, offset + 2),
        None => (text, offset),
    };
    if digits.len() != 8 {
        return Err(FormatError::new(
            ErrorKind::InvalidResourceId,
            digit_offset,
            format!(
                "resource id needs exactly 8 hexadecimal digits, found {}",
                digits.len()
            ),
        ));
    }
    for (index, character) in digits.char_indices() {
        if !character.is_ascii_hexdigit() {
            return Err(FormatError::new(
                ErrorKind::InvalidResourceId,
                digit_offset + index as u64,
                "resource id contains a non-hexadecimal character",
            ));
        }
    }
    let value = u32::from_str_radix(digits, 16).map_err(|_| {
        FormatError::new(
            ErrorKind::InvalidResourceId,
            digit_offset,
            "resource id is not a 32-bit hexadecimal value",
        )
    })?;
    Ok(ResourceId::new(value))
}

/// Recover the identifier a `data/AA/BB/CC/DD.DAT` path names.
///
/// Accepts either separator so a path copied from a Windows shell round
/// trips. The `data/` prefix is optional: the four components are what
/// carry the identity.
pub fn parse_dat_path(text: &str, offset: u64) -> Result<ResourceId> {
    let invalid =
        |detail: &str| FormatError::new(ErrorKind::InvalidResourcePath, offset, detail.to_string());

    let normalized = text.replace('\\', "/");
    let trimmed = normalized.trim_end_matches('/');
    let mut components: Vec<&str> = trimmed.split('/').filter(|part| !part.is_empty()).collect();
    if components.len() > 4 {
        let prefix_len = components.len() - 4;
        components.drain(..prefix_len);
    }
    if components.len() != 4 {
        return Err(invalid("path does not have four AA/BB/CC/DD components"));
    }

    let last = components[3];
    let (stem, extension) = last
        .rsplit_once('.')
        .ok_or_else(|| invalid("final component has no extension"))?;
    if !extension.eq_ignore_ascii_case(RESOURCE_EXTENSION) {
        return Err(invalid("final component is not a .DAT file"));
    }

    let mut bytes = [0u8; 4];
    for (index, part) in [components[0], components[1], components[2], stem]
        .into_iter()
        .enumerate()
    {
        if part.len() != 2 || !part.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid("path component is not two hexadecimal digits"));
        }
        bytes[index] =
            u8::from_str_radix(part, 16).map_err(|_| invalid("path component is not a byte"))?;
    }
    Ok(ResourceId::new(u32::from_be_bytes(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_the_cited_resource_ids() {
        // Identifiers named in the promoted evidence document.
        assert_eq!(
            ResourceId::new(0x29D9_0001).dat_path(),
            "data/29/D9/00/01.DAT"
        );
        assert_eq!(
            ResourceId::new(0x8994_0554).dat_path(),
            "data/89/94/05/54.DAT"
        );
        assert_eq!(
            ResourceId::new(0x7EB5_0002).dat_path(),
            "data/7E/B5/00/02.DAT"
        );
    }

    #[test]
    fn boundary_values_map() {
        assert_eq!(ResourceId::new(0).dat_path(), "data/00/00/00/00.DAT");
        assert_eq!(ResourceId::new(u32::MAX).dat_path(), "data/FF/FF/FF/FF.DAT");
    }

    #[test]
    fn every_byte_value_appears_in_every_position() {
        for byte in 0u32..=0xFF {
            for shift in [0u32, 8, 16, 24] {
                let id = ResourceId::new(byte << shift);
                let path = id.dat_path();
                assert_eq!(parse_dat_path(&path, 0).unwrap(), id, "{path}");
            }
        }
    }

    #[test]
    fn the_mapping_round_trips_over_a_spread_of_the_space() {
        // A deterministic linear congruential sweep: no dependency, and the
        // same 20000 identifiers on every machine and every run.
        let mut state: u32 = 0x1234_5678;
        for _ in 0..20_000 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let id = ResourceId::new(state);
            let path = id.dat_path();
            assert!(path.is_ascii());
            assert_eq!(parse_dat_path(&path, 0).unwrap(), id);
            assert_eq!(parse_resource_id(&id.to_hex(), 0).unwrap(), id);
        }
    }

    #[test]
    fn parses_both_text_forms() {
        assert_eq!(
            parse_resource_id("0x29D90001", 0).unwrap(),
            ResourceId::new(0x29D9_0001)
        );
        assert_eq!(
            parse_resource_id("29d90001", 0).unwrap(),
            ResourceId::new(0x29D9_0001)
        );
    }

    #[test]
    fn malformed_ids_carry_the_failing_offset() {
        let error = parse_resource_id("0x29D9000", 100).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidResourceId);
        assert_eq!(error.offset(), 102);

        let error = parse_resource_id("0x29D9000Z", 100).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidResourceId);
        assert_eq!(error.offset(), 109);
    }

    #[test]
    fn malformed_paths_are_rejected() {
        for path in [
            "data/29/D9/00/01.BIN",
            "data/29/D9/01.DAT",
            "data/29/D9/00/0G.DAT",
            "data/29/D9/00/001.DAT",
            "01.DAT",
            "",
        ] {
            let error = parse_dat_path(path, 7).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidResourcePath, "{path}");
            assert_eq!(error.offset(), 7);
        }
    }

    #[test]
    fn accepts_a_backslash_path_and_a_longer_prefix() {
        let id = ResourceId::new(0x29D9_0001);
        assert_eq!(parse_dat_path("data\\29\\D9\\00\\01.DAT", 0).unwrap(), id);
        assert_eq!(
            parse_dat_path("client/root/data/29/D9/00/01.dat", 0).unwrap(),
            id
        );
    }
}
