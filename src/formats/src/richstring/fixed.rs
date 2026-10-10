//! Bounded selector-0x2E diagnostics. Evidence: docs/formats/fixed-control.md.

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control<'a> {
    pub span: Span,
    pub raw: &'a [u8],
    pub selector: u8,
    /// None means an opaque unsupported selector; its span covers the remaining input.
    pub numeric_fields: Option<[u32; 2]>,
}

#[derive(Debug)]
pub enum MessageSegment<'a> {
    Literal {
        span: Span,
        raw: &'a [u8],
    },
    Control(Control<'a>),
    Error {
        span: Span,
        raw: &'a [u8],
        error: FormatError,
    },
}

pub struct MessageSegments<'a> {
    data: &'a [u8],
    position: usize,
}

/// Walk supplied message bytes using the diagnostic fixed-control grammar.
/// Unknown or malformed controls consume the opaque remainder without resynchronizing.
pub fn message_segments(data: &[u8]) -> MessageSegments<'_> {
    MessageSegments { data, position: 0 }
}

impl<'a> Iterator for MessageSegments<'a> {
    type Item = MessageSegment<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let start = self.position;
        if start == self.data.len() {
            return None;
        }
        let tail = &self.data[start..];
        if tail[0] != 0x02 {
            let length = tail
                .iter()
                .position(|&byte| byte == 0x02)
                .unwrap_or(tail.len());
            self.position += length;
            return Some(MessageSegment::Literal {
                span: Span::new(start as u64, length as u64),
                raw: &tail[..length],
            });
        }
        match decode(self.data, start) {
            Ok(control) => {
                self.position += control.raw.len();
                Some(MessageSegment::Control(control))
            }
            Err(error) => {
                self.position = self.data.len();
                Some(MessageSegment::Error {
                    span: Span::new(start as u64, tail.len() as u64),
                    raw: tail,
                    error,
                })
            }
        }
    }
}

/// Decode one control at `offset`, leaving bytes after a supported control unread.
/// Unknown selectors retain the input remainder without interpreting its framing.
pub fn decode(data: &[u8], offset: usize) -> Result<Control<'_>> {
    let tail = data.get(offset..).ok_or_else(|| {
        failure(
            ErrorKind::UnexpectedEndOfInput,
            offset,
            "offset exceeds input",
        )
    })?;
    if byte(tail, 0, offset)? != 0x02 {
        return Err(malformed(offset, "control must start with 0x02"));
    }
    let selector = byte(tail, 1, offset)?;
    if selector != 0x2E {
        return Ok(Control {
            span: Span::new(offset as u64, tail.len() as u64),
            raw: tail,
            selector,
            numeric_fields: None,
        });
    }

    let (length, width) = number(&tail[2..], offset + 2)?;
    let begin = 2 + width;
    let length = usize::try_from(length)
        .map_err(|_| malformed(offset + 2, "payload length exceeds address space"))?;
    let end = begin
        .checked_add(length)
        .ok_or_else(|| malformed(offset + 2, "payload length exceeds address space"))?;
    let payload = tail.get(begin..end).ok_or_else(|| {
        failure(
            ErrorKind::UnexpectedEndOfInput,
            data.len(),
            "declared payload exceeds available input",
        )
    })?;
    if byte(tail, end, offset)? != 0x03 {
        return Err(malformed(offset + end, "control must close with 0x03"));
    }
    // Restrict both numeric reads to the payload, never the terminator or next control.
    let (first, first_width) = number(payload, offset + begin)?;
    let (second, second_width) = number(&payload[first_width..], offset + begin + first_width)?;
    if first_width + second_width != payload.len() {
        return Err(malformed(
            offset + begin + first_width + second_width,
            "payload must contain exactly two numeric fields",
        ));
    }
    Ok(Control {
        span: Span::new(offset as u64, (end + 1) as u64),
        raw: &tail[..end + 1],
        selector,
        numeric_fields: Some([first, second]),
    })
}

fn number(data: &[u8], base: usize) -> Result<(u32, usize)> {
    let lead = byte(data, 0, base)?;
    // Chat's literal-number admission is wider than the sheet expression short range.
    if (1..=0xD7).contains(&lead) {
        return Ok((u32::from(lead - 1), 1));
    }
    if !(0xF0..=0xFE).contains(&lead) {
        return Err(malformed(base, "invalid numeric lead"));
    }
    let mask = lead - 0xEF;
    let mut value = 0u32;
    let mut width = 1;
    for shift in [24, 16, 8, 0] {
        if mask & (1 << (shift / 8)) != 0 {
            let part = byte(data, width, base)?;
            if part == 0 {
                return Err(malformed(
                    base + width,
                    "zero selected byte is outside the authenticated producer encoding",
                ));
            }
            value |= u32::from(part) << shift;
            width += 1;
        }
    }
    Ok((value, width))
}

fn byte(data: &[u8], position: usize, base: usize) -> Result<u8> {
    data.get(position).copied().ok_or_else(|| {
        failure(
            ErrorKind::UnexpectedEndOfInput,
            base + position,
            "missing control byte",
        )
    })
}

fn malformed(offset: usize, detail: &str) -> FormatError {
    failure(ErrorKind::MalformedRichStringToken, offset, detail)
}

fn failure(kind: ErrorKind, offset: usize, detail: &str) -> FormatError {
    FormatError::new(kind, offset as u64, detail)
}
