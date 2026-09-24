use std::{path::PathBuf, str::FromStr, time::Duration};

use iced::{widget::image, Subscription, Task};
use liana_ui::component::form;
use miniscript::{bitcoin::Psbt, descriptor::DescriptorPublicKey, Descriptor};

use crate::{
    codec::{self, Animation, Density, ExtendedKey, Payload, Scanned, Scanner},
    device::Device,
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
}

// Screen states are few and short-lived: their size doesn't matter.
#[allow(clippy::large_enum_variant)]
pub enum Screen {
    Home,
    Sign(Sign),
    Register(Register),
    Key(Key),
}

#[allow(clippy::large_enum_variant)]
pub enum Sign {
    Load {
        error: Option<String>,
    },
    Show {
        psbt: Psbt,
        qr: AnimatedQr,
    },
    Scan {
        psbt: Psbt,
        scan: ScanState,
    },
    Done {
        psbt: Psbt,
        added: usize,
        saved: Option<PathBuf>,
        error: Option<String>,
    },
}

#[allow(clippy::large_enum_variant)]
pub enum Register {
    Edit {
        name: form::Value<String>,
        descriptor: form::Value<String>,
    },
    Show {
        name: String,
        descriptor: String,
        qr: AnimatedQr,
    },
}

#[allow(clippy::large_enum_variant)]
pub enum Key {
    Scan(ScanState),
    Done {
        keys: Vec<ExtendedKey>,
        copied: Option<usize>,
    },
}

/// The QR code currently displayed, and the animation producing the next ones.
pub struct AnimatedQr {
    animation: Animation,
    pub image: Option<image::Handle>,
    pub position: usize,
    pub error: Option<String>,
}

impl AnimatedQr {
    fn new(payload: Payload<'_>, device: Device, density: Density) -> Result<Self, String> {
        let animation =
            Animation::new(payload, device.transport(), density).map_err(|e| e.to_string())?;
        let mut qr = Self {
            animation,
            image: None,
            position: 0,
            error: None,
        };
        qr.advance();
        Ok(qr)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Sign,
    Register,
    Key,
}

#[derive(Debug, Clone)]
pub enum Message {
    Open(Flow),
    Previous,
    DeviceSelected(Device),
    DensitySelected(Density),
    Tick,
    // Inputs.
    LoadPsbtFile,
    LoadDescriptorFile,
    PsbtLoaded(Result<String, String>),
    DescriptorLoaded(Result<String, String>),
    PastePsbt,
    NameEdited(String),
    DescriptorEdited(String),
    ShowRegistration,
    // Scanning.
    StartScan,
    Camera(CameraEvent),
    LoadImage,
    ImageDecoded(Result<Vec<String>, String>),
    // Outputs.
    SavePsbt,
    PsbtSaved(Result<Option<PathBuf>, String>),
    CopyPsbt,
    CopyKey(usize),
    SignWithAnotherDevice,
    Finish,
}

impl App {
    /// Start on the home screen, or directly on signing when given a PSBT file.
    pub fn new(psbt_path: Option<PathBuf>) -> (Self, Task<Message>) {
        let mut app = Self {
            device: Device::default(),
            density: Density::default(),
            screen: Screen::Home,
        };
        let task = match psbt_path {
            Some(path) => {
                app.screen = Screen::Sign(Sign::Load { error: None });
                app.update(Message::PsbtLoaded(read_text_file(&path)))
            }
            None => Task::none(),
        };
        (app, task)
    }

