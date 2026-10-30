//! Minimal text edits to decoded SSD definition documents.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::{ErrorKind, FormatError, Result};
use crate::ssd::{self, FileBlock, SheetBody};
use crate::{scrambled, xml, ResourceId, Span};

/// Sheet and file indices refer to the original parsed document.
#[derive(Debug, Clone)]
pub enum DefinitionEdit {
    GrowFile {
        sheet: usize,
        file: usize,
        count: u32,
    },
    InsertFile {
        sheet: usize,
        begin: u32,
        count: u32,
        data: ResourceId,
        enable: ResourceId,
        offsets: ResourceId,
    },
}

fn invalid(offset: u64, detail: impl Into<String>) -> FormatError {
    FormatError::new(ErrorKind::InvalidAttributeValue, offset, detail)
}

fn signature(file: &FileBlock) -> (u32, u32, ResourceId, ResourceId, ResourceId) {
    (file.begin, file.count, file.data, file.enable, file.offsets)
}

fn decimal(value: u32, template: &str) -> String {
    let template = template.trim();
    if template.starts_with('0') {
        format!("{value:0width$}", width = template.len())
    } else {
        value.to_string()
    }
}

fn replace_count(
    decoded: &[u8],
    element: &xml::Element,
    count: u32,
    splices: &mut Vec<(usize, usize, String)>,
) -> Result<()> {
    let attribute = element
        .attribute("count")
        .ok_or_else(|| invalid(element.span.offset, "element has no count attribute"))?;
    if element
        .attributes
        .iter()
        .filter(|a| a.name == "count")
        .count()
        != 1
    {
        return Err(invalid(element.span.offset, "ambiguous count attribute"));
    }
    let start = attribute.span.offset as usize;
    let end = attribute.span.end() as usize;
    let quote = decoded[start..end].iter().position(|&b| b == b'"').unwrap();
    splices.push((start + quote + 1, end - 1, decimal(count, &attribute.value)));
    Ok(())
}

