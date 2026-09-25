//! How Liana drives the bridge.
//!
//! Liana starts `liana-qr <request>` with piped stdin and stdout, and waits for it to exit:
//!
//! | Request | stdin | stdout on success |
//! |---|---|---|
//! | `sign` | base64 PSBT | base64 PSBT with the device's signatures merged in |
//! | `register --name <wallet name>` | descriptor | `registered` |
//! | `xpub --network <bitcoin\|testnet\|testnet4\|signet\|regtest>` | nothing | `[fingerprint/path]xpub` |
//!
//! The answer is a single line. The bridge exits without writing anything when the user cancels
//! (closes the window, or goes back from the first step). Errors are shown to the user in the
//! bridge, which lets them retry, so Liana never has to report them.
//!
//! The Liana side lives in `liana-gui/src/qr_bridge.rs`: keep both in sync.

use std::{io::Read, str::FromStr};

use miniscript::{
    bitcoin::{NetworkKind, Psbt},
    descriptor::DescriptorPublicKey,
    Descriptor,
};

pub const REGISTERED: &str = "registered";

pub const USAGE: &str = "\
Liana QR bridge: lets Liana use airgapped signing devices through QR codes.
It is started by Liana (Settings > General > QR code signing devices), not by hand.

Usage:
  liana-qr sign                            < psbt
  liana-qr register --name <wallet name>   < descriptor
  liana-qr xpub --network <network>";

#[derive(Debug, Clone)]
pub enum Request {
    Sign(Psbt),
    Register { name: String, descriptor: String },
    Xpub(NetworkKind),
}

pub fn parse(args: &[String], mut stdin: impl Read) -> Result<Request, String> {
    let mut read_stdin = || {
        let mut input = String::new();
        stdin
            .read_to_string(&mut input)
            .map_err(|e| format!("cannot read the request: {e}"))?;
        Ok::<_, String>(input.trim().to_string())
    };
    let option = |name: &str| {
        args.windows(2)
            .find(|w| w[0] == name)
            .map(|w| w[1].clone())
            .ok_or_else(|| format!("missing {name}"))
    };
    match args.first().map(String::as_str) {
        Some("sign") => {
            let psbt = Psbt::from_str(&read_stdin()?).map_err(|e| format!("invalid PSBT: {e}"))?;
            if psbt.inputs.is_empty() {
                return Err("the PSBT has no input".into());
            }
            Ok(Request::Sign(psbt))
        }
        Some("register") => {
            let name = option("--name")?;
            // Specter separates the name from the descriptor with '&'.
            if name.trim().is_empty() || name.contains('&') {
                return Err("invalid wallet name".into());
            }
            let descriptor = read_stdin()?;
            Descriptor::<DescriptorPublicKey>::from_str(&descriptor)
                .map_err(|e| format!("invalid descriptor: {e}"))?;
            Ok(Request::Register { name, descriptor })
        }
        Some("xpub") => match option("--network")?.as_str() {
            "bitcoin" => Ok(Request::Xpub(NetworkKind::Main)),
            "testnet" | "testnet4" | "signet" | "regtest" => Ok(Request::Xpub(NetworkKind::Test)),
            n => Err(format!("unknown network {n}")),
        },
        _ => Err("unknown request".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::tests_support::TEST_PSBT;

    fn args(s: &str) -> Vec<String> {
        s.split(' ').map(String::from).collect()
    }

    #[test]
    fn requests() {
        assert!(matches!(
            parse(&args("sign"), TEST_PSBT.as_bytes()),
            Ok(Request::Sign(_))
        ));
        assert!(parse(&args("sign"), "nope".as_bytes()).is_err());
        assert!(matches!(
            parse(&args("xpub --network signet"), "".as_bytes()),
            Ok(Request::Xpub(NetworkKind::Test))
        ));
        assert!(parse(&args("xpub"), "".as_bytes()).is_err());
        let desc = "wsh(pk([f714c228/48'/1'/0'/2']tpubDEwJnTwfKoMvu8AXXBPydBVWDpzNP5tatjjZ56q4TQioGL7iL9xzTbMoCCQ3tfGihtff7vtR4xsjcRuhZ7HWARVAkGZ1HZcpBhVdou76k7j/<0;1>/*))";
        assert!(matches!(
            parse(&args("register --name Liana"), desc.as_bytes()),
            Ok(Request::Register { .. })
        ));
        assert!(parse(&args("register --name A&B"), desc.as_bytes()).is_err());
        assert!(parse(&args("register --name Liana"), "wsh(nope)".as_bytes()).is_err());
    }
}
