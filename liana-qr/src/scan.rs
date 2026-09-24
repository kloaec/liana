//! Reading QR codes: from a webcam (feature `camera`) or from an image file.

use iced::futures::{channel::mpsc, SinkExt, Stream, StreamExt};

/// Decode every QR code visible in a greyscale image.
pub fn decode_luma(width: usize, height: usize, luma: &[u8]) -> Vec<String> {
    let mut image =
        rqrr::PreparedImage::prepare_from_greyscale(width, height, |x, y| luma[y * width + x]);
    image
        .detect_grids()
        .into_iter()
        .filter_map(|grid| grid.decode().ok().map(|(_, content)| content))
        .collect()
}

/// Decode the QR codes of an image file (a screenshot or a photo of the device).
pub fn decode_file(path: &std::path::Path) -> Result<Vec<String>, String> {
    let image = image::open(path).map_err(|e| e.to_string())?.to_luma8();
    let (w, h) = (image.width() as usize, image.height() as usize);
    Ok(decode_luma(w, h, image.as_raw()))
}

#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "camera"), allow(dead_code))]
pub enum CameraEvent {
    /// The latest frame, for the preview.
    Frame(iced::widget::image::Handle),
    /// Content of a QR code seen in a frame.
    Decoded(String),
    Error(String),
}

/// Stream frames and decoded QR codes from the default camera, for as long as the stream is
/// polled: dropping it (leaving the scan screen) stops the capture thread and frees the camera.
pub fn camera() -> impl Stream<Item = CameraEvent> {
    iced::stream::channel(16, |mut output: mpsc::Sender<CameraEvent>| async move {
        let (tx, mut rx) = mpsc::unbounded();
        std::thread::spawn(move || capture(tx));
        while let Some(event) = rx.next().await {
            if output.send(event).await.is_err() {
                break;
            }
        }
        // Keep the subscription alive without restarting the capture on error.
        std::future::pending::<()>().await;
    })
}

#[cfg(feature = "camera")]
fn capture(tx: mpsc::UnboundedSender<CameraEvent>) {
    use nokhwa::{
        pixel_format::RgbFormat,
        utils::{CameraIndex, RequestedFormat, RequestedFormatType},
        Camera,
    };

    // Asks for the camera permission on macOS, no-op elsewhere.
    nokhwa::nokhwa_initialize(|_| {});

    let format = RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate);
    let mut camera = match Camera::new(CameraIndex::Index(0), format) {
        Ok(camera) => camera,
        Err(e) => {
            let _ = tx.unbounded_send(CameraEvent::Error(format!("No camera available: {e}")));
            return;
        }
    };
    if let Err(e) = camera.open_stream() {
        let _ = tx.unbounded_send(CameraEvent::Error(format!("Cannot start the camera: {e}")));
        return;
    }

    loop {
        let frame = match camera.frame().and_then(|f| f.decode_image::<RgbFormat>()) {
            Ok(frame) => frame,
            Err(e) => {
                let _ = tx.unbounded_send(CameraEvent::Error(format!("Camera error: {e}")));
                break;
            }
        };
        let (w, h) = (frame.width(), frame.height());
        let rgb = frame.into_raw();
        let luma: Vec<u8> = rgb
            .chunks_exact(3)
            .map(|p| ((p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000) as u8)
            .collect();
        for content in decode_luma(w as usize, h as usize, &luma) {
            if tx.unbounded_send(CameraEvent::Decoded(content)).is_err() {
                return;
            }
        }
        let rgba: Vec<u8> = rgb
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect();
        let handle = iced::widget::image::Handle::from_rgba(w, h, rgba);
        // The receiver is gone once the scan screen is left: release the camera.
        if tx.unbounded_send(CameraEvent::Frame(handle)).is_err() {
            break;
        }
    }
    let _ = camera.stop_stream();
}

#[cfg(not(feature = "camera"))]
fn capture(tx: mpsc::UnboundedSender<CameraEvent>) {
    let _ = tx.unbounded_send(CameraEvent::Error(
        "This build has no camera support. Load a picture of the QR code instead.".into(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render a QR code the way the UI does, and read it back like a camera frame.
    #[test]
    fn decode_rendered_qr() {
        let content = "UR:CRYPTO-PSBT/1-3/LPADAXCFAXHDCXHKGRGHTS";
        let code = qrcode::QrCode::new(content.as_bytes()).unwrap();
        // 6 pixels per module and a 4 modules quiet zone.
        let (scale, quiet) = (6, 4);
        let modules = code.width();
        let w = (modules + 2 * quiet) * scale;
        let colors = code.to_colors();
        let mut luma = vec![255u8; w * w];
        for y in 0..w {
            for x in 0..w {
                let (mx, my) = (
                    (x / scale) as isize - quiet as isize,
                    (y / scale) as isize - quiet as isize,
                );
                if mx >= 0
                    && my >= 0
                    && (mx as usize) < modules
                    && (my as usize) < modules
                    && colors[my as usize * modules + mx as usize] == qrcode::Color::Dark
                {
                    luma[y * w + x] = 0;
                }
            }
        }
        assert_eq!(decode_luma(w, w, &luma), vec![content.to_string()]);
    }
}
