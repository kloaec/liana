//! Screens, built from liana-ui components so the bridge looks like the Liana installer.
//!
//! The bridge is a stopgap tool, so its few specific widgets live in `ui` below rather than in
//! liana-ui: deleting this crate must leave nothing behind.

use iced::{
    widget::{column, image, row, Space},
    Alignment,
};
use liana_ui::{
    component::{
        badge::Tile,
        button::{EntryWidth, STANDARD_ENTRY_WIDTH},
        card, form,
        installer::{self, LayoutConfig, NavBar},
        list, pick_list, text,
    },
    spacing::VSpacing,
    theme,
    widget::*,
    Variant,
};
use miniscript::bitcoin::Network;

use crate::{
    app::{AnimatedQr, App, Flow, Key, Message, Register, ScanState, Screen, Sign, QR_SIZE},
    codec::Density,
    device::Device,
};

const CONTENT_WIDTH: f32 = 800.0;
const PREVIEW_WIDTH: f32 = 480.0;

pub fn view(app: &App) -> Element<'_, Message> {
    let (title, progress, previous, content) = match &app.screen {
        Screen::Home => (String::new(), (0, 0), None, home(app.device)),
        Screen::Sign(sign) => {
            let (step, content) = match sign {
                Sign::Load { error } => (1, sign_load(error.as_deref())),
                Sign::Show { qr, .. } => (2, sign_show(app, qr)),
                Sign::Scan { scan, .. } => (3, sign_scan(scan)),
                Sign::Done {
                    added,
                    saved,
                    error,
                    ..
                } => (4, sign_done(*added, saved.as_deref(), error.as_deref())),
            };
            (
                "Sign a transaction".to_string(),
                (step, 4),
                Some(Message::Previous),
                content,
            )
        }
        Screen::Register(register) => {
            let (step, content) = match register {
                Register::Edit { name, descriptor } => (1, register_edit(name, descriptor)),
                Register::Show { qr, .. } => (2, register_show(app, qr)),
            };
            (
                "Register the wallet".to_string(),
                (step, 2),
                Some(Message::Previous),
                content,
            )
        }
        Screen::Key(key) => {
            let (step, content) = match key {
                Key::Scan(scan) => (1, key_scan(app.device, scan)),
                Key::Done { keys, copied } => (2, key_done(keys, *copied)),
            };
            (
                "Import a key".to_string(),
                (step, 2),
                Some(Message::Previous),
                content,
            )
        }
    };

    let nav_bar = NavBar::StepTitle {
        progress,
        title,
        previous_message: previous,
    };
    installer::layout(
        LayoutConfig {
            variant: Variant::Liana,
            network: Network::Bitcoin,
            email: None,
            is_ws_admin: false,
            nav_bar,
            content_width: CONTENT_WIDTH,
        },
        column![content, Space::with_height(VSpacing::XXL)],
    )
}

fn home<'a>(device: Device) -> Element<'a, Message> {
    let intro = installer::screen_intro(
        "QR signing bridge",
        Some(installer::intro_description(
            "Use an airgapped signing device with Liana. The bridge shows your device QR codes \
             and reads its answers, then hands the result back to Liana through a file or the \
             clipboard.",
        )),
        false,
    );
    let entries = column![
        list::entry_action(
            Tile::Device,
            "Sign a transaction",
            Some("Show a PSBT exported from Liana, then scan it back signed"),
            None,
            EntryWidth::Standard,
            Some(Message::Open(Flow::Sign)),
        ),
        list::entry_action(
            Tile::Wallet,
            "Register the wallet on a device",
            Some("Show the wallet descriptor so the device can verify spends and addresses"),
            None,
            EntryWidth::Standard,
            Some(Message::Open(Flow::Register)),
        ),
        list::entry_action(
            Tile::KeyInternal,
            "Import a key from a device",
            Some("Scan an extended public key and copy it for Liana's wallet creation"),
            None,
            EntryWidth::Standard,
            Some(Message::Open(Flow::Key)),
        ),
    ]
    .spacing(VSpacing::S)
    .align_x(Alignment::Center);

    column![
        intro,
        ui::device_picker(device, None),
        entries,
        Container::new(card::info(
            "To check a receive address, show its QR code in Liana and scan it with the device.",
        ))
        .width(STANDARD_ENTRY_WIDTH),
    ]
    .spacing(VSpacing::XXL)
    .align_x(Alignment::Center)
    .into()
}

