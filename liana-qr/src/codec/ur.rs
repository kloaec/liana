//! Blockchain Commons Uniform Resources (BCR-2020-005), with fountain-coded animation.
//!
//! We only map the registry types the supported devices exchange with a wallet: PSBTs, raw bytes
//! (descriptors), and the extended keys shared when creating a wallet.

use ciborium::value::Value;
use miniscript::bitcoin::{
    bip32::{ChainCode, ChildNumber, DerivationPath, Fingerprint, Xpub},
    secp256k1, NetworkKind,
};

use super::{Error, ExtendedKey};

/// UR types we emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrType {
    /// `crypto-psbt`: devices still expect this legacy name rather than the newer `psbt`, which
    /// we only decode.
    CryptoPsbt,
    /// `bytes`, used for descriptors and other text.
    Bytes,
}

impl UrType {
    pub fn name(self) -> &'static str {
        match self {
            UrType::CryptoPsbt => "crypto-psbt",
            UrType::Bytes => "bytes",
        }
    }
}

/// An endless stream of UR parts: the fragments first, then fountain-mixed parts so a scanner
/// that missed some frames still completes.
pub struct Animation {
    encoder: ur::Encoder<'static>,
    single: Option<String>,
}

impl std::fmt::Debug for Animation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Animation")
            .field("fragments", &self.encoder.fragment_count())
            .finish()
    }
}

impl Animation {
    pub fn new(data: &[u8], ur_type: UrType, max_fragment_len: usize) -> Self {
        let cbor = cbor_bytes(data);
        // Only an invalid type, an empty message or a null fragment length fail: our types are
        // valid, and CBOR is never empty.
        let encoder = ur::Encoder::new(&cbor, max_fragment_len.max(1), ur_type.name())
            .expect("valid UR type and non-empty message");
        // A payload fitting one fragment is sent as a single-part UR, which every decoder accepts.
        let single = (encoder.fragment_count() == 1)
            .then(|| ur::encode(&cbor, &ur::Type::Custom(ur_type.name())).to_uppercase());
        Self { encoder, single }
    }

    pub fn fragment_count(&self) -> usize {
        self.encoder.fragment_count()
    }

    pub fn next_part(&mut self) -> String {
        if let Some(single) = &self.single {
            return single.clone();
        }
        // Upper case lets the QR code use the denser alphanumeric mode.
        self.encoder
            .next_part()
            .expect("the type was validated at creation")
            .to_uppercase()
    }
}

pub fn is_part(s: &str) -> bool {
    s.len() > 3 && s[..3].eq_ignore_ascii_case("ur:")
}

/// What a completed UR transfer contained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decoded {
    Psbt(Vec<u8>),
    Bytes(Vec<u8>),
    Keys(Vec<ExtendedKey>),
}

#[derive(Default)]
pub struct Decoder {
    inner: ur::Decoder,
    done: Option<(String, Vec<u8>)>,
}

impl std::fmt::Debug for Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoder")
            .field("done", &self.done.is_some())
            .finish()
    }
}

impl Decoder {
    pub fn receive(&mut self, part: &str) -> Result<(), Error> {
        let part = part.to_lowercase();
        let ur_type = part[3..].split('/').next().unwrap_or_default().to_string();
        // Single-part URs have no sequence component: `ur:type/body`.
        if part.matches('/').count() == 1 {
            let (_, payload) =
                ur::decode(&part).map_err(|e| Error::Malformed(format!("UR: {e}")))?;
            self.done = Some((ur_type, payload));
            return Ok(());
        }
        if self.inner.ur_type().is_some_and(|t| t != ur_type) {
            self.inner = ur::Decoder::default();
        }
        self.inner
            .receive(&part)
            .map_err(|e| Error::Malformed(format!("UR: {e}")))?;
        if let Some(payload) = self
            .inner
            .message()
            .map_err(|e| Error::Malformed(format!("UR: {e}")))?
        {
            self.done = Some((ur_type, payload));
        }
        Ok(())
    }

    /// Percentage-like progress: (resolved fragments, total fragments).
    pub fn progress(&self) -> (usize, usize) {
        if self.done.is_some() {
            return (1, 1);
        }
        // The ur crate only exposes the resolved count; the total is in the part header, which we
        // don't track, so report what we have.
        (self.inner.resolved_fragment_count().unwrap_or(0), 0)
    }

    pub fn message(&self) -> Result<Option<Decoded>, Error> {
        let Some((ur_type, payload)) = &self.done else {
            return Ok(None);
        };
        let value: Value = ciborium::de::from_reader(payload.as_slice())
            .map_err(|e| Error::Malformed(format!("UR CBOR: {e}")))?;
        let decoded = match ur_type.as_str() {
            "crypto-psbt" | "psbt" => Decoded::Psbt(as_bytes(&value)?),
            "bytes" => Decoded::Bytes(as_bytes(&value)?),
            "crypto-account" | "account-descriptor" => Decoded::Keys(parse_account(&value)?),
            "crypto-hdkey" | "hdkey" => Decoded::Keys(vec![parse_hdkey(&value, None)?]),
            "crypto-output" | "output-descriptor" => {
                Decoded::Keys(vec![parse_output(&value, None)?])
            }
            t => return Err(Error::Unsupported(format!("UR type '{t}'"))),
        };
        Ok(Some(decoded))
    }
}

