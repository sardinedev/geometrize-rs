//! Compact SVG output.
//!
//! The format matches the SVGO-optimised covers on marabyte.com:
//!
//! ```text
//! <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 W H"><path fill="#rrggbb" d="M0 0hWvHH0z"/><path fill="#rrggbb" fill-opacity=".5" d="Mx1 y1 x2 y2 x3 y3z"/>…</svg>
//! ```
//!
//! There is no whitespace between elements and no trailing newline, so the
//! model's running byte count is exactly the length of the finished file.

use crate::Rgba;
use crate::shape::Shape;

/// Opening tag up to the viewBox numbers.
const HEADER_START: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 ";

/// Closing tag.
const FOOTER: &str = "</svg>";

/// Size and scale of an SVG drawn from a working image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Frame {
    /// Output width (viewBox units).
    pub width: u32,
    /// Output height, keeping the working image's aspect ratio.
    pub height: u32,
    /// Output units per working-image pixel.
    pub scale: f64,
}

impl Frame {
    /// The frame for scaling a `work_width × work_height` image to
    /// `out_width` wide. `out_width` is treated as at least 1.
    pub(crate) fn new(work_width: u32, work_height: u32, out_width: u32) -> Frame {
        let width = out_width.max(1);
        let scale = f64::from(width) / f64::from(work_width);
        let height = (f64::from(work_height) * scale).round().max(1.0) as u32;
        Frame {
            width,
            height,
            scale,
        }
    }
}

/// Maps a working-image pixel coordinate to output units.
///
/// The `+ 0.5` moves to the pixel's centre; primitive did the same with a
/// `translate(.5 .5)` on a group, here it's baked into the numbers.
pub(crate) fn scale_coordinate(value: i32, scale: f64) -> i64 {
    ((f64::from(value) + 0.5) * scale).round() as i64
}

/// Appends numbers separated by spaces, leaving the space out before a
/// negative number (the minus sign already separates them, as SVGO does).
pub(crate) fn push_numbers(out: &mut String, numbers: &[i64]) {
    for (i, number) in numbers.iter().enumerate() {
        if i > 0 && *number >= 0 {
            out.push(' ');
        }
        out.push_str(&number.to_string());
    }
}

/// Formats a colour as `#rrggbb`, or as `#rgb` when each channel's two hex
/// digits are the same (`#ffffff` → `#fff`). Alpha is ignored.
pub(crate) fn hex_color(color: Rgba) -> String {
    let channels = [color.r, color.g, color.b];
    let shortenable = channels.iter().all(|c| c >> 4 == c & 0xf);
    if shortenable {
        format!("#{:x}{:x}{:x}", color.r & 0xf, color.g & 0xf, color.b & 0xf)
    } else {
        format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
    }
}

/// Formats an opacity percentage for `fill-opacity`: two decimals at most and
/// no leading zero (50 → `.5`, 35 → `.35`, 5 → `.05`).
///
/// Returns `None` at 100%, meaning the attribute should be left out.
///
/// # Panics
/// Panics if `percent` is 0 or above 100, which [`crate::Model::new`] rules
/// out. (0 would produce the invalid value `.`.)
pub(crate) fn format_opacity(percent: u8) -> Option<String> {
    assert!(
        (1..=100).contains(&percent),
        "opacity percentage {percent} out of range"
    );
    if percent == 100 {
        return None;
    }
    let digits = format!("{percent:02}");
    Some(format!(".{}", digits.trim_end_matches('0')))
}

/// The presentation attributes of a shape: `fill="#…"`, plus
/// `fill-opacity="…"` unless the opacity is 100%.
pub(crate) fn fill_attributes(color: Rgba, opacity_percent: u8) -> String {
    let mut attrs = format!("fill=\"{}\"", hex_color(color));
    if let Some(value) = format_opacity(opacity_percent) {
        attrs.push_str(&format!(" fill-opacity=\"{value}\""));
    }
    attrs
}

/// Appends the opening `<svg>` tag and the opaque background rectangle.
fn push_header(out: &mut String, frame: Frame, background: Rgba) {
    let (w, h) = (frame.width, frame.height);
    out.push_str(HEADER_START);
    out.push_str(&format!("{w} {h}\">"));
    out.push_str(&format!(
        "<path fill=\"{}\" d=\"M0 0h{w}v{h}H0z\"/>",
        hex_color(background)
    ));
}

