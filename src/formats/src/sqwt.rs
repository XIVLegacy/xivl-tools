//! The SQEX container the client's UI resources are stored in.
//!
//! A SQEX file is an eight-byte signature followed by an enciphered body.
//! The body's whole 64-bit blocks are enciphered under a key derived from
//! the file's own name. A final run shorter than a block is left in the
//! clear, which is why the last few bytes of these files read as ordinary
//! markup. Nothing here interprets the decoded document.
//!
//! The key is not in the file. It is the file's base name, so a decode
//! needs the name as well as the bytes, and this crate never learns a name
//! from a path of its own: the caller supplies it.
//!
//! Byte-layout evidence and its retail citation: `docs/formats/sqex.md`,
//! "The SQEX container".

use crate::blowfish::{Blowfish, BLOCK_SIZE};
use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::Span;

/// The eight-byte signature: the tag, then a word that is zero in every
/// one of the 1155 files in the install. Both halves are the signature,
/// because a file carrying the tag and something else at 0x04 is not a
/// container this crate has ever seen.
pub const SIGNATURE: &[u8; 8] = b"SQEX\x00\x00\x00\x00";

/// Where the enciphered body begins.
pub const HEADER_SIZE: usize = SIGNATURE.len();

/// A decoded SQEX container and the facts the container itself states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqwtFile {
    /// The signature.
    pub header: Span,
    /// The whole blocks that were enciphered.
    pub enciphered: Span,
    /// The final run shorter than a block, carried in the clear. Zero
    /// length when the body divides evenly.
    pub plaintext_tail: Span,
    /// How many whole blocks the body holds.
    pub block_count: u64,
    /// The name the key was derived from, as the caller supplied it.
    pub key_name: String,
    /// The decoded body. Offsets into anything parsed out of it are
    /// relative to this buffer, not to the file.
    pub document: Vec<u8>,
}

/// Does this input carry the container signature?
///
/// The signature is the whole test here, unlike the scrambled container
/// whose recognition needs its decode: eight bytes is a strong enough tag
/// that nothing else in the install shares it, and the decode needs a name
/// this function is not given.
pub fn has_signature(data: &[u8]) -> bool {
    data.starts_with(SIGNATURE)
}

/// Decode a SQEX container, given the file's base name.
///
/// `name` is the key. It is the base name including its suffix, exactly as
/// the client stores it. A name that differs in case or suffix is a
/// different key and produces a different, meaningless body.
pub fn decode(data: &[u8], name: &str) -> Result<SqwtFile> {
    if data.len() < HEADER_SIZE {
        return Err(FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            data.len() as u64,
            format!(
                "a SQEX container opens with {HEADER_SIZE} signature bytes; this input has {}",
                data.len()
            ),
        ));
    }
    if !has_signature(data) {
        // Naming which half failed keeps the two apart: a foreign tag and a
        // reserved word this crate has never seen are different findings.
        let detail = if data.starts_with(b"SQEX") {
            "the word at 0x04 is not zero, and every SQEX container observed sets it to zero"
        } else {
            "the leading bytes are not the SQEX container signature"
        };
        let offset = if data.starts_with(b"SQEX") { 4 } else { 0 };
        return Err(FormatError::new(ErrorKind::BadMagic, offset, detail));
    }

    let cipher = Blowfish::new(name.as_bytes())?;
    let mut document = data[HEADER_SIZE..].to_vec();
    cipher.decrypt_blocks(&mut document);

    let block_count = (document.len() / BLOCK_SIZE) as u64;
    let enciphered_length = block_count * BLOCK_SIZE as u64;
    Ok(SqwtFile {
        header: Span::new(0, HEADER_SIZE as u64),
        enciphered: Span::new(HEADER_SIZE as u64, enciphered_length),
        plaintext_tail: Span::new(
            HEADER_SIZE as u64 + enciphered_length,
            document.len() as u64 - enciphered_length,
        ),
        block_count,
        key_name: name.to_string(),
        document,
    })
}

