use serde::{Deserialize, Serialize};

/// A rectangle in physical pixels of the Windows virtual desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    pub fn area(&self) -> i64 {
        self.w.max(0) as i64 * self.h.max(0) as i64
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// The point inside this rectangle closest to (`x`, `y`).
    pub fn clamp(&self, x: i32, y: i32) -> (i32, i32) {
        (x.clamp(self.x, (self.right() - 1).max(self.x)), y.clamp(self.y, (self.bottom() - 1).max(self.y)))
    }

    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (right > x && bottom > y).then(|| Rect::new(x, y, right - x, bottom - y))
    }

    /// Share of this rectangle's area that lies inside `other`, from 0 to 1.
    pub fn fraction_inside(&self, other: &Rect) -> f64 {
        match (self.area(), self.intersection(other)) {
            (0, _) | (_, None) => 0.0,
            (area, Some(overlap)) => overlap.area() as f64 / area as f64,
        }
    }

    /// Translates a rectangle given relative to `self` (e.g. display coordinates) to desktop coordinates.
    pub fn offset(&self, inner: Rect) -> Rect {
        Rect::new(self.x + inner.x, self.y + inner.y, inner.w, inner.h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_math() {
        let display = Rect::new(1920, -1079, 1920, 1080);
        assert_eq!(Rect::new(1920, -1079, 800, 600).fraction_inside(&display), 1.0);
        assert_eq!(Rect::new(0, 0, 800, 600).fraction_inside(&display), 0.0);
        assert_eq!(Rect::new(1520, -1079, 800, 600).fraction_inside(&display), 0.5);
        assert!(display.contains(1920, -1079) && !display.contains(3840, 0));
        assert_eq!(display.offset(Rect::new(10, 20, 30, 40)), Rect::new(1930, -1059, 30, 40));
    }
}
