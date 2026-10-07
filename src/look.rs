//! Pictures of the part for the AI planner: the overview its first request
//! carries and the views the model asks for with the `view_model` tool.

/// The viewport's background, under everything a picture draws.
const BACKGROUND: [u8; 3] = [239, 243, 247];
const INK: [u8; 3] = [40, 46, 54];
/// A 5 × 7 glyph per character, one byte a row, bit 4 the leftmost pixel.
fn glyph(c: char) -> Option<[u8; 7]> {
    Some(match c {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        ' ' => [0; 7],
        '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x0C, 0x04, 0x08],
        ':' => [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00],
        '/' => [0x00, 0x01, 0x02, 0x04, 0x08, 0x10, 0x00],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        'Ø' => [0x0D, 0x12, 0x15, 0x15, 0x15, 0x09, 0x16],
        '×' => [0x00, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x00],
        '%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
        '…' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x15, 0x15],
        '·' => [0x00, 0x00, 0x00, 0x0C, 0x0C, 0x00, 0x00],
        '?' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
        _ => return None,
    })
}
/// An opaque RGB picture being drawn.
pub(crate) struct Canvas {
    pub w: u32,
    pub h: u32,
    pub rgb: Vec<u8>,
}
impl Canvas {
    #[cfg(test)]
    fn blank(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            rgb: BACKGROUND.repeat((w * h) as usize),
        }
    }
    #[cfg(test)]
    fn at(&self, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * self.w + x) * 3) as usize;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }
    /// `rgba` laid over the background; mirrored left to right when `flip`.
    pub(crate) fn over(rgba: &[u8], w: u32, h: u32, flip: bool) -> Self {
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                let sx = if flip { w - 1 - x } else { x };
                let i = ((y * w + sx) * 4) as usize;
                let a = rgba[i + 3] as f32 / 255.;
                for k in 0..3 {
                    rgb.push(
                        (rgba[i + k] as f32 * a + BACKGROUND[k] as f32 * (1. - a)).round() as u8,
                    );
                }
            }
        }
        Self { w, h, rgb }
    }
    fn put(&mut self, x: i32, y: i32, color: [u8; 3]) {
        if x >= 0 && y >= 0 && (x as u32) < self.w && (y as u32) < self.h {
            let i = ((y as u32 * self.w + x as u32) * 3) as usize;
            self.rgb[i..i + 3].copy_from_slice(&color);
        }
    }
    pub(crate) fn fill(&mut self, [x0, y0, x1, y1]: [i32; 4], color: [u8; 3]) {
        for y in y0..y1 {
            for x in x0..x1 {
                self.put(x, y, color);
            }
        }
    }
    pub(crate) fn line(&mut self, a: (f32, f32), b: (f32, f32), color: [u8; 3], width: f32) {
        let steps = ((b.0 - a.0).hypot(b.1 - a.1).ceil() as usize).clamp(1, 4000);
        // Floored so a hairline's disc still covers a pixel centre on each step.
        let r = (width / 2.).max(0.75);
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            for yy in (y - r).floor() as i32..=(y + r).ceil() as i32 {
                for xx in (x - r).floor() as i32..=(x + r).ceil() as i32 {
                    if (xx as f32 + 0.5 - x).powi(2) + (yy as f32 + 0.5 - y).powi(2) <= r * r {
                        self.put(xx, yy, color);
                    }
                }
            }
        }
    }
    pub(crate) fn text_width(text: &str, scale: u32) -> u32 {
        (text.chars().count() as u32 * 6).saturating_sub(1) * scale
    }
    /// `text` in capitals from its top-left corner; unknown characters as "?".
    pub(crate) fn text(&mut self, x: i32, y: i32, text: &str, scale: u32, color: [u8; 3]) {
        let s = scale as i32;
        for (n, c) in text.chars().flat_map(char::to_uppercase).enumerate() {
            let rows = glyph(c).unwrap_or_else(|| glyph('?').unwrap());
            for (row, bits) in rows.iter().enumerate() {
                for col in 0..5 {
                    if bits & (0x10 >> col) != 0 {
                        let (px, py) = (x + (n as i32 * 6 + col) * s, y + row as i32 * s);
                        self.fill([px, py, px + s, py + s], color);
                    }
                }
            }
        }
    }
    /// A label box centred on `center`; None when it would leave the picture.
    pub(crate) fn tag(&mut self, center: (f32, f32), text: &str) -> Option<[i32; 4]> {
        let (tw, th) = (Self::text_width(text, 2) as i32, 14);
        let (w, h) = (tw + 8, th + 8);
        let x0 = center.0.round() as i32 - w / 2;
        let y0 = center.1.round() as i32 - h / 2;
        let rect = [x0, y0, x0 + w, y0 + h];
        if x0 < 0 || y0 < 0 || rect[2] > self.w as i32 || rect[3] > self.h as i32 {
            return None;
        }
        self.fill(rect, INK);
        self.fill([x0 + 1, y0 + 1, rect[2] - 1, rect[3] - 1], [255, 255, 255]);
        self.text(x0 + 4, y0 + 4, text, 2, INK);
        Some(rect)
    }
    pub(crate) fn png(&self) -> Vec<u8> {
        let mut out = vec![];
        let mut encoder = png::Encoder::new(&mut out, self.w, self.h);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("PNG header");
        writer.write_image_data(&self.rgb).expect("PNG data");
        drop(writer);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_character_a_tag_or_caption_uses_has_a_glyph() {
        for c in "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ +-.,:/()Ø×%…·".chars() {
            assert!(glyph(c).is_some(), "{c}");
        }
        assert!(glyph('ä').is_none());
        // Lower case is drawn as capitals; anything else as "?".
        let mut a = Canvas::blank(40, 20);
        let mut b = Canvas::blank(40, 20);
        a.text(0, 0, "ab", 1, [0, 0, 0]);
        b.text(0, 0, "AB", 1, [0, 0, 0]);
        assert_eq!(a.rgb, b.rgb);
    }
    #[test]
    fn a_tag_is_drawn_centred_and_refused_at_the_edge() {
        let mut c = Canvas::blank(200, 100);
        let rect = c.tag((100., 50.), "12").unwrap();
        assert!(rect[0] < 100 && rect[2] > 100 && rect[1] < 50 && rect[3] > 50);
        // Ink strictly inside the 1 px border can only be the text.
        let inked = (rect[0] + 2..rect[2] - 2)
            .any(|x| (rect[1] + 2..rect[3] - 2).any(|y| c.at(x as u32, y as u32) == INK));
        assert!(inked, "the text is drawn inside the box");
        assert!(c.tag((2., 50.), "12").is_none());
    }
    #[test]
    fn text_draws_the_glyph_pixels() {
        let mut c = Canvas::blank(40, 20);
        c.text(0, 0, "1", 1, INK);
        // '1' is [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E], bit 4 leftmost.
        assert_eq!(c.at(2, 0), INK);
        assert_eq!(c.at(0, 0), BACKGROUND);
        for x in 1..=3 {
            assert_eq!(c.at(x, 6), INK);
        }
        let mut t = Canvas::blank(40, 20);
        t.text(0, 0, "AB", 1, INK);
        assert_ne!(t.rgb, Canvas::blank(40, 20).rgb);
        let mut unknown = Canvas::blank(40, 20);
        unknown.text(0, 0, "ä", 1, INK);
        let mut question = Canvas::blank(40, 20);
        question.text(0, 0, "?", 1, INK);
        assert_ne!(question.rgb, Canvas::blank(40, 20).rgb);
        assert_eq!(unknown.rgb, question.rgb);
    }
    #[test]
    fn a_hairline_is_drawn_without_gaps() {
        let mut h = Canvas::blank(80, 70);
        h.line((10., 10.), (60., 10.), INK, 1.);
        let row = (0..80).filter(|&x| h.at(x, 10) == INK).count();
        assert!(row >= 45, "{row} pixels in row 10");
        let mut d = Canvas::blank(80, 70);
        d.line((10., 10.), (60., 40.), INK, 1.);
        for x in 10..=60 {
            assert!((0..70).any(|y| d.at(x, y) == INK), "column {x} has a gap");
        }
    }
    #[test]
    fn composite_flips_and_fills_the_background() {
        // A 2 × 1 frame: left pixel red and opaque, right pixel empty.
        let rgba = [255, 0, 0, 255, 0, 0, 0, 0];
        let c = Canvas::over(&rgba, 2, 1, true);
        assert_eq!(c.at(0, 0), [239, 243, 247]);
        assert_eq!(c.at(1, 0), [255, 0, 0]);
        let png = c.png();
        assert_eq!(&png[1..4], b"PNG");
    }
}
