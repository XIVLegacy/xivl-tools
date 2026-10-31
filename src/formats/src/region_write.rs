//! Append-only RegionResourceData profile. See docs/formats/region.md.

use std::collections::BTreeSet;

use crate::error::{ErrorKind, FormatError, Result};
use crate::region::{self, AuxiliaryKey, RegionMembership, ROW_STRIDE};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionAppend {
    pub root_key: u32,
    pub root_name: Vec<u8>,
    pub child_key: u32,
    pub child_name: Vec<u8>,
    pub child_resource: u32,
    pub child_type: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct RegionAppendRequest<'a> {
    pub append: &'a RegionAppend,
    pub environment_root: Option<u32>,
}

impl<'a> From<&'a RegionAppend> for RegionAppendRequest<'a> {
    fn from(append: &'a RegionAppend) -> Self {
        Self {
            append,
            environment_root: None,
        }
    }
}

/// Copy a template root and child, preserving the input except size and count.
/// The input must have an exact counted walk and a matching size word.
/// A request may also copy a source root's type 0/1 children verbatim.
pub fn append_region_root<'a>(
    data: &[u8],
    template_root: u32,
    template_child: Option<usize>,
    request: impl Into<RegionAppendRequest<'a>>,
) -> Result<Vec<u8>> {
    let request = request.into();
    let append = request.append;
    let table = region::parse(data)?;
    if table.trailing.length != 0 {
        return Err(FormatError::new(
            ErrorKind::TrailingBytes,
            table.trailing.offset,
            "region append requires no trailing bytes",
        ));
    }
    if u64::from(table.declared_size) != data.len() as u64 {
        return Err(FormatError::new(
            ErrorKind::DeclaredSizeOutOfRange,
            0x20,
            "region append requires size word equal to input length",
        ));
    }
    appended_size(data.len() as u64)?;
    let root_count = table.root_count.checked_add(1).ok_or_else(|| {
        FormatError::new(
            ErrorKind::ResourceLimitExceeded,
            0x24,
            "root count exceeds u32",
        )
    })?;
    let mut templates = table.rows.iter().enumerate().filter(|(_, row)| {
        matches!(row.membership, RegionMembership::Root { .. }) && row.id == template_root
    });
    let (root_index, root) = templates
        .next()
        .ok_or_else(|| invalid(0x24, "template root key is absent"))?;
    if templates.next().is_some() {
        return Err(invalid(root.span.offset, "template root key is repeated"));
    }
    if root.word_20 != 0 {
        return Err(invalid(
            root.span.offset + 0x20,
            "template root +0x20 must be zero",
        ));
    }
    let children = &table.rows[root_index + 1..root_index + 1 + root.word_0c as usize];
    let child_index = template_child
        .or_else(|| {
            children
                .iter()
                .position(|row| !matches!(row.word_04, 0 | 1))
        })
        .ok_or_else(|| invalid(root.span.offset + 0x0C, "template has no eligible child"))?;
    let child = children.get(child_index).ok_or_else(|| {
        invalid(
            root.span.offset + 0x0C,
            "template child index is out of range",
        )
    })?;

    let mut environment = Vec::new();
    if let Some(key) = request.environment_root {
        let mut sources = table.rows.iter().enumerate().filter(|(_, row)| {
            matches!(row.membership, RegionMembership::Root { .. }) && row.id == key
        });
        let (index, source) = sources
            .next()
            .ok_or_else(|| invalid(0x24, "environment root key is absent"))?;
        if sources.next().is_some() {
            return Err(invalid(
                source.span.offset,
                "environment root key is repeated",
            ));
        }
        environment.extend(
            table.rows[index + 1..index + 1 + source.word_0c as usize]
                .iter()
                .filter(|row| matches!(row.word_04, 0 | 1)),
        );
        if environment.is_empty() {
            return Err(invalid(
                source.span.offset + 0x0C,
                "environment root has no type 0 or 1 children",
            ));
        }
    }
    let size = appended_size(data.len() as u64 + environment.len() as u64 * ROW_STRIDE as u64)?;

    let start = data.len() as u64;
    let child_start = start + ROW_STRIDE as u64;
    if append.root_key == 0 || append.root_key > u16::MAX as u32 {
        return Err(invalid(
            start,
            "root key must be nonzero and at most 0xFFFF",
        ));
    }
    let child_type = append.child_type.unwrap_or(child.word_04);
    if matches!(child_type, 0 | 1) {
        return Err(invalid(child_start + 4, "child type must not be 0 or 1"));
    }
    check_name(&append.root_name, start + 0x10)?;
    check_name(&append.child_name, child_start + 0x10)?;
    if append.root_name == append.child_name {
        return Err(invalid(
            child_start + 0x10,
            "new effective names must differ",
        ));
    }

    let mut occupied_resources = BTreeSet::new();
    for (index, row) in table.rows.iter().enumerate() {
        if matches!(row.membership, RegionMembership::Root { .. }) {
            if row.id == append.root_key {
                return Err(invalid(start, "root key already exists"));
            }
            if let Some(AuxiliaryKey::Derived(key)) = region::auxiliary_key(&table, index) {
                occupied_resources.insert(key);
            }
        }
        if row.id == append.child_key {
            return Err(invalid(
                child_start,
                "child key already exists as a root or child key",
            ));
        }
        occupied_resources.insert(row.dat_key);
        let name = region::effective_name(data, row)
            .ok_or_else(|| invalid(row.token.offset, "effective name escapes input"))?;
        if name == append.root_name {
            return Err(invalid(start + 0x10, "root effective name already exists"));
        }
        if name == append.child_name {
            return Err(invalid(
                child_start + 0x10,
                "child effective name already exists",
            ));
        }
    }
    if append.child_resource == 0 {
        return Err(invalid(child_start + 8, "child resource must be nonzero"));
    }
    if append.child_resource & 0xFFFF == 0xFFFF {
        return Err(invalid(
            child_start + 8,
            "child resource low16 must not be 0xFFFF",
        ));
    }
    if occupied_resources.contains(&append.child_resource) {
        return Err(invalid(
            child_start + 8,
            "child resource collides with catalog resource or auxiliary key",
        ));
    }
    let mut highest = append.child_resource & 0xFFFF;
    for row in &environment {
        let low = row.dat_key & 0xFFFF;
        if low == 0xFFFF {
            return Err(invalid(
                row.span.offset + 8,
                "environment resource low16 must not be 0xFFFF",
            ));
        }
        highest = highest.max(low);
    }
    let auxiliary = (append.child_resource & 0xFFFF_0000) | (highest + 1);
    if auxiliary == append.child_resource || occupied_resources.contains(&auxiliary) {
        return Err(invalid(
            child_start + 8,
            "derived auxiliary key collides with catalog resource or auxiliary key",
        ));
    }

    let mut output = Vec::with_capacity(size as usize);
    output.extend_from_slice(data);
    output.extend_from_slice(
        &data[root.span.offset as usize..root.span.offset as usize + ROW_STRIDE],
    );
    output.extend_from_slice(
        &data[child.span.offset as usize..child.span.offset as usize + ROW_STRIDE],
    );
    for row in &environment {
        let offset = row.span.offset as usize;
        output.extend_from_slice(&data[offset..offset + ROW_STRIDE]);
    }
    put_word(&mut output, 0x20, size);
    put_word(&mut output, 0x24, root_count);
    let start = data.len();
    put_word(&mut output, start, append.root_key);
    put_word(&mut output, start + 0x0C, 1 + environment.len() as u32);
    put_name(&mut output, start + 0x10, &append.root_name);
    let start = start + ROW_STRIDE;
    put_word(&mut output, start, append.child_key);
    put_word(&mut output, start + 4, child_type);
    put_word(&mut output, start + 8, append.child_resource);
    put_name(&mut output, start + 0x10, &append.child_name);
    Ok(output)
}

