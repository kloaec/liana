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
        card,
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
    app::{AnimatedQr, App, Message, ScanState, Screen, QR_SIZE},
    codec::{Density, ExtendedKey},
    device::Device,
};

const CONTENT_WIDTH: f32 = 800.0;
const PREVIEW_WIDTH: f32 = 480.0;

pub fn view(app: &App) -> Element<'_, Message> {
    let (title, progress, content) = match &app.screen {
        Screen::SignShow { qr, .. } => ("Sign with a QR code device", (1, 2), sign_show(app, qr)),
        Screen::SignScan { scan, .. } => ("Sign with a QR code device", (2, 2), sign_scan(scan)),
        Screen::Register { qr, .. } => ("Register on a QR code device", (0, 0), register(app, qr)),
        Screen::KeyScan { scan, .. } => ("Import a key", (0, 0), key_scan(app.device, scan)),
        Screen::KeyChoose { keys } => ("Import a key", (0, 0), key_choose(keys)),
    };
    // On the first step, "previous" goes back to Liana.
    let nav_bar = NavBar::StepTitle {
        progress,
        title: title.to_string(),
        previous_message: Some(Message::Previous),
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
        ui::caption(
            "Once signed, the device displays the transaction as a QR code. The signatures go \
             back to Liana as soon as it is read.",
        ),
        ui::scanner(scan),
    ]
    .spacing(VSpacing::XL)
    .align_x(Alignment::Center)
    .into()
}

fn register<'a>(app: &'a App, qr: &'a AnimatedQr) -> Element<'a, Message> {
    column![
        ui::prompt("Scan this with your device"),
        ui::format_pickers(app.device, app.density),
        app.device.caveat().map(ui::warning),
        ui::caption(app.device.register_hint()),
        ui::qr(qr),
        ui::caption(
            "Check on the device that every key and timelock matches the wallet in Liana.",
        ),
        ui::primary("The wallet is registered", Some(Message::Registered)),
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

fn key_choose(keys: &[ExtendedKey]) -> Element<'_, Message> {
    let entries =
        keys.iter()
            .enumerate()
            .fold(Column::new().spacing(VSpacing::S), |col, (i, key)| {
                let mut notes = vec![format!("#{} · m/{}", key.fingerprint, key.path)];
                if !key.script.is_empty() {
                    notes.push(key.script.clone());
                }
                let title = match key.liana_account() {
                    Some(0) => "Liana key".to_string(),
                    Some(account) => format!("Liana key, account #{account}"),
                    None => "Key with a non-standard path".to_string(),
                };
                col.push(list::entry_action(
                    Tile::KeyInternal,
                    title,
                    Some(notes.join(" · ")),
                    None,
                    EntryWidth::Standard,
                    Some(Message::SelectKey(i)),
                ))
            });
    let non_standard = keys.iter().all(|k| k.liana_account().is_none()).then(|| {
        ui::warning(
            "Liana expects keys at m/48'/0'/account'/2' (m/48'/1'/account'/2' on test \
             networks). Export the multisig key from the device.",
        )
    });
    column![
        ui::prompt("Choose the key to use"),
        ui::caption("The device shared several keys."),
        non_standard,
        entries,
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
