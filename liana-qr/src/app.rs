use std::{io::Write, time::Duration};

use iced::{widget::image, Subscription, Task};
use miniscript::bitcoin::{NetworkKind, Psbt};

use crate::{
    codec::{self, Animation, Density, ExtendedKey, Payload, Scanned, Scanner},
    device::Device,
    protocol::Request,
    psbt::merge_signatures,
    scan::{self, CameraEvent},
};

/// Time each frame of an animated QR code stays on screen.
const FRAME_DURATION: Duration = Duration::from_millis(250);
/// Largest side of the displayed QR codes, in pixels.
pub const QR_SIZE: u32 = 400;

pub struct App {
    pub device: Device,
    pub density: Density,
    pub screen: Screen,
    /// The answer handed to Liana, once there is one.
    pub answer: Option<String>,
}

// Screen states are few and short-lived: their size doesn't matter.
#[allow(clippy::large_enum_variant)]
pub enum Screen {
    /// Show the PSBT to the device.
    SignShow { psbt: Psbt, qr: AnimatedQr },
    /// Scan the device's signed answer.
    SignScan { psbt: Psbt, scan: ScanState },
    /// Show the descriptor to the device.
    Register {
        name: String,
        descriptor: String,
        qr: AnimatedQr,
    },
    /// Scan an extended public key.
    KeyScan {
        network: NetworkKind,
        scan: ScanState,
    },
    /// The device shared several keys: let the user pick one.
    KeyChoose { keys: Vec<ExtendedKey> },
}

/// The QR code currently displayed, and the animation producing the next ones.
pub struct AnimatedQr {
    animation: Animation,
    pub image: Option<image::Handle>,
    pub position: usize,
    pub error: Option<String>,
}

impl AnimatedQr {
    fn new(payload: Payload<'_>, device: Device, density: Density) -> Self {
        let mut qr = Self {
            animation: Animation::new(payload, device.transport(), density),
            image: None,
            position: 0,
            error: None,
        };
        qr.advance();
        qr
    }

    fn advance(&mut self) {
        let (position, frame) = self.animation.next_frame();
        self.position = position;
        match render_qr(&frame, QR_SIZE) {
            Ok(image) => {
                self.image = Some(image);
                self.error = None;
            }
            Err(e) => {
                self.image = None;
                self.error = Some(format!("Cannot draw the QR code ({e}). Lower the density."));
            }
        }
    }

    pub fn frame_count(&self) -> usize {
        self.animation.frame_count()
    }
}

#[derive(Default)]
pub struct ScanState {
    scanner: Scanner,
    pub preview: Option<iced::widget::image::Handle>,
    pub error: Option<String>,
    pub camera_error: Option<String>,
}

impl ScanState {
    pub fn progress(&self) -> Option<(usize, usize)> {
        self.scanner.progress()
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    /// Go back a step, or cancel from the first one.
    Previous,
    DeviceSelected(Device),
    DensitySelected(Density),
    Tick,
    StartScan,
    Camera(CameraEvent),
    LoadImage,
    ImageDecoded(Result<Vec<String>, String>),
    SelectKey(usize),
    /// The descriptor was registered on the device.
    Registered,
}

impl App {
    pub fn new(request: Request) -> (Self, Task<Message>) {
        let device = crate::device::load_last_used();
        let density = Density::default();
        let screen = match request {
            Request::Sign(psbt) => Screen::SignShow {
                qr: AnimatedQr::new(Payload::Psbt(&psbt), device, density),
                psbt,
            },
            Request::Register { name, descriptor } => Screen::Register {
                qr: AnimatedQr::new(
                    Payload::Text(&device.registration_text(&name, &descriptor)),
                    device,
                    density,
                ),
                name,
                descriptor,
            },
            Request::Xpub(network) => Screen::KeyScan {
                network,
                scan: ScanState::default(),
            },
        };
        (
            Self {
                device,
                density,
                screen,
                answer: None,
            },
            Task::none(),
        )
    }