fn appended_size(length: u64) -> Result<u32> {
    length
        .checked_add(2 * ROW_STRIDE as u64)
        .and_then(|size| u32::try_from(size).ok())
        .ok_or_else(|| {
            FormatError::new(
                ErrorKind::ResourceLimitExceeded,
                0x20,
                "appended size exceeds u32",
            )
        })
}

fn invalid(offset: u64, detail: &str) -> FormatError {
    FormatError::new(ErrorKind::InvalidAttributeValue, offset, detail)
}

fn check_name(name: &[u8], offset: u64) -> Result<()> {
    if name.is_empty() || name.len() > 15 || name.iter().any(|byte| !(0x20..=0x7E).contains(byte)) {
        return Err(invalid(
            offset,
            "name must contain 1 to 15 printable ASCII bytes without NUL",
        ));
    }
    Ok(())
}

fn put_word(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_name(bytes: &mut [u8], offset: usize, name: &[u8]) {
    bytes[offset..offset + 16].fill(0);
    bytes[offset..offset + name.len()].copy_from_slice(name);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn catalog() -> Vec<u8> {
        let mut bytes = vec![0xA5; 0x100];
        bytes[..19].copy_from_slice(b"RegionResourceData\0");
        bytes[0x18..0x1E].copy_from_slice(b"1.1.0\0");
        word(&mut bytes, 0x20, 0x100);
        word(&mut bytes, 0x24, 1);
        for (index, (key, kind, resource, name)) in [
            (3, 0xABCDEF01, 0x56780020, b"old-root".as_slice()),
            (30, 0, 0x12340001, b"zero"),
            (31, 1, 0x12340002, b"one"),
            (32, 0x3000, 0x12340003, b"layout"),
        ]
        .into_iter()
        .enumerate()
        {
            let start = 0x40 + index * 0x30;
            bytes[start..start + 0x30].fill(0xB0 + index as u8);
            word(&mut bytes, start, key);
            word(&mut bytes, start + 4, kind);
            word(&mut bytes, start + 8, resource);
            word(
                &mut bytes,
                start + 0x0C,
                if index == 0 { 3 } else { 0xFFFFFFFF },
            );
            bytes[start + 0x10..start + 0x20].fill(0);
            bytes[start + 0x10..start + 0x10 + name.len()].copy_from_slice(name);
        }
        word(&mut bytes, 0x60, 0);
        bytes
    }

    fn addition() -> RegionAppend {
        RegionAppend {
            root_key: 4,
            root_name: b"new-root".to_vec(),
            child_key: 40,
            child_name: b"new-child".to_vec(),
            child_resource: 0xABCD0010,
            child_type: None,
        }
    }

    fn refusal(
        data: &[u8],
        root: u32,
        child: Option<usize>,
        add: &RegionAppend,
        kind: ErrorKind,
        offset: u64,
        reason: &str,
    ) {
        let error = append_region_root(data, root, child, add).unwrap_err();
        assert_eq!(error.kind(), kind, "{error}");
        assert_eq!(error.offset(), offset, "{error}");
        assert!(error.detail().contains(reason), "{error}");
    }

    fn environment_catalog() -> Vec<u8> {
        let mut input = catalog();
        word(&mut input, 0x78, 0x22220040);
        let source = input[0x40..0x100].to_vec();
        input.extend_from_slice(&source);
        input.extend_from_slice(&source[0x30..0x60]);
        word(&mut input, 0x20, 0x1F0);
        word(&mut input, 0x24, 2);
        word(&mut input, 0x100, 7);
        word(&mut input, 0x10C, 4);
        word(&mut input, 0x120, 0x9999);
        word(&mut input, 0x164, 0x100);
        word(&mut input, 0x194, 1);
        word(&mut input, 0x198, 0x33330020);
        input[0x1D0..0x1E0].copy_from_slice(b"opaque-name!!!!Z");
        input
    }

    #[test]
    fn copies_environment_verbatim_from_same_or_different_root() {
        let input = environment_catalog();
        let add = addition();
        for (key, offsets, length) in [
            (3, vec![0x70, 0xA0], 0x2B0),
            (7, vec![0x130, 0x190, 0x1C0], 0x2E0),
        ] {
            let output = append_region_root(
                &input,
                3,
                None,
                RegionAppendRequest {
                    append: &add,
                    environment_root: Some(key),
                },
            )
            .unwrap();
            let mut expected = input.clone();
            expected.extend_from_slice(&input[0x40..0x70]);
            expected.extend_from_slice(&input[0xD0..0x100]);
            for offset in &offsets {
                expected.extend_from_slice(&input[*offset..*offset + 0x30]);
            }
            word(&mut expected, 0x20, length);
            word(&mut expected, 0x24, 3);
            word(&mut expected, 0x1F0, 4);
            word(&mut expected, 0x1FC, 1 + offsets.len() as u32);
            expected[0x200..0x210].copy_from_slice(b"new-root\0\0\0\0\0\0\0\0");
            word(&mut expected, 0x220, 40);
            word(&mut expected, 0x228, 0xABCD0010);
            expected[0x230..0x240].copy_from_slice(b"new-child\0\0\0\0\0\0\0");
            assert_eq!(output, expected);
            let table = region::parse(&output).unwrap();
            assert_eq!(
                region::auxiliary_key(&table, 9),
                Some(AuxiliaryKey::Derived(0xABCD0041))
            );
        }
    }

    #[test]
    fn refuses_invalid_environment_sources_and_low_index_overflow() {
        let input = environment_catalog();
        let add = addition();
        let mut repeated = input.clone();
        repeated.extend_from_slice(&input[0x100..0x130]);
        word(&mut repeated, 0x20, 0x220);
        word(&mut repeated, 0x24, 3);
        word(&mut repeated, 0x1FC, 0);
        let mut empty = input.clone();
        for offset in [0x134, 0x194, 0x1C4] {
            word(&mut empty, offset, 0x3000);
        }
        let mut overflow = input.clone();
        word(&mut overflow, 0x198, 0x3333FFFF);
        for (data, key, offset, reason) in [
            (&input, 99, 0x24, "environment root key is absent"),
            (&repeated, 7, 0x100, "environment root key is repeated"),
            (&empty, 7, 0x10C, "no type 0 or 1 children"),
            (&overflow, 7, 0x198, "environment resource low16"),
        ] {
            let error = append_region_root(
                data,
                3,
                None,
                RegionAppendRequest {
                    append: &add,
                    environment_root: Some(key),
                },
            )
            .unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidAttributeValue, "{error}");
            assert_eq!(error.offset(), offset, "{error}");
            assert!(error.detail().contains(reason), "{error}");
        }
    }

    #[test]
    fn refuses_auxiliary_collisions_introduced_by_environment_maximum() {
        for occupied_offset in [0x48, 0xD8, 0x78] {
            let mut input = environment_catalog();
            word(&mut input, occupied_offset, 0xABCD0041);
            if occupied_offset == 0x78 {
                // The first child's family derives 0xABCD0041 from the layout maximum.
                word(&mut input, 0x78, 0xABCD0001);
                word(&mut input, 0xD8, 0x44440040);
            }
            let add = addition();
            assert!(append_region_root(&input, 3, None, &add).is_ok());
            let error = append_region_root(
                &input,
                3,
                None,
                RegionAppendRequest {
                    append: &add,
                    environment_root: Some(7),
                },
            )
            .unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidAttributeValue);
            assert_eq!(error.offset(), 0x228);
            assert!(error.detail().contains("derived auxiliary key collides"));
        }
    }

    #[test]
    fn appends_exact_template_bytes_and_only_changes_size_and_count() {
        let input = catalog();
        let mut add = addition();
        add.root_name = b"123456789ABCDEF".to_vec();
        let output = append_region_root(&input, 3, None, &add).unwrap();
        let mut expected = input.clone();
        expected.extend_from_slice(&input[0x40..0x70]);
        expected.extend_from_slice(&input[0xD0..0x100]);
        word(&mut expected, 0x20, 0x160);
        word(&mut expected, 0x24, 2);
        word(&mut expected, 0x100, 4);
        word(&mut expected, 0x10C, 1);
        expected[0x110..0x120].copy_from_slice(b"123456789ABCDEF\0");
        word(&mut expected, 0x130, 40);
        word(&mut expected, 0x138, 0xABCD0010);
        expected[0x140..0x150].copy_from_slice(b"new-child\0\0\0\0\0\0\0");
        assert_eq!(output, expected);
        assert_eq!(&output[..0x20], &input[..0x20]);
        assert_eq!(&output[0x28..0x100], &input[0x28..]);
        let parsed = region::parse(&output).unwrap();
        assert_eq!(
            region::auxiliary_key(&parsed, 4),
            Some(AuxiliaryKey::Derived(0xABCD0011))
        );
    }

    #[test]
    fn explicit_child_and_type_override_preserve_other_words() {
        let input = catalog();
        for index in 0..3 {
            let mut add = addition();
            add.child_type = Some(0x87654321);
            let output = append_region_root(&input, 3, Some(index), &add).unwrap();
            assert_eq!(&output[0x13C..0x140], &[0xFF; 4]);
            assert_eq!(
                &output[0x150..0x160],
                &input[0x90 + index * 0x30..0xA0 + index * 0x30]
            );
            assert_eq!(&output[0x134..0x138], &0x87654321u32.to_le_bytes());
        }
        let mut add = addition();
        add.child_key = 0;
        add.root_key = 30;
        assert!(append_region_root(&input, 3, None, &add).is_ok());
    }

    #[test]
    fn refuses_input_extent_and_template_failures() {
        let input = catalog();
        let add = addition();
        refusal(
            b"",
            3,
            None,
            &add,
            ErrorKind::UnexpectedEndOfInput,
            0,
            "wanted 19 byte(s)",
        );
        let mut data = input.clone();
        data.push(0);
        word(&mut data, 0x20, 0x101);
        refusal(
            &data,
            3,
            None,
            &add,
            ErrorKind::TrailingBytes,
            0x100,
            "no trailing",
        );
        refusal(
            &input[..0xFF],
            3,
            None,
            &add,
            ErrorKind::SubresourceCountOutOfRange,
            0x4C,
            "cannot fit",
        );
        let mut data = input.clone();
        word(&mut data, 0x20, 0);
        refusal(
            &data,
            3,
            None,
            &add,
            ErrorKind::DeclaredSizeOutOfRange,
            0x20,
            "size word",
        );
        refusal(
            &input,
            99,
            None,
            &add,
            ErrorKind::InvalidAttributeValue,
            0x24,
            "absent",
        );
        for index in [3, usize::MAX] {
            refusal(
                &input,
                3,
                Some(index),
                &add,
                ErrorKind::InvalidAttributeValue,
                0x4C,
                "out of range",
            );
        }
        let mut data = input.clone();
        word(&mut data, 0x60, 1);
        refusal(
            &data,
            3,
            None,
            &add,
            ErrorKind::InvalidAttributeValue,
            0x60,
            "+0x20 must be zero",
        );
        let mut data = input.clone();
        word(&mut data, 0xD4, 1);
        refusal(
            &data,
            3,
            None,
            &add,
            ErrorKind::InvalidAttributeValue,
            0x4C,
            "no eligible child",
        );
        let mut data = input[..0x70].to_vec();
        word(&mut data, 0x20, 0x70);
        word(&mut data, 0x4C, 0);
        refusal(
            &data,
            3,
            None,
            &add,
            ErrorKind::InvalidAttributeValue,
            0x4C,
            "no eligible child",
        );
        let mut data = input.clone();
        data.extend_from_slice(&input[0x40..0x70]);
        word(&mut data, 0x20, 0x130);
        word(&mut data, 0x24, 2);
        word(&mut data, 0x10C, 0);
        refusal(
            &data,
            3,
            None,
            &add,
            ErrorKind::InvalidAttributeValue,
            0x40,
            "repeated",
        );
    }

    #[test]
    fn refuses_keys_resources_types_and_auxiliary_collisions() {
        let input = catalog();
        for key in [0, 0x10000, u32::MAX, 3] {
            let mut add = addition();
            add.root_key = key;
            refusal(
                &input,
                3,
                None,
                &add,
                ErrorKind::InvalidAttributeValue,
                0x100,
                if key == 3 {
                    "already exists"
                } else {
                    "nonzero and at most"
                },
            );
        }
        for key in [3, 30, 31, 32] {
            let mut add = addition();
            add.child_key = key;
            refusal(
                &input,
                3,
                None,
                &add,
                ErrorKind::InvalidAttributeValue,
                0x130,
                "already exists",
            );
        }
        for kind in [0, 1] {
            let mut add = addition();
            add.child_type = Some(kind);
            refusal(
                &input,
                3,
                None,
                &add,
                ErrorKind::InvalidAttributeValue,
                0x134,
                "child type",
            );
            refusal(
                &input,
                3,
                Some(kind as usize),
                &addition(),
                ErrorKind::InvalidAttributeValue,
                0x134,
                "child type",
            );
        }
        for (key, reason) in [
            (0, "nonzero"),
            (0xABCDFFFF, "low16"),
            (u32::MAX, "low16"),
            (0x12340003, "child resource collides"),
            (0x56780020, "child resource collides"),
            (0x12340004, "child resource collides"),
            (0x12340000, "derived auxiliary key collides"),
            (0x5678001F, "derived auxiliary key collides"),
        ] {
            let mut add = addition();
            add.child_resource = key;
            refusal(
                &input,
                3,
                None,
                &add,
                ErrorKind::InvalidAttributeValue,
                0x138,
                reason,
            );
        }
        let mut data = input.clone();
        word(&mut data, 0xD8, 0x99990003);
        let mut add = addition();
        add.child_resource = 0x12340003;
        refusal(
            &data,
            3,
            None,
            &add,
            ErrorKind::InvalidAttributeValue,
            0x138,
            "derived auxiliary key collides",
        );
        let error = appended_size(u32::MAX as u64 - 95).unwrap_err();
        assert_eq!(
            (error.kind(), error.offset()),
            (ErrorKind::ResourceLimitExceeded, 0x20)
        );
        assert_eq!(appended_size(u32::MAX as u64 - 96).unwrap(), u32::MAX);
        assert!(appended_size(u64::MAX).is_err());
    }

    #[test]
    fn refuses_bad_or_colliding_effective_names() {
        let input = catalog();
        for name in [
            b"".as_slice(),
            b"1234567890123456",
            b"a\0b",
            b"a\nb",
            b"\x1F",
            b"\x7F",
            b"\x80",
        ] {
            for is_root in [true, false] {
                let mut add = addition();
                if is_root {
                    add.root_name = name.to_vec();
                } else {
                    add.child_name = name.to_vec();
                }
                refusal(
                    &input,
                    3,
                    None,
                    &add,
                    ErrorKind::InvalidAttributeValue,
                    if is_root { 0x110 } else { 0x140 },
                    "printable ASCII",
                );
            }
        }
        for name in [b"old-root".as_slice(), b"zero", b"one", b"layout"] {
            let mut add = addition();
            add.root_name = name.to_vec();
            refusal(
                &input,
                3,
                None,
                &add,
                ErrorKind::InvalidAttributeValue,
                0x110,
                "root effective name already exists",
            );
            let mut add = addition();
            add.child_name = name.to_vec();
            refusal(
                &input,
                3,
                None,
                &add,
                ErrorKind::InvalidAttributeValue,
                0x140,
                "child effective name already exists",
            );
        }
        let mut add = addition();
        add.child_name.clone_from(&add.root_name);
        refusal(
            &input,
            3,
            None,
            &add,
            ErrorKind::InvalidAttributeValue,
            0x140,
            "must differ",
        );
        for start in [0x50, 0xE0] {
            let mut data = input.clone();
            data[start..start + 16].copy_from_slice(b"123456789ABCDEFZ");
            let mut add = addition();
            add.root_name = b"123456789ABCDEF".to_vec();
            refusal(
                &data,
                3,
                None,
                &add,
                ErrorKind::InvalidAttributeValue,
                0x110,
                "already exists",
            );
            data[start..start + 16].copy_from_slice(b"old\0ignored!!!!!");
            add.root_name = b"old".to_vec();
            refusal(
                &data,
                3,
                None,
                &add,
                ErrorKind::InvalidAttributeValue,
                0x110,
                "already exists",
            );
        }
    }
}
