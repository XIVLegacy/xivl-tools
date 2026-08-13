//! SSD documents: the master schema and the per-sheet schema.
//!
//! Both are the same document shape - a `<ssd>` root holding `<sheet>`
//! elements - and which one a file is follows from what its sheets carry.
//! A master's sheet is a reference: a name and the decimal resource id of
//! the document that defines it. A sheet document's sheet is a definition:
//! a column type list, an index list, and the resource ids of the data,
//! enable, and row-offset files of each block.
//!
//! Byte-layout evidence and its retail citation: `docs/formats/ssd-sheet.md`,
//! "The SSD document stack".

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::Span;
use crate::resource::ResourceId;
use crate::sheet::ColumnType;
use crate::xml::{self, Attribute, Document, Element};

/// Root element name of every SSD document.
pub const ROOT_ELEMENT: &str = "ssd";

/// What a `<sheet>` element says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SheetBody {
    /// A master entry: a name and the document that defines the sheet.
    Reference { infofile: ResourceId },
    /// A definition: columns, index, and the file blocks holding rows.
    Definition {
        columns: Vec<ColumnType>,
        index: Vec<u32>,
        blocks: Vec<FileBlock>,
    },
}

/// One `<file>` element: a contiguous run of row identifiers and the three
/// resources that hold them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileBlock {
    pub span: Span,
    /// Identifier of the first row this block covers.
    pub begin: u32,
    /// Number of row slots, which is also the length of the row-offset
    /// array. Most of them are empty in a sparse sheet.
    pub count: u32,
    pub data: ResourceId,
    pub enable: ResourceId,
    pub offsets: ResourceId,
}

/// One `<sheet>` element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sheet {
    pub span: Span,
    pub name: String,
    /// Every attribute verbatim and in document order, so an attribute
    /// this crate does not interpret still reaches the report.
    pub attributes: Vec<Attribute>,
    pub body: SheetBody,
}

impl Sheet {
    pub fn kind_name(&self) -> &'static str {
        match self.body {
            SheetBody::Reference { .. } => "reference",
            SheetBody::Definition { .. } => "definition",
        }
    }
}

/// A whole SSD document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsdDocument {
    pub byte_order_mark: bool,
    pub declaration: Option<xml::Declaration>,
    pub version: String,
    pub sheets: Vec<Sheet>,
}

impl SsdDocument {
    /// The support-matrix format id this document belongs to.
    ///
    /// A document of references is a master. A document of definitions is
    /// a sheet schema. No document in the 1.23b install mixes the two, and
    /// one that did would be reported as `ssd-mixed` rather than silently
    /// filed under either.
    pub fn format_id(&self) -> &'static str {
        let references = self
            .sheets
            .iter()
            .filter(|sheet| matches!(sheet.body, SheetBody::Reference { .. }))
            .count();
        if references == self.sheets.len() {
            "ssd-master"
        } else if references == 0 {
            "ssd-sheet"
        } else {
            "ssd-mixed"
        }
    }
}

/// Does this input look like an SSD document?
pub fn has_document_signature(data: &[u8]) -> bool {
    let body = data.strip_prefix(xml::BYTE_ORDER_MARK).unwrap_or(data);
    body.starts_with(b"<")
}

/// Parse an SSD document from its bytes.
pub fn parse_document(data: &[u8]) -> Result<SsdDocument> {
    let document: Document = xml::parse_document(data)?;
    if document.root.name != ROOT_ELEMENT {
        return Err(FormatError::new(
            ErrorKind::UnexpectedElement,
            document.root.span.offset,
            format!(
                "root element is '{}', not '{ROOT_ELEMENT}'",
                document.root.name
            ),
        ));
    }
    let version = document
        .root
        .attribute("version")
        .map(|attribute| attribute.value.clone())
        .ok_or_else(|| {
            FormatError::new(
                ErrorKind::MalformedXml,
                document.root.span.offset,
                "the ssd element has no version attribute",
            )
        })?;

    let mut sheets = Vec::new();
    for child in &document.root.children {
        if child.name != "sheet" {
            return Err(FormatError::new(
                ErrorKind::UnexpectedElement,
                child.span.offset,
                format!("'{}' is not a 'sheet' element", child.name),
            ));
        }
        sheets.push(parse_sheet(child)?);
    }

    Ok(SsdDocument {
        byte_order_mark: document.byte_order_mark,
        declaration: document.declaration,
        version,
        sheets,
    })
}