/// Change only selected count values and inserted file elements, then reparse.
pub fn edit_definition(decoded: &[u8], edits: &[DefinitionEdit]) -> Result<Vec<u8>> {
    if edits.is_empty() {
        return Ok(decoded.to_vec());
    }
    let original = ssd::parse_document(decoded)?;
    let tree = xml::parse_document(decoded)?;
    let mut expected = original.clone();
    let mut ids = BTreeSet::new();
    for sheet in &original.sheets {
        match &sheet.body {
            SheetBody::Reference { infofile } => {
                ids.insert(*infofile);
            }
            SheetBody::Definition { blocks, .. } => {
                for file in blocks {
                    ids.extend([file.data, file.enable, file.offsets]);
                }
            }
        }
    }
    let mut touched = BTreeSet::new();
    let mut additions = BTreeMap::<usize, Vec<FileBlock>>::new();
    let mut splices = Vec::new();
    for edit in edits {
        let sheet = match edit {
            DefinitionEdit::GrowFile { sheet, .. } | DefinitionEdit::InsertFile { sheet, .. } => {
                *sheet
            }
        };
        let target = expected
            .sheets
            .get_mut(sheet)
            .ok_or_else(|| invalid(0, "sheet index is outside the document"))?;
        let SheetBody::Definition { blocks, .. } = &mut target.body else {
            return Err(invalid(target.span.offset, "sheet is not a definition"));
        };
        match *edit {
            DefinitionEdit::GrowFile { file, count, .. } => {
                let block = blocks.get_mut(file).ok_or_else(|| {
                    invalid(target.span.offset, "file index is outside the sheet")
                })?;
                if !touched.insert((sheet, file)) {
                    return Err(invalid(block.span.offset, "two edits touch the same file"));
                }
                if count < block.count {
                    return Err(invalid(block.span.offset, "a file count cannot shrink"));
                }
                block.count = count;
                let element = &tree.root.children[sheet].child("block").unwrap().children[file];
                replace_count(decoded, element, count, &mut splices)?;
            }
            DefinitionEdit::InsertFile {
                begin,
                count,
                data,
                enable,
                offsets,
                ..
            } => {
                for id in [data, enable, offsets] {
                    if !ids.insert(id) {
                        return Err(invalid(
                            target.span.offset,
                            "inserted resource id is already named",
                        ));
                    }
                }
                additions.entry(sheet).or_default().push(FileBlock {
                    span: Span::new(0, 0),
                    begin,
                    count,
                    data,
                    enable,
                    offsets,
                });
            }
        }
    }
    for (sheet, new) in &mut additions {
        new.sort_by_key(|file| file.begin);
        let block = tree.root.children[*sheet].child("block").ok_or_else(|| {
            invalid(
                original.sheets[*sheet].span.offset,
                "sheet has no block element",
            )
        })?;
        let sibling = block.children.first().ok_or_else(|| {
            invalid(
                block.span.offset,
                "insertion needs a sibling file for formatting",
            )
        })?;
        let start = sibling.span.offset as usize;
        let line_start = decoded[..start]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(start, |i| i + 1);
        let indent = &decoded[line_start..start];
        let own_line = indent.iter().all(|b| matches!(b, b' ' | b'\t'));
        let indent = if own_line {
            std::str::from_utf8(indent).unwrap()
        } else {
            ""
        };
        let ending = if !own_line || (line_start == start && !decoded[..start].contains(&b'\n')) {
            ""
        } else if line_start >= 2 && &decoded[line_start - 2..line_start] == b"\r\n" {
            "\r\n"
        } else {
            "\n"
        };
        let mut insertions = BTreeMap::<usize, String>::new();
        for file in new.iter() {
            let value = |name: &str, n| decimal(n, &sibling.attribute(name).unwrap().value);
            let text = format!(
                "<file begin=\"{}\" count=\"{}\" offset=\"{}\" enable=\"{}\">{}</file>",
                value("begin", file.begin),
                value("count", file.count),
                value("offset", file.offsets.value()),
                value("enable", file.enable.value()),
                decimal(file.data.value(), sibling.trimmed_text())
            );
            if let Some(next) = block.children.iter().find(|element| {
                xml::attribute_u32(element, "begin").is_ok_and(|begin| begin > file.begin)
            }) {
                insertions
                    .entry(next.span.offset as usize)
                    .or_default()
                    .push_str(&format!("{text}{ending}{indent}"));
            } else {
                insertions
                    .entry(block.children.last().unwrap().span.end() as usize)
                    .or_default()
                    .push_str(&format!("{ending}{indent}{text}"));
            }
        }
        for (at, text) in insertions {
            splices.push((at, at, text));
        }
        let SheetBody::Definition { blocks, .. } = &mut expected.sheets[*sheet].body else {
            unreachable!()
        };
        blocks.extend(new.iter().cloned());
        blocks.sort_by_key(|file| file.begin);
        let count = u32::try_from(blocks.len())
            .map_err(|_| invalid(block.span.offset, "too many files"))?;
        replace_count(decoded, block, count, &mut splices)?;
    }
    for sheet in &expected.sheets {
        if let SheetBody::Definition { blocks, .. } = &sheet.body {
            let mut end = 0;
            for file in blocks {
                let next = file
                    .begin
                    .checked_add(file.count)
                    .ok_or_else(|| invalid(file.span.offset, "file range overflows u32"))?;
                if file.begin < end {
                    return Err(invalid(
                        file.span.offset,
                        "file ranges overlap or are not sorted",
                    ));
                }
                end = next;
            }
        }
    }
    splices.sort_by_key(|(start, end, _)| (*start, *end));
    let mut result = Vec::new();
    let mut cursor = 0;
    for (start, end, text) in splices {
        if start < cursor {
            return Err(invalid(start as u64, "overlapping text edits"));
        }
        result.extend_from_slice(&decoded[cursor..start]);
        result.extend_from_slice(text.as_bytes());
        cursor = end;
    }
    result.extend_from_slice(&decoded[cursor..]);
    let parsed = ssd::parse_document(&result)?;
    if parsed.sheets.len() != expected.sheets.len() {
        return Err(invalid(0, "edited document changed its sheets"));
    }
    for (actual, expected) in parsed.sheets.iter().zip(&expected.sheets) {
        let attributes = |sheet: &ssd::Sheet| {
            sheet
                .attributes
                .iter()
                .map(|a| (a.name.clone(), a.value.clone()))
                .collect::<Vec<_>>()
        };
        let bodies_match = match (&actual.body, &expected.body) {
            (
                SheetBody::Definition {
                    columns: a,
                    index: ai,
                    blocks: ab,
                },
                SheetBody::Definition {
                    columns: b,
                    index: bi,
                    blocks: bb,
                },
            ) => a == b && ai == bi && ab.iter().map(signature).eq(bb.iter().map(signature)),
            (a, b) => a == b,
        };
        if actual.name != expected.name
            || attributes(actual) != attributes(expected)
            || !bodies_match
        {
            return Err(invalid(
                actual.span.offset,
                "edited document disagrees with intended definition",
            ));
        }
    }
    Ok(result)
}

