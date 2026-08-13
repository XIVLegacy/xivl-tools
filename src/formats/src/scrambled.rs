//! The scrambled-XML container.
//!
//! A scrambled resource is an ordinary XML document behind two reversible
//! steps and a one-byte trailer. Nothing here interprets the document: the
//! decode hands plain bytes to `xml` and `ssd`, so a decoded document and a
//! plaintext one go through the same reader and cannot drift.
//!
//! Byte-layout evidence and its retail citation: `docs/formats/ssd-sheet.md`,
//! "The scrambled XML container".

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::Span;

/// Final byte of every scrambled resource. A file that does not end in it
/// is not scrambled, and the client's own reader hands such a file on
/// untouched.
pub const TRAILER: u8 = 0xF1;

/// Bytes 6 and 7 of every document are "ml", from `<?xml` behind the byte
/// order mark. The second word key is recovered from them, so a decoder
/// needs no stored key.
pub const KNOWN_PLAINTEXT_WORD: u16 = 0x6C6D;

/// Byte order mark plus the opening of the declaration. A decode that does
/// not produce this is not a scrambled document, whatever its trailer says.
pub const DECODED_SIGNATURE: &[u8] = b"\xEF\xBB\xBF<?xml";

/// Shortest encoded body the decode is defined for: the second key is read
/// from bytes 6 and 7, so a shorter body has no key to recover.
pub const MINIMUM_ENCODED_LENGTH: usize = 8;

/// A decoded scrambled document and the container facts behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrambledXml {
    /// The encoded body: everything but the trailer.
    pub encoded: Span,
    /// The trailer byte.
    pub trailer: Span,
    /// Word key for the first half of each four-byte group, derived from
    /// the encoded length alone.
    pub key_a: u16,
    /// Word key for the second half, recovered from known plaintext.
    pub key_b: u16,
    /// Whether the final byte took the extra correction. It does when the
    /// encoded length leaves it outside both word passes.
    pub final_byte_corrected: bool,
    /// The decoded document. Offsets in anything parsed out of it are
    /// relative to this buffer, not to the encoded input.
    pub document: Vec<u8>,
}

/// Does this input decode as a scrambled document?
///
/// The test is the decode itself. A trailer byte alone is not a signature:
/// resources exist that end in it and are not documents, and calling them
/// scrambled because of one byte would be a guess.
pub fn has_signature(data: &[u8]) -> bool {
    decode(data).is_ok()
}

/// Decode a scrambled resource.
///
/// Two reversible steps, in this order. First the partial reversal is
/// undone: the byte at 0 trades places with the last, the byte at 2 with
/// the third from last, and so on inward by two from each end, leaving the
/// bytes each side skips untouched. Then each four-byte group is two
/// little-endian words, the first exclusive-ored with a key derived from
/// the encoded length and the second with a key recovered from the
/// document's own known opening.
pub fn decode(data: &[u8]) -> Result<ScrambledXml> {
    let Some((&last, body)) = data.split_last() else {
        return Err(FormatError::new(
            ErrorKind::MissingScrambleTrailer,
            0,
            "an empty input has no trailer byte",
        ));
    };
    if last != TRAILER {
        return Err(FormatError::new(
            ErrorKind::MissingScrambleTrailer,
            body.len() as u64,
            format!("a scrambled document ends with 0x{TRAILER:02x}, not 0x{last:02x}"),
        ));
    }
    let encoded_length = body.len();
    if encoded_length < MINIMUM_ENCODED_LENGTH {
        return Err(FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            encoded_length as u64,
            format!(
                "a scrambled document needs {MINIMUM_ENCODED_LENGTH} encoded bytes to \
                 carry its keys; this one has {encoded_length}"
            ),
        ));
    }

    let mut buffer = body.to_vec();
    unscramble(&mut buffer);

    let key_a = (encoded_length as u32).wrapping_mul(7) as u16;
    let key_b = u16::from_le_bytes([buffer[6], buffer[7]]) ^ KNOWN_PLAINTEXT_WORD;
    apply_word_key(&mut buffer, 0, key_a);
    apply_word_key(&mut buffer, 2, key_b);

    // The word passes stop before the last byte, so an encoded length one
    // past a group boundary leaves it untouched. Only that residue takes
    // the correction. The other three end on a byte a word pass covered.
    let final_byte_corrected = encoded_length % 4 == 1;
    if final_byte_corrected {
        buffer[encoded_length - 1] ^= (key_a as u8) ^ (key_b as u8);
    }

    if !buffer.starts_with(DECODED_SIGNATURE) {
        return Err(FormatError::new(
            ErrorKind::BadMagic,
            0,
            "the decode does not open on a byte order mark and an XML declaration, \
             so this resource is not a scrambled document",
        ));
    }

    Ok(ScrambledXml {
        encoded: Span::new(0, encoded_length as u64),
        trailer: Span::new(encoded_length as u64, 1),
        key_a,
        key_b,
        final_byte_corrected,
        document: buffer,
    })
}

