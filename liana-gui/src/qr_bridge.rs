//! Stopgap support for QR code signing devices (Specter DIY, Krux, Coldcard Q, Passport Prime),
//! through the external `liana-qr` bridge.
//!
//! Disabled by default: users turn it on in Settings > General. Liana then offers a "QR code
//! device" option to sign, register the wallet and import a key, and starts the bridge for that
//! one action. The protocol is described in `liana-qr/src/protocol.rs`.
//!
//! To remove this once QR devices are supported natively, delete this file, the `qr_bridge`
//! field of the global settings, and the call sites (`grep -rn qr_bridge`).

use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    str::FromStr,
};

use liana::miniscript::bitcoin::{bip32::Fingerprint, Network, Psbt};

use crate::{app::settings::global::GlobalSettings, dir::LianaDirectory};

const BINARY: &str = if cfg!(windows) {
    "liana-qr.exe"
} else {
    "liana-qr"
};
/// Answer of the bridge once the descriptor is registered.
const REGISTERED: &str = "registered";
/// Lets tests point `run` at a fake bridge.
#[cfg(test)]
const TEST_BRIDGE: &str = "LIANA_QR_TEST_BRIDGE";

pub fn is_enabled(liana_directory: &LianaDirectory) -> bool {
    GlobalSettings::load_qr_bridge(&GlobalSettings::path(liana_directory))
}

pub fn set_enabled(liana_directory: &LianaDirectory, enabled: bool) -> Result<(), String> {
    GlobalSettings::update_qr_bridge(&GlobalSettings::path(liana_directory), enabled)
}

/// Show the PSBT to a QR code device and return it with the device's signatures, or `None` if
/// the user cancelled.
pub async fn sign(psbt: Psbt) -> Result<Option<Psbt>, String> {
    let Some(answer) = run(&["sign"], psbt.to_string()).await? else {
        return Ok(None);
    };
    let signed =
        Psbt::from_str(&answer).map_err(|e| format!("Invalid PSBT from the bridge: {e}"))?;
    if signed.unsigned_tx.compute_txid() != psbt.unsigned_tx.compute_txid() {
        return Err("The QR code device signed another transaction.".into());
    }
    Ok(Some(signed))
}

/// Scan an extended public key, returned as `[fingerprint/path]xpub`, or `None` if the user
/// cancelled.
pub async fn scan_xpub(network: Network) -> Result<Option<String>, String> {
    // Keys only differ between mainnet and the test networks.
    let network = if network == Network::Bitcoin {
        "bitcoin"
    } else {
        "testnet"
    };
    run(&["xpub", "--network", network], String::new()).await
}

/// Show the descriptor to a QR code device. Returns whether the user confirmed the registration.
pub async fn register(name: String, descriptor: String) -> Result<bool, String> {
    // The bridge separates the name from the descriptor with '&' for Specter devices.
    let name = name.replace('&', "-");
    Ok(run(&["register", "--name", &name], descriptor)
        .await?
        .is_some_and(|answer| answer == REGISTERED))
}

/// The fingerprint of a key that signed in `signed` but not yet in `original`, to mark it as
/// signed like a USB device.
pub fn new_signer(original: &Psbt, signed: &Psbt) -> Option<Fingerprint> {
    original
        .inputs
        .iter()
        .zip(&signed.inputs)
        .find_map(|(before, after)| {
            let ecdsa = after
                .partial_sigs
                .keys()
                .filter(|pk| !before.partial_sigs.contains_key(pk))
                .find_map(|pk| before.bip32_derivation.get(&pk.inner).map(|(fg, _)| *fg));
            let taproot = || {
                after
                    .tap_script_sigs
                    .keys()
                    .filter(|key| !before.tap_script_sigs.contains_key(key))
                    .find_map(|(pk, _)| before.tap_key_origins.get(pk).map(|(_, (fg, _))| *fg))
            };
            ecdsa.or_else(taproot)
        })
}

