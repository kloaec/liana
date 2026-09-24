//! BBQr ("Better Bitcoin QR"), used by Coldcard and Krux.
//!
//! Each part is `B$` + encoding + file type + total (2 base36 digits) + index (2 base36 digits)
//! followed by the data. See <https://bbqr.org/BBQr.html>.

use std::io::Read;

use flate2::read::DeflateDecoder;

use super::Error;

/// BBQr file types we care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Psbt,
    Transaction,
    Json,
    Unicode,
}

impl FileType {
    fn code(self) -> char {
        match self {
            FileType::Psbt => 'P',
            FileType::Transaction => 'T',
            FileType::Json => 'J',
            FileType::Unicode => 'U',
        }
    }

    fn from_code(c: char) -> Result<Self, Error> {
        match c {
            'P' => Ok(FileType::Psbt),
            'T' => Ok(FileType::Transaction),
            'J' => Ok(FileType::Json),
            'U' => Ok(FileType::Unicode),
            c => Err(Error::Unsupported(format!("BBQr file type '{c}'"))),
        }
    }
}

const HEADER_LEN: usize = 8;
const BASE32: base32::Alphabet = base32::Alphabet::Rfc4648 { padding: false };

/// Split `data` in BBQr parts of at most `max_chars` characters each (header included).
///
/// Data is base32 encoded so every part fits the QR alphanumeric mode. We never emit the
/// compressed 'Z' encoding: it mandates a 1KiB deflate window that the pure-Rust deflate backend
/// can't enforce, and devices must accept '2' anyway.
pub fn encode(data: &[u8], file_type: FileType, max_chars: usize) -> Vec<String> {
    let encoding = '2';
    let body = base32::encode(BASE32, data);

    // Base32 parts must hold a whole number of 5-byte groups (8 characters), except the last.
    let per_part = ((max_chars.saturating_sub(HEADER_LEN)) / 8).max(1) * 8;
    let chunks: Vec<&str> = body
        .as_bytes()
        .chunks(per_part)
        .map(|c| std::str::from_utf8(c).expect("base32 is ascii"))
        .collect();
    let total = chunks.len().max(1);

    chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            format!(
                "B${encoding}{}{}{}{chunk}",
                file_type.code(),
                base36(total),
                base36(i)
            )
        })
        .collect()
}

/// Whether a scanned string looks like a BBQr part.
pub fn is_part(s: &str) -> bool {
    s.len() >= HEADER_LEN && s.starts_with("B$")
}

/// Reassembles BBQr parts scanned in any order.
#[derive(Debug, Default)]
pub struct Decoder {
    encoding: char,
    file_type: Option<FileType>,
    parts: Vec<Option<String>>,
}

impl Decoder {
    pub fn receive(&mut self, part: &str) -> Result<(), Error> {
        if !is_part(part) {
            return Err(Error::Malformed("not a BBQr part".into()));
        }
        let header: Vec<char> = part[..HEADER_LEN].chars().collect();
        let encoding = header[2];
        let file_type = FileType::from_code(header[3])?;
        let total = parse_base36(&part[4..6])?;
        let index = parse_base36(&part[6..8])?;
        if total == 0 || index >= total {
            return Err(Error::Malformed("invalid BBQr part index".into()));
        }

        // A part from another transfer resets the decoder.
        if self.file_type != Some(file_type)
            || self.encoding != encoding
            || self.parts.len() != total
        {
            *self = Decoder {
                encoding,
                file_type: Some(file_type),
                parts: vec![None; total],
            };
        }
        self.parts[index] = Some(part[HEADER_LEN..].to_string());
        Ok(())
    }

    pub fn progress(&self) -> (usize, usize) {
        (
            self.parts.iter().filter(|p| p.is_some()).count(),
            self.parts.len(),
        )
    }

    /// The reassembled data, once every part was received.
    pub fn message(&self) -> Result<Option<(FileType, Vec<u8>)>, Error> {
        let Some(file_type) = self.file_type else {
            return Ok(None);
        };
        if self.parts.iter().any(Option::is_none) {
            return Ok(None);
        }
        let body: String = self.parts.iter().flatten().map(String::as_str).collect();
        let data = match self.encoding {
            'H' => hex_decode(&body)?,
            '2' => base32::decode(BASE32, &body)
                .ok_or_else(|| Error::Malformed("invalid BBQr base32".into()))?,
            'Z' => {
                let compressed = base32::decode(BASE32, &body)
                    .ok_or_else(|| Error::Malformed("invalid BBQr base32".into()))?;
                inflate(&compressed)?
            }
            c => return Err(Error::Unsupported(format!("BBQr encoding '{c}'"))),
        };
        Ok(Some((file_type, data)))
    }
}

fn inflate(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    DeflateDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| Error::Malformed(format!("invalid BBQr compressed data: {e}")))?;
    Ok(out)
}

fn base36(n: usize) -> String {
    const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let n = n.min(36 * 36 - 1);
    format!("{}{}", DIGITS[n / 36] as char, DIGITS[n % 36] as char)
}

fn parse_base36(s: &str) -> Result<usize, Error> {
    usize::from_str_radix(s, 36).map_err(|_| Error::Malformed("invalid BBQr header".into()))
}

fn hex_decode(s: &str) -> Result<Vec<u8>, Error> {
    if s.len() % 2 != 0 {
        return Err(Error::Malformed("invalid BBQr hex".into()));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|_| Error::Malformed("invalid BBQr hex".into()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_out_of_order() {
        let data: Vec<u8> = (0..2000u32).map(|i| (i * 7 % 251) as u8).collect();
        let parts = encode(&data, FileType::Psbt, 300);
        assert!(parts.len() > 1);
        assert!(parts.iter().all(|p| p.len() <= 300));
        assert!(parts[0].starts_with("B$2P"));

        let mut decoder = Decoder::default();
        for part in parts.iter().rev() {
            decoder.receive(part).unwrap();
        }
        let (file_type, decoded) = decoder.message().unwrap().unwrap();
        assert_eq!(file_type, FileType::Psbt);
        assert_eq!(decoded, data);
    }

    #[test]
    fn decodes_compressed_parts() {
        use flate2::{write::DeflateEncoder, Compression};
        use std::io::Write;
        let text = "wsh(or_d(pk(A),and_v(v:pkh(B),older(52560))))".repeat(4);
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(text.as_bytes()).unwrap();
        let body = base32::encode(BASE32, &encoder.finish().unwrap());
        let mut decoder = Decoder::default();
        decoder.receive(&format!("B$ZU0100{body}")).unwrap();
        assert_eq!(decoder.message().unwrap().unwrap().1, text.as_bytes());
    }

    #[test]
    fn uncompressed_text() {
        let parts = encode(b"hi", FileType::Unicode, 300);
        assert_eq!(parts, vec!["B$2U0100NBUQ".to_string()]);
        let mut decoder = Decoder::default();
        decoder.receive(&parts[0]).unwrap();
        assert_eq!(decoder.message().unwrap().unwrap().1, b"hi");
    }
}
