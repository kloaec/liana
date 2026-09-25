//! What each supported device expects over QR.

use crate::codec::Transport;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Device {
    #[default]
    SpecterDiy,
    Krux,
    ColdcardQ,
    PassportPrime,
    /// Any other device speaking BC-UR.
    OtherUr,
    /// Any other device speaking BBQr.
    OtherBbqr,
}

impl Device {
    pub const ALL: [Device; 6] = [
        Device::SpecterDiy,
        Device::Krux,
        Device::ColdcardQ,
        Device::PassportPrime,
        Device::OtherUr,
        Device::OtherBbqr,
    ];

    pub fn transport(self) -> Transport {
        match self {
            // Specter and Krux answer in the format they were sent: plain base64 is what both
            // handle best.
            Device::SpecterDiy => Transport::Specter,
            Device::Krux | Device::PassportPrime | Device::OtherUr => Transport::Ur,
            Device::ColdcardQ | Device::OtherBbqr => Transport::Bbqr,
        }
    }

    /// The text to show for registering the wallet descriptor.
    pub fn registration_text(self, name: &str, descriptor: &str) -> String {
        match self {
            Device::SpecterDiy => format!("addwallet {name}&{descriptor}"),
            _ => descriptor.to_string(),
        }
    }

    /// Where to go on the device, shown next to the QR codes.
    pub fn sign_hint(self) -> &'static str {
        match self {
            Device::SpecterDiy => "On the Specter, choose Scan QR code.",
            Device::Krux => "On the Krux, choose Sign > PSBT and scan.",
            Device::ColdcardQ => "On the Coldcard Q, press the QR button and scan.",
            Device::PassportPrime => {
                "In the Liana app of the Passport Prime, scan the transaction."
            }
            Device::OtherUr | Device::OtherBbqr => "Scan the transaction with your device.",
        }
    }

    pub fn register_hint(self) -> &'static str {
        match self {
            Device::SpecterDiy => "On the Specter, choose Scan QR code, then confirm the wallet.",
            Device::Krux => "On the Krux, choose Wallet > Wallet Descriptor and scan.",
            Device::ColdcardQ => {
                "On the Coldcard Q, go to Settings > Miniscript > Import and scan."
            }
            Device::PassportPrime => "In the Liana app of the Passport Prime, import the wallet.",
            Device::OtherUr | Device::OtherBbqr => "Import the wallet descriptor on your device.",
        }
    }

    pub fn key_hint(self) -> &'static str {
        match self {
            Device::SpecterDiy => {
                "On the Specter, go to Master public keys and choose the multisig (48h/.../2h) key."
            }
            Device::Krux => {
                "On the Krux, load a Miniscript wallet policy, then show the Extended Public Key \
                 as a QR code."
            }
            Device::ColdcardQ => {
                "On the Coldcard Q, go to Settings > Miniscript > Export XPUB and show it as a QR \
                 code."
            }
            Device::PassportPrime => "In the Liana app of the Passport Prime, export the key.",
            Device::OtherUr | Device::OtherBbqr => {
                "Show the multisig (m/48'/0'/account'/2') extended public key on your device."
            }
        }
    }

    /// Known limitations, shown as a warning when the device is selected.
    pub fn caveat(self) -> Option<&'static str> {
        match self {
            Device::ColdcardQ => {
                Some("Miniscript wallets need the EDGE firmware on the Coldcard Q.")
            }
            Device::PassportPrime => Some(
                "Needs the Liana app on the Passport Prime. Its QR formats are not documented \
                 yet: if it can't read the codes, try another format.",
            ),
            _ => None,
        }
    }
}

impl std::fmt::Display for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Device::SpecterDiy => "Specter DIY",
            Device::Krux => "Krux",
            Device::ColdcardQ => "Coldcard Q",
            Device::PassportPrime => "Passport Prime",
            Device::OtherUr => "Other device (UR)",
            Device::OtherBbqr => "Other device (BBQr)",
        })
    }
}

/// The device picked last time, remembered across bridge windows: Liana opens a new one for every
/// action.
pub fn load_last_used() -> Device {
    last_used_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|id| Device::ALL.into_iter().find(|d| d.id() == id.trim()))
        .unwrap_or_default()
}

pub fn save_last_used(device: Device) {
    let Some(path) = last_used_path() else {
        return;
    };
    // Best effort: forgetting the choice only costs a click next time.
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, device.id());
}

fn last_used_path() -> Option<std::path::PathBuf> {
    Some(dirs::config_dir()?.join("liana-qr").join("device"))
}

impl Device {
    /// Stable identifier, for storage.
    fn id(self) -> &'static str {
        match self {
            Device::SpecterDiy => "specter-diy",
            Device::Krux => "krux",
            Device::ColdcardQ => "coldcard-q",
            Device::PassportPrime => "passport-prime",
            Device::OtherUr => "other-ur",
            Device::OtherBbqr => "other-bbqr",
        }
    }
}