fn parse_sheet(element: &Element) -> Result<Sheet> {
    let name = element
        .attribute("name")
        .map(|attribute| attribute.value.clone())
        .ok_or_else(|| {
            FormatError::new(
                ErrorKind::MalformedXml,
                element.span.offset,
                "a sheet element has no name attribute",
            )
        })?;

    let body = if element.children.is_empty() {
        // A master entry. `infofile` is a decimal resource id here, and an
        // empty string on a definition, which is why an empty value is not
        // treated as a reference.
        let attribute = element.attribute("infofile").ok_or_else(|| {
            FormatError::new(
                ErrorKind::MalformedXml,
                element.span.offset,
                format!("sheet '{name}' has neither children nor an infofile"),
            )
        })?;
        let value = xml::parse_u32(&attribute.value, attribute.span.offset, "infofile")?;
        SheetBody::Reference {
            infofile: ResourceId::new(value),
        }
    } else {
        let mut types = None;
        let mut index_element = None;
        let mut block_element = None;
        for child in &element.children {
            let slot = match child.name.as_str() {
                "type" => &mut types,
                "index" => &mut index_element,
                "block" => &mut block_element,
                _ => {
                    return Err(FormatError::new(
                        ErrorKind::UnexpectedElement,
                        child.span.offset,
                        format!("'{}' is not valid inside a 'sheet' element", child.name),
                    ))
                }
            };
            if slot.replace(child).is_some() {
                return Err(FormatError::new(
                    ErrorKind::UnexpectedElement,
                    child.span.offset,
                    format!("sheet '{name}' has more than one '{}' element", child.name),
                ));
            }
        }

        let types = types.ok_or_else(|| {
            FormatError::new(
                ErrorKind::MalformedXml,
                element.span.offset,
                format!("sheet '{name}' has no type element"),
            )
        })?;
        let mut columns = Vec::new();
        for param in &types.children {
            expect_param(param)?;
            columns.push(ColumnType::parse(param.trimmed_text(), param.span.offset)?);
        }

        let mut index = Vec::new();
        if let Some(element) = index_element {
            for param in &element.children {
                expect_param(param)?;
                index.push(xml::parse_u32(
                    param.trimmed_text(),
                    param.span.offset,
                    "index param",
                )?);
            }
        }

        let mut blocks = Vec::new();
        if let Some(block) = block_element {
            let declared = xml::attribute_u32(block, "count")?;
            for file in &block.children {
                if file.name != "file" {
                    return Err(FormatError::new(
                        ErrorKind::UnexpectedElement,
                        file.span.offset,
                        format!("'{}' is not a 'file' element", file.name),
                    ));
                }
                blocks.push(parse_file_block(file)?);
            }
            if declared as usize != blocks.len() {
                return Err(FormatError::new(
                    ErrorKind::InvalidAttributeValue,
                    block.span.offset,
                    format!(
                        "block declares {declared} file(s) and holds {}",
                        blocks.len()
                    ),
                ));
            }
        }

        SheetBody::Definition {
            columns,
            index,
            blocks,
        }
    };

    Ok(Sheet {
        span: element.span,
        name,
        attributes: element.attributes.clone(),
        body,
    })
}

fn expect_param(element: &Element) -> Result<()> {
    if element.name == "param" {
        return Ok(());
    }
    Err(FormatError::new(
        ErrorKind::UnexpectedElement,
        element.span.offset,
        format!("'{}' is not a 'param' element", element.name),
    ))
}