    pub fn title(&self) -> String {
        "Liana QR bridge".to_string()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let animating = matches!(
            &self.screen,
            Screen::SignShow { qr, .. } | Screen::Register { qr, .. } if qr.frame_count() > 1
        );
        let scanning = matches!(
            &self.screen,
            Screen::SignScan { .. } | Screen::KeyScan { .. }
        );
        Subscription::batch([
            if animating {
                iced::time::every(FRAME_DURATION).map(|_| Message::Tick)
            } else {
                Subscription::none()
            },
            // Leaving the scan screen drops the subscription, which releases the camera.
            if scanning {
                Subscription::run(scan::camera).map(Message::Camera)
            } else {
                Subscription::none()
            },
        ])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Previous => return self.previous(),
            Message::DeviceSelected(device) => {
                self.device = device;
                crate::device::save_last_used(device);
                self.rebuild_qr();
            }
            Message::DensitySelected(density) => {
                self.density = density;
                self.rebuild_qr();
            }
            Message::Tick => {
                if let Screen::SignShow { qr, .. } | Screen::Register { qr, .. } = &mut self.screen
                {
                    qr.advance();
                }
            }
            Message::StartScan => {
                if let Screen::SignShow { psbt, .. } = &self.screen {
                    self.screen = Screen::SignScan {
                        psbt: psbt.clone(),
                        scan: ScanState::default(),
                    };
                }
            }
            Message::Camera(CameraEvent::Frame(handle)) => {
                if let Some(scan) = self.scan_state() {
                    scan.preview = Some(handle);
                }
            }
            Message::Camera(CameraEvent::Error(e)) => {
                if let Some(scan) = self.scan_state() {
                    scan.camera_error = Some(e);
                }
            }
            Message::Camera(CameraEvent::Decoded(content)) => return self.on_scanned(&[content]),
            Message::LoadImage => {
                return Task::perform(
                    async {
                        let file = rfd::AsyncFileDialog::new()
                            .add_filter("Image", &["png", "jpg", "jpeg"])
                            .pick_file()
                            .await
                            .ok_or_else(|| "No file selected.".to_string())?;
                        scan::decode_file(file.path())
                    },
                    Message::ImageDecoded,
                )
            }
            Message::ImageDecoded(Ok(contents)) => {
                if contents.is_empty() {
                    if let Some(scan) = self.scan_state() {
                        scan.error = Some("No QR code found in this picture.".into());
                    }
                }
                return self.on_scanned(&contents);
            }
            Message::ImageDecoded(Err(e)) => {
                if let Some(scan) = self.scan_state() {
                    scan.error = Some(e);
                }
            }
            Message::SelectKey(i) => {
                if let Screen::KeyChoose { keys } = &self.screen {
                    if let Some(key) = keys.get(i) {
                        return self.respond(key.to_liana());
                    }
                }
            }
            Message::Registered => return self.respond(crate::protocol::REGISTERED.to_string()),
        }
        Task::none()
    }

    /// Hand the result to Liana and close.
    fn respond(&mut self, answer: String) -> Task<Message> {
        // Tests check the answer rather than the process output.
        if !cfg!(test) {
            let mut stdout = std::io::stdout();
            if let Err(e) = writeln!(stdout, "{answer}").and_then(|_| stdout.flush()) {
                eprintln!("liana-qr: cannot answer Liana: {e}");
            }
        }
        self.answer = Some(answer);
        iced::exit()
    }

    fn previous(&mut self) -> Task<Message> {
        match &self.screen {
            Screen::SignScan { psbt, .. } => {
                self.screen = Screen::SignShow {
                    qr: AnimatedQr::new(Payload::Psbt(psbt), self.device, self.density),
                    psbt: psbt.clone(),
                };
                Task::none()
            }
            // First step: back to Liana, without an answer.
            _ => iced::exit(),
        }
    }

    /// Redraw the QR code after the format or density changed.
    fn rebuild_qr(&mut self) {
        match &mut self.screen {
            Screen::SignShow { psbt, qr } => {
                *qr = AnimatedQr::new(Payload::Psbt(psbt), self.device, self.density);
            }
            Screen::Register {
                name,
                descriptor,
                qr,
            } => {
                let text = self.device.registration_text(name, descriptor);
                *qr = AnimatedQr::new(Payload::Text(&text), self.device, self.density);
            }
            _ => {}
        }
    }