fn cbor_bytes(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 9);
    ciborium::ser::into_writer(&Value::Bytes(data.to_vec()), &mut out).expect("in-memory write");
    out
}

fn as_bytes(value: &Value) -> Result<Vec<u8>, Error> {
    match untag(value) {
        Value::Bytes(b) => Ok(b.clone()),
        _ => Err(Error::Malformed("expected a CBOR byte string".into())),
    }
}

fn untag(value: &Value) -> &Value {
    match value {
        Value::Tag(_, inner) => untag(inner),
        v => v,
    }
}

fn map_get(value: &Value, key: u64) -> Option<&Value> {
    match untag(value) {
        Value::Map(entries) => entries.iter().find_map(|(k, v)| {
            k.as_integer()
                .and_then(|i| u64::try_from(i).ok())
                .filter(|i| *i == key)
                .map(|_| v)
        }),
        _ => None,
    }
}

fn as_u32(value: &Value) -> Option<u32> {
    untag(value)
        .as_integer()
        .and_then(|i| u32::try_from(i).ok())
}

/// `crypto-account`: a master fingerprint and a list of output descriptors, one per script type.
fn parse_account(value: &Value) -> Result<Vec<ExtendedKey>, Error> {
    let master = map_get(value, 1)
        .and_then(as_u32)
        .map(|fp| Fingerprint::from(fp.to_be_bytes()));
    let outputs = match map_get(value, 2).map(untag) {
        Some(Value::Array(outputs)) => outputs,
        _ => {
            return Err(Error::Malformed(
                "account without output descriptors".into(),
            ))
        }
    };
    let keys: Vec<ExtendedKey> = outputs
        .iter()
        .filter_map(|output| parse_output(output, master).ok())
        .collect();
    if keys.is_empty() {
        return Err(Error::Malformed("account without usable keys".into()));
    }
    Ok(keys)
}

/// `crypto-output`: script expression tags (sh, wsh, wpkh, tr...) wrapping a single key.
fn parse_output(value: &Value, master: Option<Fingerprint>) -> Result<ExtendedKey, Error> {
    let mut script = Vec::new();
    let mut current = value;
    while let Value::Tag(tag, inner) = current {
        match tag {
            400 => script.push("sh"),
            401 => script.push("wsh"),
            403 => script.push("pkh"),
            404 => script.push("wpkh"),
            409 => script.push("tr"),
            // Multisig expressions carry several keys: not an account key.
            406 | 407 => return Err(Error::Unsupported("multisig output descriptor".into())),
            303 | 40303 => {
                let mut key = parse_hdkey(current, master)?;
                key.script = script.join("-");
                return Ok(key);
            }
            _ => {}
        }
        current = inner;
    }
    // Untagged map: assume it is directly a hdkey.
    parse_hdkey(current, master)
}

/// `crypto-hdkey` (BCR-2020-007).
fn parse_hdkey(value: &Value, master: Option<Fingerprint>) -> Result<ExtendedKey, Error> {
    let key_data = map_get(value, 3)
        .map(as_bytes)
        .transpose()?
        .ok_or_else(|| Error::Malformed("hdkey without key data".into()))?;
    let chain_code = map_get(value, 4)
        .map(as_bytes)
        .transpose()?
        .ok_or_else(|| Error::Malformed("hdkey without chain code".into()))?;
    let public_key = secp256k1::PublicKey::from_slice(&key_data)
        .map_err(|_| Error::Malformed("hdkey is not a compressed public key".into()))?;
    let chain_code: [u8; 32] = chain_code
        .try_into()
        .map_err(|_| Error::Malformed("invalid chain code length".into()))?;

    // use-info: {2: network}, 0 is mainnet.
    let network = match map_get(value, 5)
        .and_then(|info| map_get(info, 2))
        .and_then(as_u32)
    {
        Some(1) => NetworkKind::Test,
        _ => NetworkKind::Main,
    };

    let (path, origin_fingerprint, depth) = match map_get(value, 6) {
        Some(origin) => parse_keypath(origin)?,
        None => (DerivationPath::master(), None, None),
    };
    let fingerprint = origin_fingerprint
        .or(master)
        .ok_or_else(|| Error::Malformed("key without master fingerprint".into()))?;
    let parent_fingerprint = map_get(value, 8)
        .and_then(as_u32)
        .map(|fp| Fingerprint::from(fp.to_be_bytes()))
        .unwrap_or_default();
    let child_number = path
        .as_ref()
        .last()
        .copied()
        .unwrap_or(ChildNumber::Normal { index: 0 });

    let xpub = Xpub {
        network,
        depth: depth.unwrap_or(path.len() as u8),
        parent_fingerprint,
        child_number,
        public_key,
        chain_code: ChainCode::from(chain_code),
    };
    Ok(ExtendedKey {
        fingerprint,
        path,
        xpub,
        script: String::new(),
    })
}

