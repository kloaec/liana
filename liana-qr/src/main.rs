//! Liana QR bridge: a stopgap companion to Liana for airgapped signing devices that talk over QR
//! codes (Specter DIY, Krux, Coldcard Q, Passport Prime).
//!
//! It exchanges data with Liana only through files and the clipboard, so Liana itself doesn't
//! change. It is meant to be dropped once Liana supports QR signing devices natively.

#![windows_subsystem = "windows"]

mod app;
mod codec;
mod device;
mod psbt;
mod scan;
mod view;

use liana_ui::{component::text, font, theme};

fn main() -> iced::Result {
    // `liana-qr <file.psbt>` opens the signing flow on that PSBT.
    let psbt_path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    iced::application(
        move || app::App::new(psbt_path.clone()),
        app::App::update,
        view::view,
    )
    .title(app::App::title)
    .theme(|_: &_| theme::Theme::default())
    .subscription(app::App::subscription)
    .settings(iced::Settings {
        id: Some("liana-qr".to_string()),
        antialiasing: false,
        default_text_size: text::P1_SIZE.into(),
        default_font: font::REGULAR,
        fonts: font::load(),
        vsync: true,
    })
    .window(iced::window::Settings {
        size: iced::Size::new(1000.0, 900.0),
        min_size: Some(iced::Size::new(900.0, 700.0)),
        ..Default::default()
    })
    .run()
}
