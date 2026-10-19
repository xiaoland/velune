//! An invitation's QR code, painted as dark modules on a light quiet zone.
use gpui_kit::*;
use qrcode::{Color, QrCode};

pub fn code(text: &str, module: Pixels) -> Option<impl IntoElement> {
    let code = QrCode::new(text.as_bytes()).ok()?;
    let width = code.width();
    let dark: Vec<bool> = code
        .to_colors()
        .into_iter()
        .map(|color| color == Color::Dark)
        .collect();
    let quiet = 4;
    let side = module * (width + quiet * 2) as f32;
    Some(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                window.paint_quad(fill(bounds, white()));
                for (index, _) in dark.iter().enumerate().filter(|(_, dark)| **dark) {
                    let x = (index % width + quiet) as f32;
                    let y = (index / width + quiet) as f32;
                    let origin = bounds.origin + point(module * x, module * y);
                    window.paint_quad(fill(Bounds::new(origin, size(module, module)), black()));
                }
            },
        )
        .size(side),
    )
}