fn sign_load<'a>(error: Option<&str>) -> Element<'a, Message> {
    column![
        ui::prompt("Load the transaction to sign"),
        ui::caption(
            "In Liana, open the transaction and export it (Export > PSBT), or copy the PSBT.",
        ),
        error.map(ui::error),
        column![
            list::entry_action(
                Tile::Import,
                "Load a PSBT file",
                Some("The .psbt file exported from Liana"),
                None,
                EntryWidth::Standard,
                Some(Message::LoadPsbtFile),
            ),
            list::entry_action(
                Tile::Paste,
                "Paste a PSBT",
                Some("A base64 PSBT from the clipboard"),
                None,
                EntryWidth::Standard,
                Some(Message::PastePsbt),
            ),
        ]
        .spacing(VSpacing::S),
    ]
    .spacing(VSpacing::XL)
    .align_x(Alignment::Center)
    .into()
}

fn sign_show<'a>(app: &'a App, qr: &'a AnimatedQr) -> Element<'a, Message> {
    column![
        ui::prompt("Scan this with your device"),
        ui::format_pickers(app.device, app.density),
        app.device.caveat().map(ui::warning),
        ui::caption(app.device.sign_hint()),
        ui::qr(qr),
        ui::primary("Scan the signed transaction", Some(Message::StartScan)),
    ]
    .spacing(VSpacing::L)
    .align_x(Alignment::Center)
    .into()
}

fn sign_scan(scan: &ScanState) -> Element<'_, Message> {
    column![
        ui::prompt("Show the signed transaction to the camera"),
        ui::caption("Once signed, the device displays the transaction as a QR code."),
        ui::scanner(scan),
    ]
    .spacing(VSpacing::XL)
    .align_x(Alignment::Center)
    .into()
}

fn sign_done<'a>(
    added: usize,
    saved: Option<&std::path::Path>,
    error: Option<&str>,
) -> Element<'a, Message> {
    let signatures = if added == 1 {
        "1 signature added".to_string()
    } else {
        format!("{added} signatures added")
    };
    let saved = saved.map(|path| ui::caption(format!("Saved to {}", path.display())));
    column![
        ui::success(signatures),
        ui::caption(
            "Save the signed PSBT, then import it in Liana: open the transaction and choose \
             Import. Liana merges the new signatures.",
        ),
        saved,
        error.map(ui::error),
        row![
            ui::secondary("Copy PSBT", Some(Message::CopyPsbt)),
            ui::primary("Save PSBT file", Some(Message::SavePsbt)),
        ]
        .spacing(VSpacing::M),
        list::entry_action(
            Tile::Device,
            "Sign with another device",
            Some("Show the transaction again, with these signatures, to the next signer"),
            None,
            EntryWidth::Standard,
            Some(Message::SignWithAnotherDevice),
        ),
        ui::secondary("Done", Some(Message::Finish)),
    ]
    .spacing(VSpacing::XL)
    .align_x(Alignment::Center)
    .into()
}

fn register_edit<'a>(
    name: &'a form::Value<String>,
    descriptor: &'a form::Value<String>,
) -> Element<'a, Message> {
    let ready = name.valid && descriptor.valid && !descriptor.value.is_empty();
    column![
        ui::prompt("Enter the wallet descriptor"),
        ui::caption(
            "In Liana, go to Settings > Wallet and copy the descriptor, or export it from \
             Settings > Import/Export.",
        ),
        Container::new(
            column![
                form::Form::new("Wallet name", name, Message::NameEdited)
                    .label("Wallet name")
                    .warning("Enter a name without '&'")
                    .padding(10),
                form::Form::new_trimmed("Descriptor", descriptor, Message::DescriptorEdited)
                    .label("Descriptor")
                    .warning(
                        descriptor
                            .warning
                            .clone()
                            .unwrap_or_else(|| "This is not a valid descriptor".to_string()),
                    )
                    .padding(10),
            ]
            .spacing(VSpacing::L),
        )
        .width(STANDARD_ENTRY_WIDTH),
        list::entry_action(
            Tile::Import,
            "Load a descriptor file",
            Some("The descriptor file exported from Liana"),
            None,
            EntryWidth::Standard,
            Some(Message::LoadDescriptorFile),
        ),
        ui::primary("Show on screen", ready.then_some(Message::ShowRegistration)),
    ]
    .spacing(VSpacing::XL)
    .align_x(Alignment::Center)
    .into()
}