/// Build a container from a document, which is what makes the decode a
/// checked round trip rather than a claim about itself.
pub fn encode(document: &[u8], name: &str) -> Result<Vec<u8>> {
    let cipher = Blowfish::new(name.as_bytes())?;
    let mut body = document.to_vec();
    cipher.encrypt_blocks(&mut body);
    let mut out = SIGNATURE.to_vec();
    out.extend_from_slice(&body);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENT: &[u8] = b"<Window Name=\"a\">\r\n\t<Grid />\r\n</Window>";

    #[test]
    fn a_round_trip_reproduces_the_document() {
        // Every remainder modulo the block size, because the tail run is
        // where a decoder that enciphers the whole body goes wrong.
        for extra in 0..8 {
            let mut plain = DOCUMENT.to_vec();
            plain.extend(std::iter::repeat_n(b' ', extra));
            let encoded = encode(&plain, "widget.form").unwrap();
            let decoded = decode(&encoded, "widget.form").unwrap();
            assert_eq!(decoded.document, plain, "extra {extra}");
            assert_eq!(
                decoded.plaintext_tail.length,
                (plain.len() % BLOCK_SIZE) as u64
            );
            assert_eq!(decoded.block_count, (plain.len() / BLOCK_SIZE) as u64);
        }
    }

    #[test]
    fn the_tail_shorter_than_a_block_stays_in_the_clear() {
        let mut plain = DOCUMENT.to_vec();
        plain.extend(b"</Root>");
        let tail = plain.len() % BLOCK_SIZE;
        assert_ne!(tail, 0, "the fixture text must not divide evenly");
        let encoded = encode(&plain, "widget.form").unwrap();
        assert_eq!(
            &encoded[encoded.len() - tail..],
            &plain[plain.len() - tail..]
        );
    }

    #[test]
    fn the_key_is_the_name() {
        let encoded = encode(DOCUMENT, "widget.form").unwrap();
        let wrong = decode(&encoded, "Widget.form").unwrap();
        assert_ne!(wrong.document, DOCUMENT);
        assert_eq!(wrong.key_name, "Widget.form");
        let right = decode(&encoded, "widget.form").unwrap();
        assert_eq!(right.document, DOCUMENT);
    }

    #[test]
    fn the_spans_tile_the_input() {
        let encoded = encode(b"0123456789012", "widget.form").unwrap();
        let decoded = decode(&encoded, "widget.form").unwrap();
        assert_eq!(decoded.header, Span::new(0, 8));
        assert_eq!(decoded.enciphered, Span::new(8, 8));
        assert_eq!(decoded.plaintext_tail, Span::new(16, 5));
        assert_eq!(encoded.len() as u64, decoded.plaintext_tail.end());
    }

    #[test]
    fn a_foreign_signature_is_refused() {
        let error = decode(b"GTEX\x00\x00\x00\x00rest", "a.gtex").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::BadMagic);
        assert_eq!(error.offset(), 0);

        let error = decode(b"SQEX\x01\x00\x00\x00rest", "a.form").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::BadMagic);
        assert_eq!(error.offset(), 4);

        let error = decode(b"SQEX", "a.form").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEndOfInput);
        assert_eq!(error.offset(), 4);

        assert!(!has_signature(b"SEDB\x00\x00\x00\x00"));
        assert!(has_signature(SIGNATURE));
    }

    #[test]
    fn a_decode_without_a_name_says_so() {
        let encoded = encode(DOCUMENT, "widget.form").unwrap();
        let error = decode(&encoded, "").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::MissingContainerName);
    }

    #[test]
    fn an_empty_body_is_a_container_with_no_blocks() {
        let decoded = decode(SIGNATURE, "widget.form").unwrap();
        assert_eq!(decoded.block_count, 0);
        assert!(decoded.document.is_empty());
        assert_eq!(decoded.plaintext_tail.length, 0);
    }
}