    pub fn title(&self) -> String {
        "Liana QR bridge".to_string()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let animating = matches!(
            &self.screen,
            Screen::Sign(Sign::Show { qr, .. }) | Screen::Register(Register::Show { qr, .. })
                if qr.frame_count() > 1
        );
        let scanning = matches!(
            &self.screen,
            Screen::Sign(Sign::Scan { .. }) | Screen::Key(Key::Scan(_))
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
            Message::Open(Flow::Sign) => self.screen = Screen::Sign(Sign::Load { error: None }),
            Message::Open(Flow::Register) => {
                self.screen = Screen::Register(Register::Edit {
                    name: form::Value {
                        value: "Liana".to_string(),
                        ..Default::default()
                    },
                    descriptor: form::Value::default(),
                })
            }
            Message::Open(Flow::Key) => self.screen = Screen::Key(Key::Scan(ScanState::default())),
            Message::Previous => self.previous(),
            Message::DeviceSelected(device) => {
                self.device = device;
                self.rebuild_qr();
            }
            Message::DensitySelected(density) => {
                self.density = density;
                self.rebuild_qr();
            }
            Message::Tick => {
                if let Screen::Sign(Sign::Show { qr, .. })
                | Screen::Register(Register::Show { qr, .. }) = &mut self.screen
                {
                    qr.advance();
                }
            }
            Message::LoadPsbtFile => {
                return Task::perform(
                    pick_text_file("PSBT", &["psbt", "txt"]),
                    Message::PsbtLoaded,
                )
            }
            Message::PastePsbt => {
                return iced::clipboard::read().map(|text| {
                    Message::PsbtLoaded(text.ok_or_else(|| "The clipboard is empty.".to_string()))
                })
            }
            Message::PsbtLoaded(result) => {
                if let Screen::Sign(sign) = &mut self.screen {
                    match result.and_then(|text| parse_psbt(&text)) {
                        Ok(psbt) => {
                            match AnimatedQr::new(Payload::Psbt(&psbt), self.device, self.density) {
                                Ok(qr) => *sign = Sign::Show { psbt, qr },
                                Err(e) => *sign = Sign::Load { error: Some(e) },
                            }
                        }
                        Err(e) => *sign = Sign::Load { error: Some(e) },
                    }
                }
            }
            Message::LoadDescriptorFile => {
                return Task::perform(
                    pick_text_file("Descriptor", &["txt", "descriptor"]),
                    Message::DescriptorLoaded,
                )
            }
            Message::DescriptorLoaded(Ok(text)) => {
                return self.update(Message::DescriptorEdited(text.trim().to_string()))
            }
            Message::DescriptorLoaded(Err(e)) => {
                if let Screen::Register(Register::Edit { descriptor, .. }) = &mut self.screen {
                    descriptor.warning = Some(e);
                    descriptor.valid = false;
                }
            }
            Message::NameEdited(value) => {
                if let Screen::Register(Register::Edit { name, .. }) = &mut self.screen {
                    // Specter separates the name from the descriptor with '&'.
                    name.valid = !value.trim().is_empty() && !value.contains('&');
                    name.value = value;
                }
            }
            Message::DescriptorEdited(value) => {
                if let Screen::Register(Register::Edit { descriptor, .. }) = &mut self.screen {
                    let value = value.trim().to_string();
                    descriptor.valid = value.is_empty()
                        || Descriptor::<DescriptorPublicKey>::from_str(&value).is_ok();
                    descriptor.warning = None;
                    descriptor.value = value;
                }
            }
            Message::ShowRegistration => {
                if let Screen::Register(Register::Edit { name, descriptor }) = &mut self.screen {
                    let text = self
                        .device
                        .registration_text(name.value.trim(), &descriptor.value);
                    match AnimatedQr::new(Payload::Text(&text), self.device, self.density) {
                        Ok(qr) => {
                            self.screen = Screen::Register(Register::Show {
                                name: name.value.trim().to_string(),
                                descriptor: descriptor.value.clone(),
                                qr,
                            })
                        }
                        Err(e) => {
                            descriptor.warning = Some(e);
                            descriptor.valid = false;
                        }
                    }
                }
            }
            Message::StartScan => {
                if let Screen::Sign(sign @ Sign::Show { .. }) = &mut self.screen {
                    let Sign::Show { psbt, .. } =
                        std::mem::replace(sign, Sign::Load { error: None })
                    else {
                        unreachable!()
                    };
                    *sign = Sign::Scan {
                        psbt,
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
            Message::Camera(CameraEvent::Decoded(content)) => self.on_scanned(&[content]),
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
                self.on_scanned(&contents);
            }
            Message::ImageDecoded(Err(e)) => {
                if let Some(scan) = self.scan_state() {
                    scan.error = Some(e);
                }
            }
            Message::SavePsbt => {
                if let Screen::Sign(Sign::Done { psbt, .. }) = &self.screen {
                    let text = psbt.to_string();
                    return Task::perform(
                        async move {
                            let Some(file) = rfd::AsyncFileDialog::new()
                                .set_file_name("signed.psbt")
                                .save_file()
                                .await
                            else {
                                return Ok(None);
                            };
                            std::fs::write(file.path(), text)
                                .map(|_| Some(file.path().to_path_buf()))
                                .map_err(|e| e.to_string())
                        },
                        Message::PsbtSaved,
                    );
                }
            }
            Message::PsbtSaved(result) => {
                if let Screen::Sign(Sign::Done { saved, error, .. }) = &mut self.screen {
                    match result {
                        Ok(path) => {
                            *saved = path.or(saved.take());
                            *error = None;
                        }
                        Err(e) => *error = Some(format!("Cannot save the file: {e}")),
                    }
                }
            }
            Message::CopyPsbt => {
                if let Screen::Sign(Sign::Done { psbt, .. }) = &self.screen {
                    return iced::clipboard::write(psbt.to_string());
                }
            }
            Message::CopyKey(i) => {
                if let Screen::Key(Key::Done { keys, copied }) = &mut self.screen {
                    if let Some(key) = keys.get(i) {
                        *copied = Some(i);
                        return iced::clipboard::write(key.to_liana());
                    }
                }
            }
            Message::SignWithAnotherDevice => {
                if let Screen::Sign(sign @ Sign::Done { .. }) = &mut self.screen {
                    let Sign::Done { psbt, .. } =
                        std::mem::replace(sign, Sign::Load { error: None })
                    else {
                        unreachable!()
                    };
                    *sign = match AnimatedQr::new(Payload::Psbt(&psbt), self.device, self.density) {
                        Ok(qr) => Sign::Show { psbt, qr },
                        Err(e) => Sign::Load { error: Some(e) },
                    };
                }
            }
            Message::Finish => self.screen = Screen::Home,
        }
        Task::none()
    }

    fn previous(&mut self) {
        self.screen = match std::mem::replace(&mut self.screen, Screen::Home) {
            Screen::Sign(Sign::Scan { psbt, .. }) => {
                match AnimatedQr::new(Payload::Psbt(&psbt), self.device, self.density) {
                    Ok(qr) => Screen::Sign(Sign::Show { psbt, qr }),
                    Err(e) => Screen::Sign(Sign::Load { error: Some(e) }),
                }
            }
            Screen::Sign(Sign::Show { .. }) => Screen::Sign(Sign::Load { error: None }),
            Screen::Register(Register::Show {
                name, descriptor, ..
            }) => Screen::Register(Register::Edit {
                name: form::Value {
                    value: name,
                    ..Default::default()
                },
                descriptor: form::Value {
                    value: descriptor,
                    ..Default::default()
                },
            }),
            Screen::Key(Key::Done { .. }) => Screen::Key(Key::Scan(ScanState::default())),
            _ => Screen::Home,
        };
    }

    /// Redraw the QR code after the format or density changed.
    fn rebuild_qr(&mut self) {
        match &mut self.screen {
            Screen::Sign(Sign::Show { psbt, qr }) => {
                match AnimatedQr::new(Payload::Psbt(psbt), self.device, self.density) {
                    Ok(new) => *qr = new,
                    Err(e) => qr.error = Some(e),
                }
            }
            Screen::Register(Register::Show {
                name,
                descriptor,
                qr,
            }) => {
                let text = self.device.registration_text(name, descriptor);
                match AnimatedQr::new(Payload::Text(&text), self.device, self.density) {
                    Ok(new) => *qr = new,
                    Err(e) => qr.error = Some(e),
                }
            }
            _ => {}
        }
    }

    fn scan_state(&mut self) -> Option<&mut ScanState> {
        match &mut self.screen {
            Screen::Sign(Sign::Scan { scan, .. }) | Screen::Key(Key::Scan(scan)) => Some(scan),
            _ => None,
        }
    }

    fn on_scanned(&mut self, contents: &[String]) {
        for content in contents {
            let Some(scan) = self.scan_state() else {
                return;
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
                Screen::Sign(sign @ Sign::Scan { .. }) => {
                    let Sign::Scan { psbt, scan } = sign else {
                        unreachable!()
                    };
                    let Scanned::Psbt(signed) = scanned else {
                        scan.error = Some(
                            "This is not a transaction. Show the signed PSBT on the device.".into(),
                        );
                        continue;
                    };
                    let mut merged = psbt.clone();
                    match merge_signatures(&mut merged, &signed) {
                        Ok(added) => {
                            *sign = Sign::Done {
                                psbt: merged,
                                added,
                                saved: None,
                                error: None,
                            };
                            return;
                        }
                        Err(e) => scan.error = Some(e.to_string()),
                    }
                }
                Screen::Key(key @ Key::Scan(_)) => {
                    let Key::Scan(scan) = key else { unreachable!() };
                    let keys = match scanned {
                        Scanned::Keys(keys) => Ok(keys),
                        Scanned::Text(text) => codec::parse_keys(&text).map_err(|e| e.to_string()),
                        Scanned::Psbt(_) => {
                            Err("This is a transaction, not an extended public key.".to_string())
                        }
                    };
                    match keys {
                        Ok(keys) => {
                            *key = Key::Done { keys, copied: None };
                            return;
                        }
                        Err(e) => scan.error = Some(e),
                    }
                }
                _ => return,
            }
        }
    }
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

/// A PSBT exported by Liana is base64 text; also accept a binary PSBT file.
fn parse_psbt(text: &str) -> Result<Psbt, String> {
    let psbt = Psbt::from_str(text.trim()).map_err(|e| format!("This is not a valid PSBT: {e}"))?;
    if psbt.inputs.is_empty() {
        return Err("This PSBT has no input.".into());
    }
    Ok(psbt)
}

async fn pick_text_file(name: &str, extensions: &[&str]) -> Result<String, String> {
    let file = rfd::AsyncFileDialog::new()
        .add_filter(name, extensions)
        .pick_file()
        .await
        .ok_or_else(|| "No file selected.".to_string())?;
    read_text_file(file.path())
}

/// Read a text file, converting a binary PSBT to base64.
fn read_text_file(path: &std::path::Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    if bytes.starts_with(b"psbt\xff") {
        use base64::Engine;
        return Ok(base64::engine::general_purpose::STANDARD.encode(bytes));
    }
    String::from_utf8(bytes).map_err(|_| "This file is not a text file.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIANA_DESCRIPTOR: &str = "wsh(or_d(multi(2,[f714c228/48'/1'/0'/2']tpubDEwJnTwfKoMvu8AXXBPydBVWDpzNP5tatjjZ56q4TQioGL7iL9xzTbMoCCQ3tfGihtff7vtR4xsjcRuhZ7HWARVAkGZ1HZcpBhVdou76k7j/<0;1>/*,[2522f23c/48'/1'/0'/2']tpubDEoTU4bDW1EXN1rnLXnRfue1a7DeqjJcs39PkEeLcVXhVKzCnFo9yQX2EeeXJ6kh4hgbz5o9v7YAc1EE97AEJpJbKNmDxE3ZQo4msGPSp2J/<0;1>/*),and_v(v:thresh(1,pkh([f714c228/48'/1'/0'/2']tpubDEwJnTwfKoMvu8AXXBPydBVWDpzNP5tatjjZ56q4TQioGL7iL9xzTbMoCCQ3tfGihtff7vtR4xsjcRuhZ7HWARVAkGZ1HZcpBhVdou76k7j/<2;3>/*),a:pkh([2522f23c/48'/1'/0'/2']tpubDEoTU4bDW1EXN1rnLXnRfue1a7DeqjJcs39PkEeLcVXhVKzCnFo9yQX2EeeXJ6kh4hgbz5o9v7YAc1EE97AEJpJbKNmDxE3ZQo4msGPSp2J/<2;3>/*)),older(65535))))#9s8ekrce";

    #[test]
    fn register_liana_descriptor() {
        let (mut app, _) = App::new(None);
        let _ = app.update(Message::Open(Flow::Register));
        let _ = app.update(Message::DescriptorEdited(LIANA_DESCRIPTOR.to_string()));
        let Screen::Register(Register::Edit { descriptor, .. }) = &app.screen else {
            panic!("expected the edit screen");
        };
        assert!(descriptor.valid);

        let _ = app.update(Message::ShowRegistration);
        let Screen::Register(Register::Show { qr, .. }) = &app.screen else {
            panic!("expected the QR screen");
        };
        // Long descriptors need an animation with the default Specter transport.
        assert!(qr.frame_count() > 1);
        assert!(qr.image.is_some());
    }

    #[test]
    fn invalid_descriptor_is_flagged() {
        let (mut app, _) = App::new(None);
        let _ = app.update(Message::Open(Flow::Register));
        let _ = app.update(Message::DescriptorEdited("wsh(pk(nope))".to_string()));
        let Screen::Register(Register::Edit { descriptor, .. }) = &app.screen else {
            panic!("expected the edit screen");
        };
        assert!(!descriptor.valid);
    }

    #[test]
    fn sign_flow_merges_scanned_signatures() {
        use crate::codec::tests_support::TEST_PSBT;
        let (mut app, _) = App::new(None);
        let _ = app.update(Message::Open(Flow::Sign));
        let _ = app.update(Message::PsbtLoaded(Ok(TEST_PSBT.to_string())));
        assert!(matches!(app.screen, Screen::Sign(Sign::Show { .. })));
        let _ = app.update(Message::StartScan);

        // The device answers with the same PSBT plus a signature, as a single base64 QR.
        let mut signed = Psbt::from_str(TEST_PSBT).unwrap();
        let secp = miniscript::bitcoin::secp256k1::Secp256k1::new();
        let sk = miniscript::bitcoin::secp256k1::SecretKey::from_slice(&[3; 32]).unwrap();
        let msg = miniscript::bitcoin::secp256k1::Message::from_digest([4; 32]);
        signed.inputs[0].partial_sigs.insert(
            miniscript::bitcoin::PublicKey::new(sk.public_key(&secp)),
            miniscript::bitcoin::ecdsa::Signature::sighash_all(secp.sign_ecdsa(&msg, &sk)),
        );
        let _ = app.update(Message::Camera(CameraEvent::Decoded(signed.to_string())));
        let Screen::Sign(Sign::Done { psbt, added, .. }) = &app.screen else {
            panic!("expected the done screen");
        };
        assert_eq!(*added, 1);
        assert_eq!(psbt, &signed);
    }
}
