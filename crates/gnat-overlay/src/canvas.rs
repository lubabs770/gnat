//! Software drawing surface shared by every window this crate opens.

/// Straight (non-premultiplied) colour. [`Canvas`] premultiplies on write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const TRANSPARENT: Rgba = Rgba::new(0, 0, 0, 0);

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
        Self::new(r, g, b, 0xFF)
    }

    /// The same colour at a different alpha.
    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }
}

/// The surface to paint, handed to the draw callback once per frame.
pub struct Canvas<'a> {
    pub width: u32,
    pub height: u32,
    /// ARGB8888, little-endian, **premultiplied** alpha.
    pub pixels: &'a mut [u8],
    /// Milliseconds since the overlay was created.
    pub time_ms: u64,
}

impl Canvas<'_> {
    /// Fill with fully transparent pixels. An overlay that does not do this
    /// first will show whatever the last frame left in the buffer.
    pub fn clear(&mut self) {
        self.pixels.fill(0);
    }

    /// Write one pixel. Out-of-bounds coordinates are dropped rather than
    /// wrapping to the next row.
    pub fn put(&mut self, x: i32, y: i32, c: Rgba) {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        let m = |v: u8| ((v as u16 * c.a as u16) / 255) as u8;
        // wl_shm ARGB8888 is little-endian, so the byte order is B, G, R, A,
        // and the colour channels must be premultiplied by alpha.
        self.pixels[i] = m(c.b);
        self.pixels[i + 1] = m(c.g);
        self.pixels[i + 2] = m(c.r);
        self.pixels[i + 3] = c.a;
    }

    /// Filled axis-aligned rectangle, clipped to the canvas.
    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgba) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.put(xx, yy, c);
            }
        }
    }

    /// Filled circle, clipped to the canvas.
    pub fn disc(&mut self, cx: i32, cy: i32, radius: i32, c: Rgba) {
        let r2 = radius * radius;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy <= r2 {
                    self.put(cx + dx, cy + dy, c);
                }
            }
        }
    }

    /// Filled ellipse with semi-axes `rx`/`ry`, rotated by `angle` radians.
    ///
    /// Iterates the *bounding box* of the rotated shape and tests each pixel
    /// against the un-rotated ellipse equation, which keeps the edge clean at
    /// any angle without needing a scanline rasteriser.
    pub fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, angle: f32, c: Rgba) {
        if rx <= 0.0 || ry <= 0.0 {
            return;
        }
        let (sin, cos) = angle.sin_cos();
        let half_w = (rx * cos).abs() + (ry * sin).abs();
        let half_h = (rx * sin).abs() + (ry * cos).abs();

        let x0 = (cx - half_w).floor() as i32;
        let x1 = (cx + half_w).ceil() as i32;
        let y0 = (cy - half_h).floor() as i32;
        let y1 = (cy + half_h).ceil() as i32;

        for y in y0..=y1 {
            for x in x0..=x1 {
                let (dx, dy) = (x as f32 - cx, y as f32 - cy);
                // Rotate the sample back into the ellipse's own frame.
                let u = dx * cos + dy * sin;
                let v = -dx * sin + dy * cos;
                if (u / rx).powi(2) + (v / ry).powi(2) <= 1.0 {
                    self.put(x, y, c);
                }
            }
        }
    }

    /// Line of the given width, drawn as a run of discs. Widths under 2 fall
    /// back to single pixels, which is what legs and antennae want.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, c: Rgba) {
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len = (dx * dx + dy * dy).sqrt();
        let steps = len.ceil().max(1.0) as i32;
        let r = (width / 2.0).round() as i32;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let (x, y) = (x0 + dx * t, y0 + dy * t);
            if r < 1 {
                self.put(x.round() as i32, y.round() as i32, c);
            } else {
                self.disc(x.round() as i32, y.round() as i32, r, c);
            }
        }
    }
}