fn register_show<'a>(app: &'a App, qr: &'a AnimatedQr) -> Element<'a, Message> {
    column![
        ui::prompt("Scan this with your device"),
        ui::format_pickers(app.device, app.density),
        app.device.caveat().map(ui::warning),
        ui::caption(app.device.register_hint()),
        ui::qr(qr),
        ui::caption(
            "Check on the device that every key and timelock matches the wallet in Liana.",
        ),
        ui::primary("Done", Some(Message::Finish)),
    ]
    .spacing(VSpacing::L)
    .align_x(Alignment::Center)
    .into()
}

fn key_scan(device: Device, scan: &ScanState) -> Element<'_, Message> {
    column![
        ui::prompt("Show the extended public key to the camera"),
        ui::device_picker(device, None),
        ui::caption(device.key_hint()),
        ui::scanner(scan),
    ]
    .spacing(VSpacing::XL)
    .align_x(Alignment::Center)
    .into()
}

fn key_done(keys: &[crate::codec::ExtendedKey], copied: Option<usize>) -> Element<'_, Message> {
    let entries =
        keys.iter()
            .enumerate()
            .fold(Column::new().spacing(VSpacing::S), |col, (i, key)| {
                let mut notes = vec![format!("#{} · m/{}", key.fingerprint, key.path)];
                if !key.script.is_empty() {
                    notes.push(key.script.clone());
                }
                if !key.is_mainnet() {
                    notes.push("testnet".into());
                }
                let title = match key.liana_account() {
                    Some(0) => "Liana key".to_string(),
                    Some(account) => format!("Liana key, account #{account}"),
                    None => "Key with a non-standard path".to_string(),
                };
                let trailing = (copied == Some(i)).then(|| ui::caption("Copied"));
                col.push(list::entry_action(
                    Tile::KeyExternal,
                    title,
                    Some(notes.join(" · ")),
                    trailing,
                    EntryWidth::Standard,
                    Some(Message::CopyKey(i)),
                ))
            });
    let non_standard = keys.iter().all(|k| k.liana_account().is_none()).then(|| {
        ui::warning(
            "Liana expects keys at m/48'/0'/account'/2' (m/48'/1'/account'/2' on test \
             networks). Export the multisig key from the device.",
        )
    });
    column![
        ui::prompt("Copy the key"),
        ui::caption(
            "Click a key to copy it, then paste it in Liana where the wallet creation asks for \
             the key's extended public key.",
        ),
        non_standard,
        entries,
        ui::secondary("Done", Some(Message::Finish)),
    ]
    .spacing(VSpacing::XL)
    .align_x(Alignment::Center)
    .into()
}

mod ui {
    use super::*;
    use iced::widget::column;
    use liana_ui::component::button::{btn_primary, btn_secondary, BtnWidth};

