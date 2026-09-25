//! Liana QR bridge: a stopgap companion to Liana for airgapped signing devices that talk over QR
//! codes (Specter DIY, Krux, Coldcard Q, Passport Prime).
//!
//! Liana starts it for one action (sign, register the wallet, import a key) and reads the result
//! from its output, see `protocol`. It is meant to be dropped once Liana supports QR signing
//! devices natively.

#![windows_subsystem = "windows"]

mod app;
mod codec;
mod device;
mod protocol;
mod psbt;
mod scan;
mod view;

use liana_ui::{component::text, font, theme};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let request = match protocol::parse(&args, std::io::stdin()) {
        Ok(request) => request,
        Err(e) => {
            eprintln!("liana-qr: {e}\n\n{}", protocol::USAGE);
            std::process::exit(2);
        }
    };

    let result = iced::application(
        move || app::App::new(request.clone()),
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
    .run();
    if let Err(e) = result {
        eprintln!("liana-qr: {e}");
        std::process::exit(1);
    }
}