    fn scan_state(&mut self) -> Option<&mut ScanState> {
        match &mut self.screen {
            Screen::SignScan { scan, .. } | Screen::KeyScan { scan, .. } => Some(scan),
            _ => None,
        }
    }

    fn on_scanned(&mut self, contents: &[String]) -> Task<Message> {
        for content in contents {
            let Some(scan) = self.scan_state() else {
                break;
            };
            let scanned = match scan.scanner.receive(content) {
                Ok(Some(scanned)) => scanned,
                Ok(None) => continue,
                Err(e) => {
                    scan.error = Some(e.to_string());
                    continue;
                }
            };
            // A transfer completed: start afresh if what follows doesn't fit.
            scan.scanner = Scanner::default();
            match &mut self.screen {
                Screen::SignScan { psbt, scan } => {
                    let Scanned::Psbt(signed) = scanned else {
                        scan.error = Some(
                            "This is not a transaction. Show the signed PSBT on the device.".into(),
                        );
                        continue;
                    };
                    let mut merged = psbt.clone();
                    match merge_signatures(&mut merged, &signed) {
                        Ok(_) => return self.respond(merged.to_string()),
                        Err(e) => scan.error = Some(e.to_string()),
                    }
                }
                Screen::KeyScan { network, scan } => {
                    let keys = match scanned {
                        Scanned::Keys(keys) => Ok(keys),
                        Scanned::Text(text) => codec::parse_keys(&text).map_err(|e| e.to_string()),
                        Scanned::Psbt(_) => {
                            Err("This is a transaction, not an extended public key.".to_string())
                        }
                    };
                    match keys.and_then(|keys| usable_keys(keys, *network)) {
                        Ok(keys) if keys.len() == 1 => return self.respond(keys[0].to_liana()),
                        Ok(keys) => {
                            self.screen = Screen::KeyChoose { keys };
                            break;
                        }
                        Err(e) => scan.error = Some(e),
                    }
                }
                _ => break,
            }
        }
        Task::none()
    }
}

/// Keep the keys for the wallet's network, Liana's standard path first.
fn usable_keys(keys: Vec<ExtendedKey>, network: NetworkKind) -> Result<Vec<ExtendedKey>, String> {
    let mut keys: Vec<ExtendedKey> = keys
        .into_iter()
        .filter(|k| k.xpub.network == network)
        .collect();
    if keys.is_empty() {
        return Err("This key is for another network than the wallet.".into());
    }
    // A single key on Liana's path needs no choice: drop the other script types the device
    // advertised (single-sig, nested segwit...).
    if keys.iter().filter(|k| k.liana_account().is_some()).count() == 1 {
        keys.retain(|k| k.liana_account().is_some());
    }
    Ok(keys)
}

