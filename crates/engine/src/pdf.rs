//! A small PDF writer for reports (the CMM setup sheet): A4 pages with text in the standard
//! Helvetica fonts (WinAnsi, so German text works), filled and stroked shapes and JPEG images.
//! Nothing is embedded but the images; every PDF reader has the standard fonts.

use std::fmt::Write as _;

/// A4 in points.
pub const A4: (f64, f64) = (595.28, 841.89);

/// An RGB colour, 0..1 per channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub f64, pub f64, pub f64);

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0)
    }
}

/// Regular or bold Helvetica.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Font {
    Regular,
    Bold,
}

impl Font {
    fn name(self) -> &'static str {
        match self {
            Font::Regular => "F1",
            Font::Bold => "F2",
        }
    }
}

/// One page: its content stream (y up from the bottom left, in points) and the images it uses.
#[derive(Default)]
pub struct Page {
    ops: String,
    images: Vec<usize>,
}

/// A JPEG image (DCT) of `w × h` pixels.
struct Image {
    w: u32,
    h: u32,
    jpeg: Vec<u8>,
}

#[derive(Default)]
pub struct Document {
    pages: Vec<Page>,
    images: Vec<Image>,
    title: String,
}

fn n(x: f64) -> String {
    let x = if x.is_finite() { x } else { 0.0 };
    let s = format!("{x:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// A text in WinAnsi bytes (Latin-1 and the usual typographic marks); what has no code there
/// becomes a close ASCII stand-in.
pub fn win_ansi(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\u{20}'..='\u{7e}' => out.push(c as u8),
            '\u{a0}'..='\u{ff}' => out.push(c as u32 as u8),
            '€' => out.push(0x80),
            '‚' => out.push(0x82),
            '„' => out.push(0x84),
            '…' => out.push(0x85),
            '‘' => out.push(0x91),
            '’' => out.push(0x92),
            '“' => out.push(0x93),
            '”' => out.push(0x94),
            '•' => out.push(0x95),
            '–' => out.push(0x96),
            '—' => out.push(0x97),
            '→' => out.extend_from_slice(b"->"),
            '←' => out.extend_from_slice(b"<-"),
            '\t' => out.push(b' '),
            '\n' | '\r' => {}
            _ => out.push(b'?'),
        }
    }
    out
}

/// Width of a WinAnsi byte in Helvetica, in 1/1000 em.
fn glyph_width(b: u8, font: Font) -> f64 {
    const REG: [u16; 95] = [
        278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278,
        584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944,
        667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500,
        278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
    ];
    const BOLD: [u16; 95] = [
        278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333,
        584, 584, 584, 611, 975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944,
        667, 667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556,
        333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
    ];
    let table = match font {
        Font::Regular => &REG,
        Font::Bold => &BOLD,
    };
    let base = |c: u8| table.get(usize::from(c.saturating_sub(32))).copied().map_or(556.0, f64::from);
    match b {
        32..=126 => base(b),
        // accented letters as their base letter
        0xC0..=0xC5 => base(b'A'),
        0xC7 => base(b'C'),
        0xC8..=0xCB => base(b'E'),
        0xCC..=0xCF => base(b'I'),
        0xD1 => base(b'N'),
        0xD2..=0xD6 | 0xD8 => base(b'O'),
        0xD9..=0xDC => base(b'U'),
        0xDF => 611.0,
        0xE0..=0xE5 => base(b'a'),
        0xE7 => base(b'c'),
        0xE8..=0xEB => base(b'e'),
        0xEC..=0xEF => base(b'i'),
        0xF1 => base(b'n'),
        0xF2..=0xF6 | 0xF8 => base(b'o'),
        0xF9..=0xFC => base(b'u'),
        0xB0 => 400.0,
        0xB1 | 0xD7 => 584.0,
        0xB7 | 0x95 => 350.0,
        0x96 => 556.0,
        0x97 => 1000.0,
        _ => 556.0,
    }
}

/// Width of a text in points.
pub fn text_width(s: &str, font: Font, size: f64) -> f64 {
    win_ansi(s).iter().map(|b| glyph_width(*b, font)).sum::<f64>() * size / 1000.0
}

/// Breaks a text into lines no wider than `width` (at spaces; a word wider than a line stays
/// whole).
pub fn wrap(s: &str, font: Font, size: f64, width: f64) -> Vec<String> {
    let mut lines = Vec::new();
    for para in s.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if text_width(&candidate, font, size) <= width || line.is_empty() {
                line = candidate;
            } else {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            }
        }
        lines.push(line);
    }
    lines
}

fn pdf_string(s: &str) -> String {
    let mut out = String::from("(");
    for b in win_ansi(s) {
        match b {
            b'(' | b')' | b'\\' => {
                out.push('\\');
                out.push(b as char);
            }
            32..=126 => out.push(b as char),
            _ => {
                let _ = write!(out, "\\{b:03o}");
            }
        }
    }
    out.push(')');
    out
}