/// Keep plain documents plain and scrambled documents scrambled.
pub fn rewrite_definition_container(original: &[u8], edits: &[DefinitionEdit]) -> Result<Vec<u8>> {
    if edits.is_empty() {
        return Ok(original.to_vec());
    }
    if original.last() == Some(&scrambled::TRAILER) {
        let edited = edit_definition(&scrambled::decode(original)?.document, edits)?;
        let encoded = scrambled::encode(&edited)?;
        if scrambled::decode(&encoded)?.document != edited {
            return Err(FormatError::new(
                ErrorKind::InvalidAttributeValue,
                0,
                "the re-encoded definition does not decode to the edited document",
            ));
        }
        Ok(encoded)
    } else {
        edit_definition(original, edits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENT: &str = "\u{feff}<?xml version=\"1.0\"?>\r\n<ssd version=\"0.1\">\r\n\t<sheet name=\"s\" lang=\"en\" infofile=\"\">\r\n\t\t<type><param>str</param></type>\r\n\t\t<index><param>0</param></index>\r\n\t\t<block count=\"2\">\r\n\t\t\t<file begin=\"10\" count=\"2\" offset=\"3\" enable=\"2\">1</file>\r\n\t\t\t<file begin=\"20\" count=\"2\" offset=\"6\" enable=\"5\">4</file>\r\n\t\t</block>\r\n\t</sheet>\r\n</ssd>\r\n";

    fn insert(begin: u32, count: u32, id: u32) -> DefinitionEdit {
        DefinitionEdit::InsertFile {
            sheet: 0,
            begin,
            count,
            data: ResourceId::new(id),
            enable: ResourceId::new(id + 1),
            offsets: ResourceId::new(id + 2),
        }
    }

    #[test]
    fn no_edits_preserve_plain_and_scrambled_bytes() {
        let scrambled = scrambled::encode(DOCUMENT.as_bytes()).unwrap();
        for original in [DOCUMENT.as_bytes(), &scrambled] {
            assert_eq!(
                rewrite_definition_container(original, &[]).unwrap(),
                original
            );
        }
        assert_eq!(
            edit_definition(DOCUMENT.as_bytes(), &[]).unwrap(),
            DOCUMENT.as_bytes()
        );
    }

    #[test]
    fn growing_changes_only_the_count_and_scrambled_round_trips() {
        let edits = [DefinitionEdit::GrowFile {
            sheet: 0,
            file: 0,
            count: 10,
        }];
        let expected = DOCUMENT.replace("begin=\"10\" count=\"2\"", "begin=\"10\" count=\"10\"");
        let plain = edit_definition(DOCUMENT.as_bytes(), &edits).unwrap();
        assert_eq!(plain, expected.as_bytes());
        assert_eq!(
            rewrite_definition_container(DOCUMENT.as_bytes(), &edits).unwrap(),
            plain
        );
        let original = scrambled::encode(DOCUMENT.as_bytes()).unwrap();
        let result = rewrite_definition_container(&original, &edits).unwrap();
        assert_eq!(scrambled::decode(&result).unwrap().document, plain);
    }

    #[test]
    fn insertions_copy_crlf_indentation_and_bump_block_count() {
        for indent in ["\t\t\t", "      "] {
            let original = DOCUMENT.replace("\t\t\t<file", &format!("{indent}<file"));
            let edits = [insert(30, 3, 13), insert(0, 2, 7), insert(15, 2, 10)];
            let result = edit_definition(original.as_bytes(), &edits).unwrap();
            let expected = original.replace("block count=\"2\"", "block count=\"5\"")
                .replace(&format!("{indent}<file begin=\"10\""), &format!("{indent}<file begin=\"0\" count=\"2\" offset=\"9\" enable=\"8\">7</file>\r\n{indent}<file begin=\"10\""))
                .replace(&format!("{indent}<file begin=\"20\""), &format!("{indent}<file begin=\"15\" count=\"2\" offset=\"12\" enable=\"11\">10</file>\r\n{indent}<file begin=\"20\""))
                .replace("enable=\"5\">4</file>", &format!("enable=\"5\">4</file>\r\n{indent}<file begin=\"30\" count=\"3\" offset=\"15\" enable=\"14\">13</file>"));
            assert_eq!(result, expected.as_bytes());
        }
    }

    #[test]
    fn decimal_padding_is_preserved() {
        let original = DOCUMENT
            .replace("begin=\"10\"", "begin=\"0010\"")
            .replace("count=\"2\"", "count=\"02\"");
        let output = edit_definition(original.as_bytes(), &[insert(0, 3, 7)]).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("<block count=\"03\">"));
        assert!(
            text.contains("<file begin=\"0000\" count=\"03\" offset=\"9\" enable=\"8\">7</file>")
        );
    }

    #[test]
    fn invalid_definition_edits_are_refused() {
        let grow = |count| DefinitionEdit::GrowFile {
            sheet: 0,
            file: 0,
            count,
        };
        for (edits, detail) in [
            (vec![grow(1)], "cannot shrink"),
            (vec![grow(11)], "overlap"),
            (vec![grow(u32::MAX)], "overflows"),
            (vec![grow(3), grow(4)], "same file"),
            (vec![insert(11, 2, 7)], "overlap"),
            (vec![insert(18, 3, 7)], "overlap"),
            (vec![insert(u32::MAX, 1, 7)], "overflows"),
            (vec![insert(30, 1, 1)], "already named"),
            (vec![insert(30, 1, 7), insert(40, 1, 9)], "already named"),
            (
                vec![DefinitionEdit::GrowFile {
                    sheet: 0,
                    file: 9,
                    count: 3,
                }],
                "file index",
            ),
            (
                vec![DefinitionEdit::GrowFile {
                    sheet: 9,
                    file: 0,
                    count: 3,
                }],
                "sheet index",
            ),
            (vec![insert(30, 1, 7), insert(30, 1, 10)], "overlap"),
        ] {
            let error = edit_definition(DOCUMENT.as_bytes(), &edits).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidAttributeValue);
            assert!(error.detail().contains(detail), "{error}");
        }
        let mut duplicate_id = insert(30, 1, 7);
        if let DefinitionEdit::InsertFile { enable, .. } = &mut duplicate_id {
            *enable = ResourceId::new(7);
        }
        assert!(edit_definition(DOCUMENT.as_bytes(), &[duplicate_id])
            .unwrap_err()
            .detail()
            .contains("already named"));
        let reference = b"<ssd version=\"0.1\"><sheet name=\"r\" infofile=\"7\"/></ssd>";
        assert!(edit_definition(reference, &[insert(0, 1, 7)])
            .unwrap_err()
            .detail()
            .contains("not a definition"));
    }
}