/// Start the bridge and wait for its one-line answer. `None` means the user cancelled.
async fn run(args: &[&str], input: String) -> Result<Option<String>, String> {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    tokio::task::spawn_blocking(move || {
        let mut child = Command::new(bridge_path())
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    format!("The QR code bridge ({BINARY}) is not installed next to Liana.")
                } else {
                    format!("Cannot start the QR code bridge: {e}")
                }
            })?;
        // Closing stdin tells the bridge the whole request was sent.
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(input.as_bytes())
                .map_err(|e| format!("Cannot talk to the QR code bridge: {e}"))?;
        }
        let output = child
            .wait_with_output()
            .map_err(|e| format!("QR code bridge failure: {e}"))?;
        if !output.status.success() {
            return Err(format!("The QR code bridge failed ({}).", output.status));
        }
        let answer = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok((!answer.is_empty()).then_some(answer))
    })
    .await
    .map_err(|e| format!("QR code bridge failure: {e}"))?
}

/// The bridge ships next to the Liana executable. Fall back to the `PATH` otherwise.
fn bridge_path() -> PathBuf {
    #[cfg(test)]
    if let Ok(path) = std::env::var(TEST_BRIDGE) {
        return path.into();
    }
    std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(Path::parent)
        .map(|dir| dir.join(BINARY))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(BINARY))
}

#[cfg(test)]
mod tests {
    use super::*;
    use liana::miniscript::bitcoin::{ecdsa, secp256k1, PublicKey};

    // A PSBT spending from a Liana wallet, with the BIP32 derivations of all its keys.
    const PSBT: &str = "cHNidP8BAIkCAAAAAc0x/jtWvFugrl8zc34KVIlWCugXT6JNtgir6UqX+Vv6AQAAAAD9////AkBCDwAAAAAAIgAgtQu/fA/8rQhJ0I6wUoBDO0vNa3lgsEpEIj7rTOMnBcXuIEkBAAAAACIAIOdCiXh7yL2V/f6S6KMTOzgqKkqyIXgmFuwDnmXbIiosAAAAAAABAP04AQIAAAAAAQKYYriMs/PtSqm6LPNWWFYskTL6nWZegJdwxYcVCRn8vwEAAAAA/f///87D7dkdgMd1Laj/v6xspNRtrQXGP+8BPFMLqkeBb6MRAQAAAAD9////AuGQDgAAAAAAIlEg7DgdNxI7WybaPUZXcMCh+uN1E4X8E5DzJIlj83S+tIMQZFgBAAAAACIAIJZAn7j5iOen7xo2sKzjMc24llTZIuS+RpdwcLHtE6ufAUCksqYUJBbHB9x8eHdoRvRqiGzG4wQXpmY96vh14zAJEM2CS/oZaNVC4Wj8rY2cdjAvZj9dlVZFPbOxx9g5tFxUAUA24s2KJ7sjSHUAcUSd4yqRK/G3CZM8qhkhyHhGDSS0zZvZaIcgoqOPe23gH32wAI9Aax1gJUDv4kKOqOx64ltg9BADAAEBKxBkWAEAAAAAIgAglkCfuPmI56fvGjawrOMxzbiWVNki5L5Gl3Bwse0Tq58BBYZSIQIeYxzruE4/cvi6zbRmB1asJO0bMfUutoH0bpubw1zAZSEDLZSmORZKW/k5A+4QxJR2/H+vcV8U0WPX9SvS+MRMffNSrnNkdqkUmNf1mL657o/oxxnHkIrtdNkbge+IrGt2qRSIigBO15eaB9dj93ihNpAX9HHDuoisbJNRiAP//wCyaCIGAh5jHOu4Tj9y+LrNtGYHVqwk7Rsx9S62gfRum5vDXMBlHPcUwigwAACAAQAAgAAAAIACAACAAAAAAAAAAAAiBgIr7HqsyKEvERWQsmsv6FleMuXThpI77+TVkQ3TSOOLURz3FMIoMAAAgAEAAIAAAACAAgAAgAIAAAAAAAAAIgYDLZSmORZKW/k5A+4QxJR2/H+vcV8U0WPX9SvS+MRMffMcJSLyPDAAAIABAACAAAAAgAIAAIAAAAAAAAAAACIGA/h0pUXGHq1+kSuTYVTO8RHKfQLJlhfNtm+qdcIIr09jHCUi8jwwAACAAQAAgAAAAIACAACAAgAAAAAAAAAAIgICGAO/4xFiX/S5DXTV6uARFTcMwP1hto8BtPkdn3gIjf0c9xTCKDAAAIABAACAAAAAgAIAAIACAAAAAgAAACICAuNOSbsNRv31XkF2ygwCOuCnsJNRLhV0isJ/VRdj1k7IHPcUwigwAACAAQAAgAAAAIACAACAAAAAAAIAAAAiAgOpBJHEchNOeXuQwuLHlwOfkAyfoGvrYfb4pCFLKEPw2hwlIvI8MAAAgAEAAIAAAACAAgAAgAIAAAACAAAAIgIDyLkJiZTjLCysDOQotYs9us5CEYev4kyTYW2uL2r5H1McJSLyPDAAAIABAACAAAAAgAIAAIAAAAAAAgAAAAAiAgIlvGBvHRPmmVP6sn9g/akW2VJAvbJagMnZ/24gLdITsxz3FMIoMAAAgAEAAIAAAACAAgAAgAMAAAADAAAAIgIDNmVQOMMezQgABjk1zjfc3I2eKFJ4xLqT55jG4BP4p0Ec9xTCKDAAAIABAACAAAAAgAIAAIABAAAAAwAAACICA4Subm7T6yYCMWLgDtMy92hOgjanJefukbCOSVEHlX0IHCUi8jwwAACAAQAAgAAAAIACAACAAwAAAAMAAAAA";