impl Page {
    fn op(&mut self, s: &str) {
        self.ops.push_str(s);
        self.ops.push('\n');
    }
    pub fn fill(&mut self, c: Color) {
        self.op(&format!("{} {} {} rg", n(c.0), n(c.1), n(c.2)));
    }
    pub fn stroke(&mut self, c: Color, width: f64) {
        self.op(&format!("{} {} {} RG {} w", n(c.0), n(c.1), n(c.2), n(width)));
    }
    /// A filled rectangle (x, y: bottom left).
    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, c: Color) {
        self.fill(c);
        self.op(&format!("{} {} {} {} re f", n(x), n(y), n(w), n(h)));
    }
    /// A rectangle with rounded corners, filled and/or stroked.
    pub fn round_rect(&mut self, x: f64, y: f64, w: f64, h: f64, r: f64, fill: Option<Color>, line: Option<(Color, f64)>) {
        let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
        let k = r * 0.5523;
        if let Some(c) = fill {
            self.fill(c);
        }
        if let Some((c, w)) = line {
            self.stroke(c, w);
        }
        let (x1, y1) = (x + w, y + h);
        self.op(&format!(
            "{} {} m {} {} l {} {} {} {} {} {} c {} {} l {} {} {} {} {} {} c {} {} l {} {} {} {} {} {} c {} {} l {} {} {} {} {} {} c h {}",
            n(x + r),
            n(y),
            n(x1 - r),
            n(y),
            n(x1 - r + k),
            n(y),
            n(x1),
            n(y + r - k),
            n(x1),
            n(y + r),
            n(x1),
            n(y1 - r),
            n(x1),
            n(y1 - r + k),
            n(x1 - r + k),
            n(y1),
            n(x1 - r),
            n(y1),
            n(x + r),
            n(y1),
            n(x + r - k),
            n(y1),
            n(x),
            n(y1 - r + k),
            n(x),
            n(y1 - r),
            n(x),
            n(y + r),
            n(x),
            n(y + r - k),
            n(x + r - k),
            n(y),
            n(x + r),
            n(y),
            match (fill.is_some(), line.is_some()) {
                (true, true) => "B",
                (true, false) => "f",
                (false, true) => "S",
                (false, false) => "n",
            }
        ));
    }
    /// A circle, filled and/or stroked.
    pub fn circle(&mut self, cx: f64, cy: f64, r: f64, fill: Option<Color>, line: Option<(Color, f64)>) {
        self.round_rect(cx - r, cy - r, 2.0 * r, 2.0 * r, r, fill, line);
    }
    /// A straight line.
    pub fn line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, c: Color, width: f64) {
        self.stroke(c, width);
        self.op(&format!("1 J {} {} m {} {} l S 0 J", n(x0), n(y0), n(x1), n(y1)));
    }
    /// A polyline (open or closed).
    pub fn polyline(&mut self, pts: &[(f64, f64)], closed: bool, c: Color, width: f64) {
        let Some((first, rest)) = pts.split_first() else { return };
        self.stroke(c, width);
        let mut s = format!("1 J 1 j {} {} m", n(first.0), n(first.1));
        for p in rest {
            let _ = write!(s, " {} {} l", n(p.0), n(p.1));
        }
        s.push_str(if closed { " h S 0 J 0 j" } else { " S 0 J 0 j" });
        self.op(&s);
    }
    /// An arrow from (x0, y0) to its head at (x1, y1).
    pub fn arrow(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, c: Color, width: f64, head: f64) {
        let (dx, dy) = (x1 - x0, y1 - y0);
        let l = dx.hypot(dy);
        if l < 1e-6 {
            return;
        }
        let (ux, uy) = (dx / l, dy / l);
        let (bx, by) = (x1 - ux * head, y1 - uy * head);
        self.line(x0, y0, bx, by, c, width);
        self.fill(c);
        self.op(&format!(
            "{} {} m {} {} l {} {} l h f",
            n(x1),
            n(y1),
            n(bx - uy * head * 0.45),
            n(by + ux * head * 0.45),
            n(bx + uy * head * 0.45),
            n(by - ux * head * 0.45)
        ));
    }
    /// Text with its baseline starting at (x, y).
    pub fn text(&mut self, x: f64, y: f64, s: &str, font: Font, size: f64, c: Color) {
        self.fill(c);
        self.op(&format!("BT /{} {} Tf {} {} Td {} Tj ET", font.name(), n(size), n(x), n(y), pdf_string(s)));
    }
    /// Text centred on x.
    pub fn text_center(&mut self, x: f64, y: f64, s: &str, font: Font, size: f64, c: Color) {
        let w = text_width(s, font, size);
        self.text(x - w / 2.0, y, s, font, size, c);
    }
    /// Text ending at x.
    pub fn text_right(&mut self, x: f64, y: f64, s: &str, font: Font, size: f64, c: Color) {
        let w = text_width(s, font, size);
        self.text(x - w, y, s, font, size, c);
    }
    /// A wrapped paragraph from its first baseline down; returns the y below it.
    pub fn paragraph(&mut self, x: f64, y: f64, width: f64, s: &str, font: Font, size: f64, leading: f64, c: Color) -> f64 {
        let mut y = y;
        for line in wrap(s, font, size, width) {
            self.text(x, y, &line, font, size, c);
            y -= leading;
        }
        y
    }
}

