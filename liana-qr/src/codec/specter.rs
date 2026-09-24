//! Specter DIY's animated text transport: each part is `pMofN <chunk>`, chunks concatenated in
//! order. Krux understands it as well.

use super::Error;

pub fn encode(text: &str, max_chars: usize) -> Vec<String> {
    // Reserve room for the "pMMofNN " prefix.
    let per_part = max_chars.saturating_sub(10).max(1);
    if text.len() <= max_chars {
        return vec![text.to_string()];
    }
    let chunks: Vec<&str> = text
        .as_bytes()
        .chunks(per_part)
        .map(|c| std::str::from_utf8(c).expect("callers only pass ascii"))
        .collect();
    let total = chunks.len();
    chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| format!("p{}of{total} {chunk}", i + 1))
        .collect()
}

/// Parses the `pMofN ` header, returning (index from 0, total, chunk).
fn header(s: &str) -> Option<(usize, usize, &str)> {
    let rest = s.strip_prefix('p')?;
    let (head, chunk) = rest.split_once(' ')?;
    let (m, n) = head.split_once("of")?;
    let (m, n): (usize, usize) = (m.parse().ok()?, n.parse().ok()?);
    (m >= 1 && m <= n).then_some((m - 1, n, chunk))
}

pub fn is_part(s: &str) -> bool {
    header(s).is_some()
}

#[derive(Debug, Default)]
pub struct Decoder {
    parts: Vec<Option<String>>,
}

impl Decoder {
    pub fn receive(&mut self, part: &str) -> Result<(), Error> {
        let (index, total, chunk) =
            header(part).ok_or_else(|| Error::Malformed("not a pMofN part".into()))?;
        if self.parts.len() != total {
            self.parts = vec![None; total];
        }
        self.parts[index] = Some(chunk.to_string());
        Ok(())
    }

    pub fn progress(&self) -> (usize, usize) {
        (
            self.parts.iter().filter(|p| p.is_some()).count(),
            self.parts.len(),
        )
    }

    pub fn message(&self) -> Option<String> {
        if self.parts.is_empty() || self.parts.iter().any(Option::is_none) {
            return None;
        }
        Some(self.parts.iter().flatten().map(String::as_str).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let text = "cHNidP8B".repeat(100);
        let parts = encode(&text, 120);
        assert!(parts[0].starts_with("p1of"));
        let mut decoder = Decoder::default();
        for part in parts.iter().rev() {
            assert!(decoder.message().is_none());
            decoder.receive(part).unwrap();
        }
        assert_eq!(decoder.message().unwrap(), text);
    }

    #[test]
    fn short_text_is_a_single_plain_qr() {
        assert_eq!(encode("abc", 120), vec!["abc".to_string()]);
    }
}
