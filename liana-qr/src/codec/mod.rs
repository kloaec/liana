//! QR transports spoken by the supported signing devices, and conversion of what they send back
//! into something Liana understands.

pub mod bbqr;
pub mod specter;
pub mod ur;

use std::str::FromStr;

use base64::Engine;
use miniscript::{
    bitcoin::{
        base58,
        bip32::{ChildNumber, DerivationPath, Fingerprint, Xpub},
        NetworkKind, Psbt,
    },
    descriptor::DescriptorPublicKey,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Malformed(String),
    Unsupported(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Malformed(e) => write!(f, "Invalid data: {e}"),
            Error::Unsupported(e) => write!(f, "Not supported: {e}"),
        }
    }
}

/// How data is split across animated QR frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// BC-UR with fountain codes (Passport, Krux, Specter).
    Ur,
    /// BBQr (Coldcard, Krux).
    Bbqr,
    /// Plain text, split in `pMofN` parts when too long (Specter, Krux).
    Specter,
}

/// How much data goes in each frame. Denser frames mean fewer of them, but need a better camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Density {
    Low,
    #[default]
    Medium,
    High,
}

impl Density {
    pub const ALL: [Density; 3] = [Density::Low, Density::Medium, Density::High];

    /// Maximum characters in one frame.
    fn chars(self) -> usize {
        match self {
            Density::Low => 200,
            Density::Medium => 400,
            Density::High => 800,
        }
    }
}

impl std::fmt::Display for Density {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Density::Low => "Low density",
            Density::Medium => "Medium density",
            Density::High => "High density",
        })
    }
}