/// Undo the partial reversal. It is its own inverse, which is why the
/// encoder and the decoder run the same walk.
fn unscramble(buffer: &mut [u8]) {
    if buffer.is_empty() {
        return;
    }
    let mut low = 0usize;
    let mut high = buffer.len() - 1;
    while low < high {
        buffer.swap(low, high);
        low += 2;
        if high < 2 {
            break;
        }
        high -= 2;
    }
}

/// Exclusive-or the little-endian word at `start`, then every fourth byte
/// after it, stopping before the final byte the way the format does.
fn apply_word_key(buffer: &mut [u8], start: usize, key: u16) {
    if buffer.len() < 2 {
        return;
    }
    let limit = buffer.len() - 1;
    let bytes = key.to_le_bytes();
    let mut offset = start;
    while offset < limit {
        // `offset < limit` is `offset + 1 < buffer.len()`, so the whole
        // word is in range and the last byte is never a half word.
        buffer[offset] ^= bytes[0];
        buffer[offset + 1] ^= bytes[1];
        offset += 4;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a document the way the client does, so a round trip proves
    /// the decode rather than restating it.
    ///
    /// `key_b` is the encoder's free choice: nothing stores it, and the
    /// decoder gets it back only because the plaintext at 6..8 is known.
    /// Passing several values is what tests that recovery.
    pub(crate) fn encode(document: &[u8], key_b: u16) -> Vec<u8> {
        let encoded_length = document.len();
        let mut buffer = document.to_vec();
        let key_a = (encoded_length as u32).wrapping_mul(7) as u16;
        if encoded_length % 4 == 1 {
            buffer[encoded_length - 1] ^= (key_a as u8) ^ (key_b as u8);
        }
        apply_word_key(&mut buffer, 2, key_b);
        apply_word_key(&mut buffer, 0, key_a);
        unscramble(&mut buffer);
        buffer.push(TRAILER);
        buffer
    }

    fn document(body: &str) -> Vec<u8> {
        let mut out = b"\xEF\xBB\xBF<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n".to_vec();
        out.extend_from_slice(body.as_bytes());
        out
    }

    #[test]
    fn a_round_trip_reproduces_the_document() {
        // Every residue of the encoded length modulo four, against several
        // second keys, because the final byte and the key recovery are the
        // two places the residue and the key can be got wrong.
        for extra in 0..8 {
            for key_b in [0x0000, 0x1234, 0xD2E8, 0xFFFF] {
                let text = format!("<ssd version=\"0.1\">{}</ssd>\r\n", " ".repeat(extra));
                let plain = document(&text);
                let decoded = decode(&encode(&plain, key_b)).unwrap();
                assert_eq!(decoded.document, plain, "extra {extra} key {key_b:04x}");
                assert_eq!(decoded.key_b, key_b);
            }
        }
    }

    #[test]
    fn the_word_keys_are_derived_not_stored() {
        let plain = document("<ssd version=\"0.1\"></ssd>\r\n");
        let encoded = encode(&plain, 0xBEEF);
        let decoded = decode(&encoded).unwrap();
        assert_eq!(
            decoded.key_a,
            ((encoded.len() - 1) as u32).wrapping_mul(7) as u16
        );
        assert_eq!(decoded.key_b, 0xBEEF);
        assert_eq!(decoded.encoded.length, (encoded.len() - 1) as u64);
        assert_eq!(decoded.trailer.length, 1);
    }

    #[test]
    fn a_missing_trailer_is_not_a_scrambled_document() {
        let error = decode(b"<?xml version=\"1.0\"?>").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::MissingScrambleTrailer);
        let error = decode(b"").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::MissingScrambleTrailer);
        assert!(!has_signature(b"SEDB\x00\x00\x00\x00"));
    }

    #[test]
    fn a_short_body_reports_the_end_of_the_input() {
        let error = decode(b"\x00\x01\x02\xF1").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEndOfInput);
        assert_eq!(error.offset(), 3);
    }

    #[test]
    fn a_trailer_on_something_else_fails_rather_than_decoding_it() {
        let mut data = vec![0x42u8; 64];
        data.push(TRAILER);
        let error = decode(&data).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::BadMagic);
        assert_eq!(error.offset(), 0);
    }
}
