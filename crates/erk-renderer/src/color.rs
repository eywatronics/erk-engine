use erk_style::style::color::{AbsoluteColor, ColorSpace};

/// Straight (non-premultiplied) sRGB bytes, `[r, g, b, a]`.
pub(crate) type Rgba = [u8; 4];

pub(crate) fn srgb_bytes(color: AbsoluteColor) -> Rgba {
    let color = color.to_color_space(ColorSpace::Srgb);
    let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
    [
        byte(color.components.0),
        byte(color.components.1),
        byte(color.components.2),
        byte(color.alpha),
    ]
}