impl Document {
    pub fn new(title: &str) -> Document {
        Document { title: title.to_string(), ..Default::default() }
    }
    /// Adds a page and returns it to draw on.
    pub fn page(&mut self) -> &mut Page {
        self.pages.push(Page::default());
        let i = self.pages.len() - 1;
        &mut self.pages[i]
    }
    pub fn pages(&self) -> usize {
        self.pages.len()
    }
    pub fn page_mut(&mut self, i: usize) -> Option<&mut Page> {
        self.pages.get_mut(i)
    }
    /// Places a JPEG of `w × h` pixels on page `page` into the box (x, y, bw, bh).
    pub fn image(&mut self, page: usize, jpeg: Vec<u8>, w: u32, h: u32, x: f64, y: f64, bw: f64, bh: f64) {
        let id = self.images.len();
        self.images.push(Image { w, h, jpeg });
        if let Some(p) = self.pages.get_mut(page) {
            p.images.push(id);
            p.op(&format!("q {} 0 0 {} {} {} cm /Im{id} Do Q", n(bw), n(bh), n(x), n(y)));
        }
    }

    /// The PDF file.
    pub fn finish(&self) -> Vec<u8> {
        // objects: 1 catalog, 2 pages, 3 font regular, 4 font bold, 5 info, then per image one,
        // then per page its page object and its content
        let mut objects: Vec<Vec<u8>> = Vec::new();
        let first_image = 6;
        let first_page = first_image + self.images.len();
        let kids: Vec<String> = (0..self.pages.len()).map(|i| format!("{} 0 R", first_page + 2 * i)).collect();
        objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
        objects.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), self.pages.len()).into_bytes());
        objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec());
        objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".to_vec());
        objects.push(format!("<< /Title {} /Producer (SolidCraft) >>", pdf_string(&self.title)).into_bytes());
        for im in &self.images {
            let mut o = format!(
                "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
                im.w,
                im.h,
                im.jpeg.len()
            )
            .into_bytes();
            o.extend_from_slice(&im.jpeg);
            o.extend_from_slice(b"\nendstream");
            objects.push(o);
        }
        for (i, p) in self.pages.iter().enumerate() {
            let xobjects: String = p.images.iter().map(|id| format!("/Im{id} {} 0 R ", first_image + id)).collect();
            objects.push(
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> /XObject << {xobjects}>> >> /Contents {} 0 R >>",
                    n(A4.0),
                    n(A4.1),
                    first_page + 2 * i + 1
                )
                .into_bytes(),
            );
            let mut o = format!("<< /Length {} >>\nstream\n", p.ops.len()).into_bytes();
            o.extend_from_slice(p.ops.as_bytes());
            o.extend_from_slice(b"\nendstream");
            objects.push(o);
        }
        let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
        let mut offsets = Vec::with_capacity(objects.len());
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(o);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
        for off in offsets {
            let _ = writeln!(tail, "{off:010} 00000 n ");
        }
        let _ = write!(tail, "trailer\n<< /Size {} /Root 1 0 R /Info 5 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1);
        out.extend_from_slice(tail.as_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_is_a_pdf_with_its_pages() {
        let mut d = Document::new("Einrichtblatt Ø12");
        let p = d.page();
        p.text(40.0, 800.0, "Primärebene ±0,05 – Ø12 → X", Font::Bold, 12.0, Color(0.0, 0.0, 0.0));
        p.circle(100.0, 100.0, 10.0, Some(Color(1.0, 0.5, 0.0)), None);
        d.page();
        let pdf = d.finish();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.starts_with("%PDF-1.4") && text.trim_end().ends_with("%%EOF"));
        assert!(text.contains("/Count 2"));
        // umlauts and signs in WinAnsi, the arrow as ASCII
        assert!(text.contains("Prim\\344rebene"), "{text}");
        assert_eq!(win_ansi("Ø±°–→"), vec![0xD8, 0xB1, 0xB0, 0x96, b'-', b'>']);
        // the cross-reference table points at the objects
        let xref = text.find("xref").unwrap();
        assert!(text[xref..].contains("0000000015 00000 n"));
    }

    #[test]
    fn text_wraps_at_the_width() {
        let lines = wrap("Mit dem Taster die Oberseite an vier Punkten antasten", Font::Regular, 10.0, 120.0);
        assert!(lines.len() >= 2);
        assert!(lines.iter().all(|l| text_width(l, Font::Regular, 10.0) <= 120.0 || !l.contains(' ')));
    }
}