/// Draw a QR code as an image of at most `max_size` pixels, with the standard 4 modules quiet
/// zone. Every module gets the same whole number of pixels, and the image is shown unscaled, so
/// modules stay crisp on every renderer: this is what cameras read best.
fn render_qr(content: &str, max_size: u32) -> Result<image::Handle, qrcode::types::QrError> {
    // Frames are sized for low error correction: the screen is a clean medium.
    let code = qrcode::QrCode::with_error_correction_level(content, qrcode::EcLevel::L)?;
    const QUIET: usize = 4;
    let width = code.width();
    let modules = width + 2 * QUIET;
    let scale = (max_size as usize / modules).max(1);
    let size = modules * scale;
    let colors = code.to_colors();
    let mut rgba = vec![255u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let (mx, my) = (x / scale, y / scale);
            let dark = (QUIET..QUIET + width).contains(&mx)
                && (QUIET..QUIET + width).contains(&my)
                && colors[(my - QUIET) * width + (mx - QUIET)] == qrcode::Color::Dark;
            if dark {
                let offset = (y * size + x) * 4;
                rgba[offset..offset + 3].fill(0);
            }
        }
    }
    Ok(image::Handle::from_rgba(size as u32, size as u32, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::tests_support::TEST_PSBT;
    use std::str::FromStr;

    const XPUB: &str = "tpubDEwJnTwfKoMvu8AXXBPydBVWDpzNP5tatjjZ56q4TQioGL7iL9xzTbMoCCQ3tfGihtff7vtR4xsjcRuhZ7HWARVAkGZ1HZcpBhVdou76k7j";

    #[test]
    fn sign_stays_on_scan_until_new_signatures() {
        let psbt = Psbt::from_str(TEST_PSBT).unwrap();
        let (mut app, _) = App::new(Request::Sign(psbt.clone()));
        let _ = app.update(Message::StartScan);
        // The unsigned PSBT scanned back adds nothing: stay and explain.
        let _ = app.update(Message::Camera(CameraEvent::Decoded(psbt.to_string())));
        let Screen::SignScan { scan, .. } = &app.screen else {
            panic!("expected the scan screen");
        };
        assert!(scan.error.is_some());
        let _ = app.update(Message::Previous);
        assert!(matches!(app.screen, Screen::SignShow { .. }));
    }

    #[test]
    fn sign_answers_the_merged_psbt() {
        let psbt = Psbt::from_str(TEST_PSBT).unwrap();
        let (mut app, _) = App::new(Request::Sign(psbt.clone()));
        let _ = app.update(Message::StartScan);

        // Specter and Krux answer with the transaction and the signatures only.
        let mut trimmed = Psbt::from_unsigned_tx(psbt.unsigned_tx.clone()).unwrap();
        let secp = miniscript::bitcoin::secp256k1::Secp256k1::new();
        let sk = miniscript::bitcoin::secp256k1::SecretKey::from_slice(&[3; 32]).unwrap();
        let msg = miniscript::bitcoin::secp256k1::Message::from_digest([4; 32]);
        let pk = miniscript::bitcoin::PublicKey::new(sk.public_key(&secp));
        let sig = miniscript::bitcoin::ecdsa::Signature::sighash_all(secp.sign_ecdsa(&msg, &sk));
        trimmed.inputs[0].partial_sigs.insert(pk, sig);
        let _ = app.update(Message::Camera(CameraEvent::Decoded(trimmed.to_string())));

        // Liana gets its full PSBT back, with the new signature.
        let answer = Psbt::from_str(app.answer.as_deref().unwrap()).unwrap();
        let mut expected = psbt;
        expected.inputs[0].partial_sigs.insert(pk, sig);
        assert_eq!(answer, expected);
    }

    #[test]
    fn register_and_xpub_answers() {
        let desc = "wsh(pk([f714c228/48'/1'/0'/2']tpubDEwJnTwfKoMvu8AXXBPydBVWDpzNP5tatjjZ56q4TQioGL7iL9xzTbMoCCQ3tfGihtff7vtR4xsjcRuhZ7HWARVAkGZ1HZcpBhVdou76k7j/<0;1>/*))";
        let (mut app, _) = App::new(Request::Register {
            name: "Liana".into(),
            descriptor: desc.into(),
        });
        let _ = app.update(Message::Registered);
        assert_eq!(app.answer.as_deref(), Some(crate::protocol::REGISTERED));

        let (mut app, _) = App::new(Request::Xpub(NetworkKind::Test));
        let _ = app.update(Message::Camera(CameraEvent::Decoded(format!(
            "[f714c228/48h/1h/0h/2h]{XPUB}"
        ))));
        assert_eq!(
            app.answer.as_deref(),
            Some(format!("[f714c228/48'/1'/0'/2']{XPUB}").as_str())
        );
    }

    #[test]
    fn keys_are_filtered_by_network() {
        let tpub = codec::parse_keys(&format!("[f714c228/48h/1h/0h/2h]{XPUB}")).unwrap();
        assert_eq!(
            usable_keys(tpub.clone(), NetworkKind::Test).unwrap().len(),
            1
        );
        assert!(usable_keys(tpub, NetworkKind::Main).is_err());
    }

    #[test]
    fn key_on_liana_path_is_preferred() {
        let mut keys = codec::parse_keys(&format!("[f714c228/48h/1h/0h/2h]{XPUB}")).unwrap();
        keys.extend(codec::parse_keys(&format!("[f714c228/84h/1h/0h]{XPUB}")).unwrap());
        let usable = usable_keys(keys, NetworkKind::Test).unwrap();
        assert_eq!(usable.len(), 1);
        assert_eq!(usable[0].liana_account(), Some(0));
    }
}