/// Appends one shape's element.
fn push_shape(out: &mut String, frame: Frame, shape: &Shape, color: Rgba, opacity_percent: u8) {
    shape.write_svg(out, frame.scale, &fill_attributes(color, opacity_percent));
}

/// Length in bytes of an SVG with only the background.
pub(crate) fn empty_len(frame: Frame, background: Rgba) -> usize {
    let mut out = String::new();
    push_header(&mut out, frame, background);
    out.len() + FOOTER.len()
}

/// Length in bytes that one shape adds to the SVG.
///
/// Uses the same writer as [`document`], so the running byte count can't
/// drift from the real output.
pub(crate) fn shape_len(frame: Frame, shape: &Shape, color: Rgba, opacity_percent: u8) -> usize {
    let mut out = String::new();
    push_shape(&mut out, frame, shape, color, opacity_percent);
    out.len()
}

/// Writes the complete SVG document.
pub(crate) fn document(
    frame: Frame,
    background: Rgba,
    shapes: &[(Shape, Rgba)],
    opacity_percent: u8,
) -> String {
    let mut out = String::new();
    push_header(&mut out, frame, background);
    for (shape, color) in shapes {
        push_shape(&mut out, frame, shape, *color, opacity_percent);
    }
    out.push_str(FOOTER);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::Triangle;

    #[test]
    fn opacity_formatting() {
        assert_eq!(format_opacity(50).as_deref(), Some(".5"));
        assert_eq!(format_opacity(30).as_deref(), Some(".3"));
        assert_eq!(format_opacity(35).as_deref(), Some(".35"));
        assert_eq!(format_opacity(5).as_deref(), Some(".05"));
        assert_eq!(format_opacity(1).as_deref(), Some(".01"));
        assert_eq!(format_opacity(10).as_deref(), Some(".1"));
        assert_eq!(format_opacity(100), None);
    }

    #[test]
    fn opacity_attribute_is_left_out_when_opaque() {
        let color = Rgba::new(255, 255, 255, 255);
        assert_eq!(
            fill_attributes(color, 30),
            "fill=\"#fff\" fill-opacity=\".3\""
        );
        assert_eq!(fill_attributes(color, 100), "fill=\"#fff\"");
    }

    #[test]
    fn colors_are_shortened_when_possible() {
        assert_eq!(hex_color(Rgba::new(255, 255, 255, 255)), "#fff");
        assert_eq!(hex_color(Rgba::new(0x11, 0x22, 0xcc, 255)), "#12c");
        assert_eq!(hex_color(Rgba::new(255, 0, 254, 255)), "#ff00fe");
        assert_eq!(hex_color(Rgba::new(0x0a, 0x0b, 0x0c, 255)), "#0a0b0c");
    }

    #[test]
    fn frame_keeps_the_aspect_ratio() {
        let frame = Frame::new(256, 171, 1200);
        assert_eq!((frame.width, frame.height), (1200, 802));
        assert_eq!(frame.scale, 1200.0 / 256.0);
    }

    #[test]
    fn byte_counts_match_the_document() {
        let frame = Frame::new(32, 32, 64);
        let background = Rgba::new(10, 20, 30, 255);
        let shapes = [
            (
                Shape::Triangle(Triangle {
                    x1: 0,
                    y1: 0,
                    x2: 10,
                    y2: 0,
                    x3: 5,
                    y3: 9,
                }),
                Rgba::new(1, 2, 3, 128),
            ),
            (
                Shape::Triangle(Triangle {
                    x1: -5,
                    y1: 3,
                    x2: 30,
                    y2: 12,
                    x3: 8,
                    y3: 40,
                }),
                Rgba::new(255, 255, 255, 128),
            ),
        ];
        let expected: usize = empty_len(frame, background)
            + shapes
                .iter()
                .map(|(s, c)| shape_len(frame, s, *c, 50))
                .sum::<usize>();
        let doc = document(frame, background, &shapes, 50);
        assert_eq!(doc.len(), expected);
        assert!(doc.starts_with(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 64 64\"><path fill=\"#0a141e\" d=\"M0 0h64v64H0z\"/>"
        ));
        assert!(doc.ends_with("z\"/></svg>"));
    }
}