    pub fn prompt<'a>(value: &'a str) -> Element<'a, Message> {
        installer::intro_prompt(value, None::<String>)
    }

    pub fn caption<'a>(value: impl std::fmt::Display) -> Element<'a, Message> {
        installer::intro_description(value)
    }

    pub fn primary<'a>(label: &'a str, msg: Option<Message>) -> Element<'a, Message> {
        btn_primary(None, label, BtnWidth::XXL, msg).into()
    }

    pub fn secondary<'a>(label: &'a str, msg: Option<Message>) -> Element<'a, Message> {
        btn_secondary(None, label, BtnWidth::XL, msg).into()
    }

    pub fn error<'a>(message: &str) -> Element<'a, Message> {
        Container::new(text::new::caption(message.to_string()).style(theme::text::error))
            .width(STANDARD_ENTRY_WIDTH)
            .padding(card::CardPadding::Soft)
            .style(theme::card::error)
            .into()
    }

    pub fn warning<'a>(message: &'a str) -> Element<'a, Message> {
        Container::new(text::new::caption(message))
            .width(STANDARD_ENTRY_WIDTH)
            .padding(card::CardPadding::Soft)
            .style(theme::card::soft_warning)
            .into()
    }

    pub fn success<'a>(message: String) -> Element<'a, Message> {
        Container::new(
            row![
                liana_ui::icon::check_icon().style(theme::text::success),
                text::new::b3_medium(message),
            ]
            .spacing(VSpacing::S)
            .align_y(Alignment::Center),
        )
        .padding(card::CardPadding::Soft)
        .style(theme::card::success)
        .into()
    }

    pub fn device_picker<'a>(device: Device, width: Option<f32>) -> Element<'a, Message> {
        let picker =
            pick_list::field_pick_list(&Device::ALL[..], Some(device), Message::DeviceSelected);
        column![
            text::new::b5_medium("Signing device").style(theme::text::secondary),
            picker,
        ]
        .spacing(VSpacing::XS)
        .width(width.unwrap_or(320.0))
        .into()
    }

    pub fn format_pickers<'a>(device: Device, density: Density) -> Element<'a, Message> {
        let density = column![
            text::new::b5_medium("QR density").style(theme::text::secondary),
            pick_list::field_pick_list(&Density::ALL[..], Some(density), Message::DensitySelected),
        ]
        .spacing(VSpacing::XS)
        .width(220.0);
        row![device_picker(device, Some(280.0)), density]
            .spacing(VSpacing::M)
            .into()
    }

    pub fn qr<'a>(qr: &'a AnimatedQr) -> Element<'a, Message> {
        let code: Element<'a, Message> = match &qr.image {
            Some(handle) => Container::new(image(handle.clone()))
                .center(QR_SIZE as f32)
                .into(),
            None => Space::with_height(QR_SIZE as f32).into(),
        };
        let count = qr.frame_count();
        let position = (count > 1).then(|| {
            text::new::caption(format!("Part {} of {count}", qr.position + 1))
                .style(theme::text::secondary)
        });
        column![code, position, qr.error.as_deref().map(error)]
            .spacing(VSpacing::S)
            .align_x(Alignment::Center)
            .into()
    }

    pub fn scanner(scan: &ScanState) -> Element<'_, Message> {
        let preview: Element<'_, Message> = match (&scan.preview, &scan.camera_error) {
            (Some(handle), _) => image(handle.clone()).width(PREVIEW_WIDTH).into(),
            (None, Some(e)) => Container::new(
                column![
                    text::new::b4_medium("No camera"),
                    text::new::caption(e.clone())
                        .style(theme::text::secondary)
                        .align_x(Alignment::Center),
                ]
                .spacing(VSpacing::S)
                .align_x(Alignment::Center),
            )
            .padding(30)
            .center_x(PREVIEW_WIDTH)
            .center_y(PREVIEW_WIDTH * 0.75)
            .style(theme::card::simple)
            .into(),
            (None, None) => Container::new(
                text::new::caption("Starting the camera…").style(theme::text::secondary),
            )
            .center_x(PREVIEW_WIDTH)
            .center_y(PREVIEW_WIDTH * 0.75)
            .style(theme::card::simple)
            .into(),
        };
        let progress = scan.progress().map(|(received, total)| {
            let label = if total > 0 {
                format!("Received {received} of {total} parts")
            } else {
                format!("Received {received} parts")
            };
            text::new::caption(label).style(theme::text::secondary)
        });
        column![
            preview,
            progress,
            scan.error.as_deref().map(error),
            secondary("Load a picture instead", Some(Message::LoadImage)),
        ]
        .spacing(VSpacing::M)
        .align_x(Alignment::Center)
        .into()
    }
}