fn parse_file_block(element: &Element) -> Result<FileBlock> {
    let data = xml::parse_u32(element.trimmed_text(), element.span.offset, "file content")?;
    Ok(FileBlock {
        span: element.span,
        begin: xml::attribute_u32(element, "begin")?,
        count: xml::attribute_u32(element, "count")?,
        data: ResourceId::new(data),
        enable: ResourceId::new(xml::attribute_u32(element, "enable")?),
        offsets: ResourceId::new(xml::attribute_u32(element, "offset")?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n\
        <ssd version=\"0.1\">\r\n\
        \x20 <sheet name=\"xtx/_text_error\" infofile=\"664076291\" />\r\n\
        </ssd>\r\n";

    const SHEET: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n\
        <ssd version=\"0.1\">\r\n\
        \x20 <sheet name=\"s\" mode=\"client\" column_max=\"4\" column_count=\"1\" \
        cache=\"-1\" type=\"none\" lang=\"ja\" infofile=\"\">\r\n\
        \x20   <type><param>str</param><param>s32</param></type>\r\n\
        \x20   <index><param>3</param></index>\r\n\
        \x20   <block count=\"1\">\r\n\
        \x20     <file begin=\"10000\" count=\"7\" offset=\"3\" enable=\"2\">1</file>\r\n\
        \x20   </block>\r\n\
        \x20 </sheet>\r\n\
        </ssd>\r\n";

    #[test]
    fn a_master_is_a_document_of_references() {
        let document = parse_document(MASTER.as_bytes()).unwrap();
        assert_eq!(document.format_id(), "ssd-master");
        assert_eq!(document.version, "0.1");
        assert_eq!(document.sheets.len(), 1);
        match document.sheets[0].body {
            SheetBody::Reference { infofile } => {
                assert_eq!(infofile, ResourceId::new(0x2795_0003));
                assert_eq!(infofile.dat_path(), "data/27/95/00/03.DAT");
            }
            _ => panic!("a master sheet parsed as a definition"),
        }
    }

    #[test]
    fn a_sheet_document_carries_columns_index_and_blocks() {
        let document = parse_document(SHEET.as_bytes()).unwrap();
        assert_eq!(document.format_id(), "ssd-sheet");
        let sheet = &document.sheets[0];
        // Every attribute survives, including the ones this crate does not
        // interpret.
        let names: Vec<&str> = sheet
            .attributes
            .iter()
            .map(|attribute| attribute.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "name",
                "mode",
                "column_max",
                "column_count",
                "cache",
                "type",
                "lang",
                "infofile"
            ]
        );
        match &sheet.body {
            SheetBody::Definition {
                columns,
                index,
                blocks,
            } => {
                assert_eq!(columns, &[ColumnType::Text, ColumnType::Signed32]);
                assert_eq!(index, &[3]);
                assert_eq!(blocks.len(), 1);
                assert_eq!(blocks[0].begin, 10000);
                assert_eq!(blocks[0].count, 7);
                assert_eq!(blocks[0].data, ResourceId::new(1));
                assert_eq!(blocks[0].enable, ResourceId::new(2));
                assert_eq!(blocks[0].offsets, ResourceId::new(3));
            }
            _ => panic!("a definition parsed as a reference"),
        }
    }

    #[test]
    fn a_wrong_root_is_named_rather_than_guessed() {
        let error = parse_document(b"<sdd version=\"0.1\"><sheet name=\"a\"/></sdd>").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedElement);
        assert_eq!(error.offset(), 0);
    }

    #[test]
    fn a_block_that_miscounts_its_files_fails() {
        let text = SHEET.replace("block count=\"1\"", "block count=\"2\"");
        let error = parse_document(text.as_bytes()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidAttributeValue);
    }

    #[test]
    fn unknown_and_duplicate_sheet_children_are_rejected() {
        let unknown = SHEET.replace(
            "<index><param>3</param></index>",
            "<index><param>3</param></index><mystery>secret</mystery>",
        );
        let error = parse_document(unknown.as_bytes()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedElement);
        assert!(error.detail().contains("mystery"));

        let duplicate = SHEET.replace(
            "<index><param>3</param></index>",
            "<index><param>3</param></index><index><param>4</param></index>",
        );
        let error = parse_document(duplicate.as_bytes()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedElement);
        assert!(error.detail().contains("more than one 'index'"));
    }

    #[test]
    fn a_non_numeric_infofile_fails_with_its_offset() {
        let text = MASTER.replace("664076291", "later");
        let error = parse_document(text.as_bytes()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidAttributeValue);
    }

    #[test]
    fn the_signature_test_accepts_both_byte_order_mark_forms() {
        assert!(has_document_signature(b"\xEF\xBB\xBF<?xml"));
        assert!(has_document_signature(b"<ssd"));
        assert!(!has_document_signature(b"SEDB"));
        assert!(!has_document_signature(b""));
    }
}
