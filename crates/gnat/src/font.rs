//! A 3x5 bitmap font, because the brain view needs to name what you clicked and
//! there is no text on a raw pixel buffer.
//!
//! Uppercase only. Three pixels wide is the smallest a letter can be and stay
//! readable, and at 3x scale it is perfectly legible next to a point cloud.
//! Anything a glyph is missing renders as a blank, never a panic.

use gnat_overlay::{Canvas, Rgba};

pub const GLYPH_W: i32 = 3;
pub const GLYPH_H: i32 = 5;

/// Rows top to bottom; `#` is on.
fn glyph(c: char) -> Option<[&'static str; 5]> {
    Some(match c.to_ascii_uppercase() {
        'A' => [".#.", "#.#", "###", "#.#", "#.#"],
        'B' => ["##.", "#.#", "##.", "#.#", "##."],
        'C' => [".##", "#..", "#..", "#..", ".##"],
        'D' => ["##.", "#.#", "#.#", "#.#", "##."],
        'E' => ["###", "#..", "##.", "#..", "###"],
        'F' => ["###", "#..", "##.", "#..", "#.."],
        'G' => [".##", "#..", "#.#", "#.#", ".##"],
        'H' => ["#.#", "#.#", "###", "#.#", "#.#"],
        'I' => ["###", ".#.", ".#.", ".#.", "###"],
        'J' => ["..#", "..#", "..#", "#.#", ".#."],
        'K' => ["#.#", "#.#", "##.", "#.#", "#.#"],
        'L' => ["#..", "#..", "#..", "#..", "###"],
        'M' => ["#.#", "###", "###", "#.#", "#.#"],
        'N' => ["#.#", "###", "###", "###", "#.#"],
        'O' => [".#.", "#.#", "#.#", "#.#", ".#."],
        'P' => ["##.", "#.#", "##.", "#..", "#.."],
        'Q' => [".#.", "#.#", "#.#", "##.", ".##"],
        'R' => ["##.", "#.#", "##.", "#.#", "#.#"],
        'S' => [".##", "#..", ".#.", "..#", "##."],
        'T' => ["###", ".#.", ".#.", ".#.", ".#."],
        'U' => ["#.#", "#.#", "#.#", "#.#", ".##"],
        'V' => ["#.#", "#.#", "#.#", "#.#", ".#."],
        'W' => ["#.#", "#.#", "###", "###", "#.#"],
        'X' => ["#.#", "#.#", ".#.", "#.#", "#.#"],
        'Y' => ["#.#", "#.#", ".#.", ".#.", ".#."],
        'Z' => ["###", "..#", ".#.", "#..", "###"],
        '0' => [".#.", "#.#", "#.#", "#.#", ".#."],
        '1' => [".#.", "##.", ".#.", ".#.", "###"],
        '2' => ["##.", "..#", ".#.", "#..", "###"],
        '3' => ["##.", "..#", ".#.", "..#", "##."],
        '4' => ["#.#", "#.#", "###", "..#", "..#"],
        '5' => ["###", "#..", "##.", "..#", "##."],
        '6' => [".##", "#..", "###", "#.#", "###"],
        '7' => ["###", "..#", ".#.", ".#.", ".#."],
        '8' => ["###", "#.#", "###", "#.#", "###"],
        '9' => ["###", "#.#", "###", "..#", "##."],
        '/' => ["..#", "..#", ".#.", "#..", "#.."],
        '-' => ["...", "...", "###", "...", "..."],
        '.' => ["...", "...", "...", "...", ".#."],
        ',' => ["...", "...", "...", ".#.", "#.."],
        ':' => ["...", ".#.", "...", ".#.", "..."],
        '!' => [".#.", ".#.", ".#.", "...", ".#."],
        '(' => ["..#", ".#.", ".#.", ".#.", "..#"],
        ')' => ["#..", ".#.", ".#.", ".#.", "#.."],
        '+' => ["...", ".#.", "###", ".#.", "..."],
        '=' => ["...", "###", "...", "###", "..."],
        ' ' => ["...", "...", "...", "...", "..."],
        _ => return None,
    })
}

/// Width in pixels of `text` at the given scale, including inter-glyph gaps.
pub fn width(text: &str, scale: i32) -> i32 {
    let n = text.chars().count() as i32;
    if n == 0 {
        0
    } else {
        n * (GLYPH_W + 1) * scale - scale
    }
}

pub fn height(scale: i32) -> i32 {
    GLYPH_H * scale
}

/// Draw `text` with its top-left at `(x, y)`.
pub fn draw(canvas: &mut Canvas, x: i32, y: i32, text: &str, scale: i32, colour: Rgba) {
    let mut cx = x;
    for ch in text.chars() {
        if let Some(rows) = glyph(ch) {
            for (ry, row) in rows.iter().enumerate() {
                for (rx, cell) in row.chars().enumerate() {
                    if cell == '#' {
                        canvas.rect(
                            cx + rx as i32 * scale,
                            y + ry as i32 * scale,
                            scale,
                            scale,
                            colour,
                        );
                    }
                }
            }
        }
        cx += (GLYPH_W + 1) * scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_is_three_by_five() {
        for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789/-.,:!()+= ".chars() {
            let g = glyph(c).unwrap_or_else(|| panic!("{c} is missing"));
            assert_eq!(g.len(), GLYPH_H as usize, "{c} has the wrong height");
            for row in g {
                assert_eq!(row.len(), GLYPH_W as usize, "{c} row {row:?} is wrong");
                assert!(row.chars().all(|p| p == '#' || p == '.'), "{c}: {row:?}");
            }
        }
    }

    #[test]
    fn unknown_characters_are_skipped_rather_than_panicking() {
        assert!(glyph('\u{26A1}').is_none());
        let mut px = vec![0u8; 64 * 16 * 4];
        let mut canvas = Canvas {
            width: 64,
            height: 16,
            pixels: &mut px,
            time_ms: 0,
        };
        // A string of glyphs this font does not have must still advance and
        // draw nothing, not crash.
        draw(
            &mut canvas,
            0,
            0,
            "\u{26A1}\u{00B7}~",
            2,
            Rgba::opaque(255, 255, 255),
        );
        assert!(px.chunks_exact(4).all(|p| p[3] == 0));
    }

    #[test]
    fn lowercase_is_folded_to_uppercase() {
        assert_eq!(glyph('a'), glyph('A'));
    }

    #[test]
    fn width_accounts_for_the_gaps_but_not_a_trailing_one() {
        assert_eq!(width("", 2), 0);
        assert_eq!(width("A", 2), GLYPH_W * 2);
        assert_eq!(width("AB", 2), (GLYPH_W + 1) * 2 + GLYPH_W * 2);
    }

    #[test]
    fn drawing_puts_pixels_where_the_glyph_says() {
        let mut px = vec![0u8; 32 * 16 * 4];
        let mut canvas = Canvas {
            width: 32,
            height: 16,
            pixels: &mut px,
            time_ms: 0,
        };
        draw(&mut canvas, 0, 0, "L", 1, Rgba::opaque(255, 255, 255));
        let on = |x: usize, y: usize| px[(y * 32 + x) * 4 + 3] > 0;
        // 'L' is a full left column and a full bottom row.
        assert!(on(0, 0) && on(0, 4) && on(2, 4));
        assert!(!on(2, 0), "top-right of an L must be blank");
    }
}