/// What we show to a device.
#[derive(Debug, Clone, Copy)]
pub enum Payload<'a> {
    Psbt(&'a Psbt),
    /// A descriptor or other text, already formatted for the device.
    Text(&'a str),
}

/// The frames of a QR animation, cycled by the UI.
#[derive(Debug)]
pub enum Animation {
    Ur(ur::Animation),
    Frames { frames: Vec<String>, next: usize },
}

impl Animation {
    pub fn new(
        payload: Payload<'_>,
        transport: Transport,
        density: Density,
    ) -> Result<Self, Error> {
        let chars = density.chars();
        Ok(match (transport, payload) {
            (Transport::Ur, Payload::Psbt(psbt)) => Animation::Ur(ur::Animation::new(
                &psbt.serialize(),
                ur::UrType::CryptoPsbt,
                // Minimal bytewords take two characters per byte, plus the part header.
                chars / 2 - 20,
            )?),
            (Transport::Ur, Payload::Text(text)) => Animation::Ur(ur::Animation::new(
                text.as_bytes(),
                ur::UrType::Bytes,
                chars / 2 - 20,
            )?),
            (Transport::Bbqr, Payload::Psbt(psbt)) => Animation::Frames {
                frames: bbqr::encode(&psbt.serialize(), bbqr::FileType::Psbt, chars),
                next: 0,
            },
            (Transport::Bbqr, Payload::Text(text)) => Animation::Frames {
                frames: bbqr::encode(text.as_bytes(), bbqr::FileType::Unicode, chars),
                next: 0,
            },
            (Transport::Specter, Payload::Psbt(psbt)) => Animation::Frames {
                frames: specter::encode(&psbt.to_string(), chars),
                next: 0,
            },
            (Transport::Specter, Payload::Text(text)) => Animation::Frames {
                frames: specter::encode(text, chars),
                next: 0,
            },
        })
    }

    /// Number of distinct frames the device needs to see (at least).
    pub fn frame_count(&self) -> usize {
        match self {
            Animation::Ur(ur) => ur.fragment_count(),
            Animation::Frames { frames, .. } => frames.len(),
        }
    }

    /// The next frame, and its position in the cycle.
    pub fn next_frame(&mut self) -> (usize, String) {
        match self {
            Animation::Ur(ur) => {
                let count = ur.fragment_count();
                // UR keeps generating new fountain parts; report the position in the cycle.
                let part = ur.next_part();
                let seq = part
                    .split('/')
                    .nth(1)
                    .and_then(|s| s.split('-').next())
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(1);
                ((seq - 1) % count.max(1), part)
            }
            Animation::Frames { frames, next } => {
                let index = *next;
                *next = (index + 1) % frames.len();
                (index, frames[index].clone())
            }
        }
    }
}

/// What a device showed us, once fully received.
#[derive(Debug, Clone)]
pub enum Scanned {
    Psbt(Psbt),
    Text(String),
    Keys(Vec<ExtendedKey>),
}

/// Accumulates scanned frames in whichever transport the device uses, detected per frame.
#[derive(Debug, Default)]
pub struct Scanner {
    ur: ur::Decoder,
    bbqr: bbqr::Decoder,
    specter: specter::Decoder,
    active: Option<Transport>,
    last: Option<String>,
}

impl Scanner {
    /// Feed a decoded QR code. Returns the content once a transfer is complete.
    pub fn receive(&mut self, frame: &str) -> Result<Option<Scanned>, Error> {
        let frame = frame.trim();
        // Cameras decode the same frame many times in a row.
        if self.last.as_deref() == Some(frame) {
            return Ok(None);
        }
        self.last = Some(frame.to_string());

        if ur::is_part(frame) {
            self.active = Some(Transport::Ur);
            self.ur.receive(frame)?;
            return Ok(match self.ur.message()? {
                Some(ur::Decoded::Psbt(bytes)) => Some(Scanned::Psbt(parse_psbt_bytes(&bytes)?)),
                Some(ur::Decoded::Bytes(bytes)) => Some(from_bytes(bytes)?),
                Some(ur::Decoded::Keys(keys)) => Some(Scanned::Keys(keys)),
                None => None,
            });
        }
        if bbqr::is_part(frame) {
            self.active = Some(Transport::Bbqr);
            self.bbqr.receive(frame)?;
            return match self.bbqr.message()? {
                Some((bbqr::FileType::Psbt, bytes)) => {
                    Ok(Some(Scanned::Psbt(parse_psbt_bytes(&bytes)?)))
                }
                Some((bbqr::FileType::Transaction, _)) => Err(Error::Unsupported(
                    "the device sent a finalized transaction, not a signed PSBT. Look for an \
                     option to export the signed PSBT instead"
                        .into(),
                )),
                Some((_, bytes)) => Ok(Some(from_bytes(bytes)?)),
                None => Ok(None),
            };
        }
        if specter::is_part(frame) {
            self.active = Some(Transport::Specter);
            self.specter.receive(frame)?;
            return self.specter.message().map(|t| from_text(&t)).transpose();
        }
        from_text(frame).map(Some)
    }

    /// (received, total) frames of the transfer in progress. The total is unknown (0) for UR,
    /// where fountain codes make it only an estimate.
    pub fn progress(&self) -> Option<(usize, usize)> {
        match self.active? {
            Transport::Ur => Some(self.ur.progress()),
            Transport::Bbqr => Some(self.bbqr.progress()),
            Transport::Specter => Some(self.specter.progress()),
        }
    }
}

const PSBT_MAGIC: &[u8] = b"psbt\xff";

fn parse_psbt_bytes(bytes: &[u8]) -> Result<Psbt, Error> {
    Psbt::deserialize(bytes).map_err(|e| Error::Malformed(format!("PSBT: {e}")))
}

fn from_bytes(bytes: Vec<u8>) -> Result<Scanned, Error> {
    if bytes.starts_with(PSBT_MAGIC) {
        return parse_psbt_bytes(&bytes).map(Scanned::Psbt);
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| Error::Unsupported("binary data that is not a PSBT".into()))?;
    from_text(&text)
}

/// A single text QR: a base64 or hex PSBT, or anything else as text.
fn from_text(text: &str) -> Result<Scanned, Error> {
    let text = text.trim();
    if text.starts_with("cHNidP") {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(|e| Error::Malformed(format!("PSBT base64: {e}")))?;
        return parse_psbt_bytes(&bytes).map(Scanned::Psbt);
    }
    if text.len() > 10 && text[..10].eq_ignore_ascii_case("70736274ff") {
        let bytes = hex_decode(text).ok_or_else(|| Error::Malformed("PSBT hex".into()))?;
        return parse_psbt_bytes(&bytes).map(Scanned::Psbt);
    }
    Ok(Scanned::Text(text.to_string()))
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    (s.len() % 2 == 0).then_some(())?;
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// An extended public key with its origin, as shared by a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtendedKey {
    pub fingerprint: Fingerprint,
    pub path: DerivationPath,
    pub xpub: Xpub,
    /// Script type the device advertised it for (e.g. "wsh"), if any.
    pub script: String,
}

impl ExtendedKey {
    /// The key as Liana expects it: `[fingerprint/origin]xpub`.
    pub fn to_liana(&self) -> String {
        format!("[{}/{}]{}", self.fingerprint, self.path, self.xpub)
    }

    /// Account index, if the key follows the `m/48'/coin'/account'/2'` path Liana uses.
    pub fn liana_account(&self) -> Option<u32> {
        match self.path.as_ref() {
            [ChildNumber::Hardened { index: 48 }, ChildNumber::Hardened { index: coin }, ChildNumber::Hardened { index: account }, ChildNumber::Hardened { index: 2 }]
                if *coin == self.coin_type() =>
            {
                Some(*account)
            }
            _ => None,
        }
    }

    fn coin_type(&self) -> u32 {
        match self.xpub.network {
            NetworkKind::Main => 0,
            NetworkKind::Test => 1,
        }
    }

    pub fn is_mainnet(&self) -> bool {
        self.xpub.network == NetworkKind::Main
    }
}

/// Keys found in a scanned text: a `[fp/path]xpub` expression (also with SLIP-132 prefixes such as
/// Zpub), or a Coldcard JSON export.
pub fn parse_keys(text: &str) -> Result<Vec<ExtendedKey>, Error> {
    let text = text.trim();
    if text.starts_with('{') {
        return parse_coldcard_json(text);
    }
    parse_key_expression(text).map(|k| vec![k])
}

fn parse_key_expression(text: &str) -> Result<ExtendedKey, Error> {
    let Some(rest) = text.strip_prefix('[') else {
        return Err(Error::Malformed(
            "the key has no origin ([fingerprint/path]). Export it from the device's multisig \
             or miniscript menu"
                .into(),
        ));
    };
    let (origin, key) = rest
        .split_once(']')
        .ok_or_else(|| Error::Malformed("unterminated key origin".into()))?;
    let (fingerprint, path) = origin.split_once('/').unwrap_or((origin, ""));
    let fingerprint = Fingerprint::from_str(fingerprint)
        .map_err(|_| Error::Malformed("invalid fingerprint".into()))?;
    let path = DerivationPath::from_str(&format!("m/{path}"))
        .map_err(|_| Error::Malformed("invalid derivation path".into()))?;
    // Some devices append the multipath derivation used for addresses: Liana adds it itself.
    let key = key.split('/').next().unwrap_or(key);
    let xpub = parse_xpub(key)?;

    let key = ExtendedKey {
        fingerprint,
        path,
        xpub,
        script: String::new(),
    };
    // Make sure the result parses the way Liana will parse it.
    DescriptorPublicKey::from_str(&key.to_liana())
        .map_err(|e| Error::Malformed(format!("key: {e}")))?;
    Ok(key)
}

/// Parse an xpub, converting SLIP-132 variants (ypub, Zpub, vpub...) to xpub/tpub.
fn parse_xpub(s: &str) -> Result<Xpub, Error> {
    let mut data =
        base58::decode_check(s).map_err(|_| Error::Malformed("invalid extended key".into()))?;
    if data.len() != 78 {
        return Err(Error::Malformed("invalid extended key length".into()));
    }
    const MAINNET: [u8; 4] = [0x04, 0x88, 0xb2, 0x1e];
    const TESTNET: [u8; 4] = [0x04, 0x35, 0x87, 0xcf];
    let version: [u8; 4] = data[..4].try_into().expect("checked length");
    let normalized = match version {
        // xpub ypub Ypub zpub Zpub
        MAINNET
        | [0x04, 0x9d, 0x7c, 0xb2]
        | [0x02, 0x95, 0xb4, 0x3f]
        | [0x04, 0xb2, 0x47, 0x46]
        | [0x02, 0xaa, 0x7e, 0xd3] => MAINNET,
        // tpub upub Upub vpub Vpub
        TESTNET
        | [0x04, 0x4a, 0x52, 0x62]
        | [0x02, 0x42, 0x89, 0xef]
        | [0x04, 0x5f, 0x1c, 0xf6]
        | [0x02, 0x57, 0x54, 0x83] => TESTNET,
        _ => return Err(Error::Unsupported("extended key version".into())),
    };
    data[..4].copy_from_slice(&normalized);
    Xpub::decode(&data).map_err(|e| Error::Malformed(format!("extended key: {e}")))
}

/// Coldcard exports: "Export XPUB" for miniscript (`p2wsh_key_exp`), or generic JSON (`bip48_2`).
fn parse_coldcard_json(text: &str) -> Result<Vec<ExtendedKey>, Error> {
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|e| Error::Malformed(format!("JSON: {e}")))?;
    let xfp = json.get("xfp").and_then(|v| v.as_str()).unwrap_or_default();

    let mut keys = Vec::new();
    if let Some(expr) = json.get("p2wsh_key_exp").and_then(|v| v.as_str()) {
        keys.push(parse_key_expression(&expr.to_lowercase_origin())?);
    }
    for (field, script) in [("bip48_2", "wsh"), ("bip86", "tr")] {
        let Some(entry) = json.get(field) else {
            continue;
        };
        let (Some(deriv), Some(xpub)) = (
            entry.get("deriv").and_then(|v| v.as_str()),
            entry.get("xpub").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        let path = deriv.trim_start_matches('m').trim_start_matches('/');
        let mut key = parse_key_expression(&format!("[{}/{path}]{xpub}", xfp.to_lowercase()))?;
        key.script = script.into();
        if !keys.iter().any(|k| k.xpub == key.xpub) {
            keys.push(key);
        }
    }
    if keys.is_empty() {
        return Err(Error::Unsupported(
            "this JSON has no multisig or miniscript key. On the Coldcard, use Settings > \
             Miniscript > Export XPUB"
                .into(),
        ));
    }
    Ok(keys)
}

trait LowercaseOrigin {
    fn to_lowercase_origin(&self) -> String;
}

impl LowercaseOrigin for str {
    /// Coldcard writes fingerprints upper case, which rust-bitcoin rejects.
    fn to_lowercase_origin(&self) -> String {
        match self.split_once(']') {
            Some((origin, rest)) => format!("{}]{rest}", origin.to_lowercase()),
            None => self.to_string(),
        }
    }
}

#[cfg(test)]
pub mod tests_support {
    /// A signet PSBT spending from a Liana wallet, exported by lianad.
    pub const TEST_PSBT: &str = "cHNidP8BAIkCAAAAAc0x/jtWvFugrl8zc34KVIlWCugXT6JNtgir6UqX+Vv6AQAAAAD9////AkBCDwAAAAAAIgAgtQu/fA/8rQhJ0I6wUoBDO0vNa3lgsEpEIj7rTOMnBcXuIEkBAAAAACIAIOdCiXh7yL2V/f6S6KMTOzgqKkqyIXgmFuwDnmXbIiosAAAAAAABAP04AQIAAAAAAQKYYriMs/PtSqm6LPNWWFYskTL6nWZegJdwxYcVCRn8vwEAAAAA/f///87D7dkdgMd1Laj/v6xspNRtrQXGP+8BPFMLqkeBb6MRAQAAAAD9////AuGQDgAAAAAAIlEg7DgdNxI7WybaPUZXcMCh+uN1E4X8E5DzJIlj83S+tIMQZFgBAAAAACIAIJZAn7j5iOen7xo2sKzjMc24llTZIuS+RpdwcLHtE6ufAUCksqYUJBbHB9x8eHdoRvRqiGzG4wQXpmY96vh14zAJEM2CS/oZaNVC4Wj8rY2cdjAvZj9dlVZFPbOxx9g5tFxUAUA24s2KJ7sjSHUAcUSd4yqRK/G3CZM8qhkhyHhGDSS0zZvZaIcgoqOPe23gH32wAI9Aax1gJUDv4kKOqOx64ltg9BADAAEBKxBkWAEAAAAAIgAglkCfuPmI56fvGjawrOMxzbiWVNki5L5Gl3Bwse0Tq58BBYZSIQIeYxzruE4/cvi6zbRmB1asJO0bMfUutoH0bpubw1zAZSEDLZSmORZKW/k5A+4QxJR2/H+vcV8U0WPX9SvS+MRMffNSrnNkdqkUmNf1mL657o/oxxnHkIrtdNkbge+IrGt2qRSIigBO15eaB9dj93ihNpAX9HHDuoisbJNRiAP//wCyaCIGAh5jHOu4Tj9y+LrNtGYHVqwk7Rsx9S62gfRum5vDXMBlHPcUwigwAACAAQAAgAAAAIACAACAAAAAAAAAAAAiBgIr7HqsyKEvERWQsmsv6FleMuXThpI77+TVkQ3TSOOLURz3FMIoMAAAgAEAAIAAAACAAgAAgAIAAAAAAAAAIgYDLZSmORZKW/k5A+4QxJR2/H+vcV8U0WPX9SvS+MRMffMcJSLyPDAAAIABAACAAAAAgAIAAIAAAAAAAAAAACIGA/h0pUXGHq1+kSuTYVTO8RHKfQLJlhfNtm+qdcIIr09jHCUi8jwwAACAAQAAgAAAAIACAACAAgAAAAAAAAAAIgICGAO/4xFiX/S5DXTV6uARFTcMwP1hto8BtPkdn3gIjf0c9xTCKDAAAIABAACAAAAAgAIAAIACAAAAAgAAACICAuNOSbsNRv31XkF2ygwCOuCnsJNRLhV0isJ/VRdj1k7IHPcUwigwAACAAQAAgAAAAIACAACAAAAAAAIAAAAiAgOpBJHEchNOeXuQwuLHlwOfkAyfoGvrYfb4pCFLKEPw2hwlIvI8MAAAgAEAAIAAAACAAgAAgAIAAAACAAAAIgIDyLkJiZTjLCysDOQotYs9us5CEYev4kyTYW2uL2r5H1McJSLyPDAAAIABAACAAAAAgAIAAIAAAAAAAgAAAAAiAgIlvGBvHRPmmVP6sn9g/akW2VJAvbJagMnZ/24gLdITsxz3FMIoMAAAgAEAAIAAAACAAgAAgAMAAAADAAAAIgIDNmVQOMMezQgABjk1zjfc3I2eKFJ4xLqT55jG4BP4p0Ec9xTCKDAAAIABAACAAAAAgAIAAIABAAAAAwAAACICA4Subm7T6yYCMWLgDtMy92hOgjanJefukbCOSVEHlX0IHCUi8jwwAACAAQAAgAAAAIACAACAAwAAAAMAAAAA";
}

#[cfg(test)]
mod tests {
    use super::tests_support::TEST_PSBT;
    use super::*;

    const XPUB: &str = "tpubDEwJnTwfKoMvu8AXXBPydBVWDpzNP5tatjjZ56q4TQioGL7iL9xzTbMoCCQ3tfGihtff7vtR4xsjcRuhZ7HWARVAkGZ1HZcpBhVdou76k7j";

    #[test]
    fn key_expressions() {
        for text in [
            format!("[f714c228/48'/1'/0'/2']{XPUB}"),
            format!("[f714c228/48h/1h/0h/2h]{XPUB}"),
            format!("[f714c228/48h/1h/0h/2h]{XPUB}/<0;1>/*"),
        ] {
            let keys = parse_keys(&text).unwrap();
            assert_eq!(keys[0].to_liana(), format!("[f714c228/48'/1'/0'/2']{XPUB}"));
            assert_eq!(keys[0].liana_account(), Some(0));
        }
        assert!(parse_keys(XPUB).is_err());
    }

    #[test]
    fn slip132_is_normalized() {
        // Same key as XPUB, with the Vpub version bytes.
        let mut data = base58::decode_check(XPUB).unwrap();
        data[..4].copy_from_slice(&[0x02, 0x57, 0x54, 0x83]);
        let vpub = base58::encode_check(&data);
        let keys = parse_keys(&format!("[f714c228/48h/1h/0h/2h]{vpub}")).unwrap();
        assert_eq!(keys[0].xpub.to_string(), XPUB);
    }

    #[test]
    fn coldcard_json() {
        let json =
            format!(r#"{{"xfp": "F714C228", "p2wsh_key_exp": "[F714C228/48h/1h/0h/2h]{XPUB}"}}"#);
        let keys = parse_keys(&json).unwrap();
        assert_eq!(keys[0].to_liana(), format!("[f714c228/48'/1'/0'/2']{XPUB}"));

        let json = format!(
            r#"{{"xfp": "F714C228", "bip48_2": {{"deriv": "m/48'/1'/0'/2'", "xpub": "{XPUB}"}}}}"#
        );
        assert_eq!(parse_keys(&json).unwrap()[0].liana_account(), Some(0));
    }

    #[test]
    fn scanner_detects_transports() {
        let psbt = Psbt::from_str(TEST_PSBT).unwrap();
        for transport in [Transport::Ur, Transport::Bbqr, Transport::Specter] {
            let mut animation =
                Animation::new(Payload::Psbt(&psbt), transport, Density::Low).unwrap();
            assert!(animation.frame_count() > 1, "{transport:?}");
            let mut scanner = Scanner::default();
            let mut result = None;
            for _ in 0..animation.frame_count() * 3 {
                if let Some(done) = scanner.receive(&animation.next_frame().1).unwrap() {
                    result = Some(done);
                    break;
                }
            }
            match result {
                Some(Scanned::Psbt(scanned)) => assert_eq!(scanned, psbt, "{transport:?}"),
                r => panic!("{transport:?}: {r:?}"),
            }
        }
    }

    #[test]
    fn single_frame_psbt() {
        let mut scanner = Scanner::default();
        assert!(matches!(
            scanner.receive(TEST_PSBT).unwrap(),
            Some(Scanned::Psbt(_))
        ));
    }
}