/// `crypto-keypath`: {1: [index, hardened, ...], 2: source fingerprint, 3: depth}.
fn parse_keypath(
    value: &Value,
) -> Result<(DerivationPath, Option<Fingerprint>, Option<u8>), Error> {
    let components = match map_get(value, 1).map(untag) {
        Some(Value::Array(c)) => c,
        _ => return Err(Error::Malformed("keypath without components".into())),
    };
    let mut path = Vec::new();
    for pair in components.chunks(2) {
        let [index, hardened] = pair else {
            return Err(Error::Malformed("odd keypath components".into()));
        };
        let index = as_u32(index)
            .ok_or_else(|| Error::Unsupported("wildcard or range in key origin".into()))?;
        let hardened = untag(hardened).as_bool().unwrap_or(false);
        let child = if hardened {
            ChildNumber::from_hardened_idx(index)
        } else {
            ChildNumber::from_normal_idx(index)
        }
        .map_err(|_| Error::Malformed("invalid child index".into()))?;
        path.push(child);
    }
    let fingerprint = map_get(value, 2)
        .and_then(as_u32)
        .map(|fp| Fingerprint::from(fp.to_be_bytes()));
    let depth = map_get(value, 3).and_then(as_u32).map(|d| d as u8);
    Ok((DerivationPath::from(path), fingerprint, depth))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psbt_roundtrip_multipart() {
        let data: Vec<u8> = (0..3000u32).map(|i| (i % 256) as u8).collect();
        let mut animation = Animation::new(&data, UrType::CryptoPsbt, 200);
        assert!(animation.fragment_count() > 1);
        let mut decoder = Decoder::default();
        // Skip a few frames: the fountain parts must fill the gaps.
        for i in 0..animation.fragment_count() * 4 {
            let part = animation.next_part();
            assert!(part.starts_with("UR:CRYPTO-PSBT/"));
            if i % 3 != 1 {
                decoder.receive(&part).unwrap();
            }
            if decoder.message().unwrap().is_some() {
                break;
            }
        }
        assert_eq!(decoder.message().unwrap(), Some(Decoded::Psbt(data)));
    }

    #[test]
    fn single_part_bytes() {
        let mut animation = Animation::new(b"wsh(pk(A))", UrType::Bytes, 200);
        let part = animation.next_part();
        assert_eq!(part.matches('/').count(), 1);
        let mut decoder = Decoder::default();
        decoder.receive(&part).unwrap();
        assert_eq!(
            decoder.message().unwrap(),
            Some(Decoded::Bytes(b"wsh(pk(A))".to_vec()))
        );
    }

    /// A `crypto-account` as Jade or Passport export it: a wsh output wrapping a hdkey.
    #[test]
    fn crypto_account() {
        use miniscript::bitcoin::bip32::Xpub;
        use std::str::FromStr;
        let xpub = Xpub::from_str("tpubDEwJnTwfKoMvu8AXXBPydBVWDpzNP5tatjjZ56q4TQioGL7iL9xzTbMoCCQ3tfGihtff7vtR4xsjcRuhZ7HWARVAkGZ1HZcpBhVdou76k7j").unwrap();
        let int = |i: u64| Value::Integer(i.into());
        let keypath = Value::Tag(
            304,
            Box::new(Value::Map(vec![
                (
                    int(1),
                    Value::Array(vec![
                        int(48),
                        Value::Bool(true),
                        int(1),
                        Value::Bool(true),
                        int(0),
                        Value::Bool(true),
                        int(2),
                        Value::Bool(true),
                    ]),
                ),
                (int(2), int(0xf714c228)),
                (int(3), int(4)),
            ])),
        );
        let hdkey = Value::Tag(
            303,
            Box::new(Value::Map(vec![
                (int(3), Value::Bytes(xpub.public_key.serialize().to_vec())),
                (int(4), Value::Bytes(xpub.chain_code.to_bytes().to_vec())),
                (
                    int(5),
                    Value::Tag(305, Box::new(Value::Map(vec![(int(2), int(1))]))),
                ),
                (int(6), keypath),
                (
                    int(8),
                    int(u32::from_be_bytes(xpub.parent_fingerprint.to_bytes()) as u64),
                ),
            ])),
        );
        let account = Value::Map(vec![
            (int(1), int(0xf714c228)),
            (int(2), Value::Array(vec![Value::Tag(401, Box::new(hdkey))])),
        ]);
        let mut cbor = Vec::new();
        ciborium::ser::into_writer(&account, &mut cbor).unwrap();
        let part = ur::encode(&cbor, &ur::Type::Custom("crypto-account"));

        let mut decoder = Decoder::default();
        decoder.receive(&part).unwrap();
        let Some(Decoded::Keys(keys)) = decoder.message().unwrap() else {
            panic!("expected keys");
        };
        assert_eq!(keys[0].script, "wsh");
        assert_eq!(keys[0].to_liana(), format!("[f714c228/48'/1'/0'/2']{xpub}"));
    }
}