    #[test]
    fn finds_the_new_signer() {
        let original = Psbt::from_str(PSBT).unwrap();
        let mut signed = original.clone();
        assert_eq!(new_signer(&original, &signed), None);

        // Sign for the first key of the input's derivations: its fingerprint is the signer.
        let (pk, (fingerprint, _)) = original.inputs[0].bip32_derivation.iter().next().unwrap();
        let secp = secp256k1::Secp256k1::new();
        let sk = secp256k1::SecretKey::from_slice(&[1; 32]).unwrap();
        let msg = secp256k1::Message::from_digest([2; 32]);
        signed.inputs[0].partial_sigs.insert(
            PublicKey::new(*pk),
            ecdsa::Signature::sighash_all(secp.sign_ecdsa(&msg, &sk)),
        );
        assert_eq!(new_signer(&original, &signed), Some(*fingerprint));
    }

    /// A fake bridge answering like the real one, to check the process plumbing.
    #[cfg(unix)]
    #[tokio::test]
    async fn talks_to_the_bridge() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("liana-qr-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("bridge");
        // Echo the PSBT back for "sign", cancel (no output) for anything else.
        std::fs::write(
            &script,
            "#!/bin/sh\nif [ \"$1\" = sign ]; then cat; else cat >/dev/null; fi\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let answer = run_with(&script, &["sign"], PSBT.to_string())
            .await
            .unwrap();
        assert_eq!(answer.as_deref(), Some(PSBT));
        let cancelled = run_with(&script, &["xpub"], String::new()).await.unwrap();
        assert_eq!(cancelled, None);
        assert!(run_with(&dir.join("missing"), &["sign"], String::new())
            .await
            .unwrap_err()
            .contains("not installed"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    async fn run_with(path: &Path, args: &[&str], input: String) -> Result<Option<String>, String> {
        std::env::set_var(TEST_BRIDGE, path);
        run(args, input).await
    }
}
