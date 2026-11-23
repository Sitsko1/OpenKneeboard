//! Port of the C++ `SpriteAtlas` (`src/lib/SpriteAtlas.cpp`), including the vendored
//! `rbp::MaxRectsBinPack` (Jukka Jylanki, public domain; `RectBestShortSideFit`, no rotation).
//! Free-list iteration order and swap-removal are kept so ties break identically.

use okb_shm::wire::{PixelPoint, PixelRect, PixelSize};

/// C++ `MaxViewRenderSize`.
pub const MAX_VIEW_RENDER_SIZE: PixelSize = PixelSize {
    width: 2048,
    height: 2048,
};

/// `D3D11_REQ_TEXTURE2D_U_OR_V_DIMENSION`.
const MAX_DIMENSION: u32 = 16384;
const INCREMENT: u32 = 256;

/// C++ throws `std::runtime_error("Can not fit SpriteAtlas in resource limits")`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtlasError {
    DoesNotFit,
}

/// C++ `rbp::Rect`.
#[derive(Clone, Copy, Default)]
struct Rect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

fn is_contained_in(a: Rect, b: Rect) -> bool {
    a.x >= b.x && a.y >= b.y && a.x + a.width <= b.x + b.width && a.y + a.height <= b.y + b.height
}

struct MaxRectsBinPack {
    free: Vec<Rect>,
    new_free: Vec<Rect>,
    new_free_last_size: usize,
}

impl MaxRectsBinPack {
    fn new(width: i32, height: i32) -> Self {
        Self {
            free: vec![Rect {
                x: 0,
                y: 0,
                width,
                height,
            }],
            new_free: Vec::new(),
            new_free_last_size: 0,
        }
    }

    /// `Insert(..., RectBestShortSideFit)`: a zero-height result means it did not fit.
    fn insert(&mut self, width: i32, height: i32) -> Rect {
        let node = self.find_best_short_side_fit(width, height);
        if node.height != 0 {
            self.place_rect(node);
        }
        node
    }

    fn find_best_short_side_fit(&self, width: i32, height: i32) -> Rect {
        let mut best = Rect::default();
        let mut best_short = i32::MAX;
        let mut best_long = i32::MAX;
        for f in &self.free {
            if f.width >= width && f.height >= height {
                let horiz = (f.width - width).abs();
                let vert = (f.height - height).abs();
                let short = horiz.min(vert);
                let long = horiz.max(vert);
                if short < best_short || (short == best_short && long < best_long) {
                    best = Rect {
                        x: f.x,
                        y: f.y,
                        width,
                        height,
                    };
                    best_short = short;
                    best_long = long;
                }
            }
        }
        best
    }

    fn place_rect(&mut self, node: Rect) {
        let mut i = 0;
        while i < self.free.len() {
            if self.split_free_node(self.free[i], node) {
                self.free.swap_remove(i);
            } else {
                i += 1;
            }
        }
        self.prune_free_list();
    }

    fn split_free_node(&mut self, free: Rect, used: Rect) -> bool {
        if used.x >= free.x + free.width
            || used.x + used.width <= free.x
            || used.y >= free.y + free.height
            || used.y + used.height <= free.y
        {
            return false;
        }
        self.new_free_last_size = self.new_free.len();

        if used.x < free.x + free.width && used.x + used.width > free.x {
            if used.y > free.y && used.y < free.y + free.height {
                self.insert_new_free_rectangle(Rect {
                    height: used.y - free.y,
                    ..free
                });
            }
            if used.y + used.height < free.y + free.height {
                self.insert_new_free_rectangle(Rect {
                    y: used.y + used.height,
                    height: free.y + free.height - (used.y + used.height),
                    ..free
                });
            }
        }
        if used.y < free.y + free.height && used.y + used.height > free.y {
            if used.x > free.x && used.x < free.x + free.width {
                self.insert_new_free_rectangle(Rect {
                    width: used.x - free.x,
                    ..free
                });
            }
            if used.x + used.width < free.x + free.width {
                self.insert_new_free_rectangle(Rect {
                    x: used.x + used.width,
                    width: free.x + free.width - (used.x + used.width),
                    ..free
                });
            }
        }
        true
    }

    fn insert_new_free_rectangle(&mut self, rect: Rect) {
        let mut i = 0;
        while i < self.new_free_last_size {
            if is_contained_in(rect, self.new_free[i]) {
                return;
            }
            if is_contained_in(self.new_free[i], rect) {
                // Same swap dance as C++, preserving the old-vs-newest ordering.
                self.new_free_last_size -= 1;
                self.new_free[i] = self.new_free[self.new_free_last_size];
                let last = *self.new_free.last().expect("non-empty");
                self.new_free[self.new_free_last_size] = last;
                self.new_free.pop();
            } else {
                i += 1;
            }
        }
        self.new_free.push(rect);
    }

    fn prune_free_list(&mut self) {
        for f in &self.free {
            let mut j = 0;
            while j < self.new_free.len() {
                if is_contained_in(self.new_free[j], *f) {
                    self.new_free.swap_remove(j);
                } else {
                    j += 1;
                }
            }
        }
        self.free.append(&mut self.new_free);
    }
}

/// C++ `SpriteAtlas`.
#[derive(Clone, Debug)]
pub struct SpriteAtlas {
    size: PixelSize,
    rects: Vec<PixelRect>,
}

impl SpriteAtlas {
    /// Panics if `sprites` is empty (C++: `ranges::max` of an empty range is UB).
    pub fn new(sprites: &[PixelSize]) -> Result<Self, AtlasError> {
        // "+1" to allow an empty space between each one
        let pad = |v: u32| v.wrapping_add(1);
        let max_width = pad(sprites.iter().map(|s| s.width).max().expect("no sprites"));
        let max_height = pad(sprites.iter().map(|s| s.height).max().expect("no sprites"));
        let total_area = sprites.iter().fold(0u64, |acc, s| {
            acc + u64::from(pad(s.width).wrapping_mul(pad(s.height)))
        });
        let estimated_edge = ((total_area as f64).sqrt().ceil() as u32).max(64);

        let mut width = max_width.max(estimated_edge).next_power_of_two();
        let mut height = max_height.max(estimated_edge).next_power_of_two();

        let rects = loop {
            let mut bp = MaxRectsBinPack::new(width as i32, height as i32);
            let mut rects = Vec::with_capacity(sprites.len());
            let mut failed = false;
            for s in sprites {
                let dest = bp.insert(pad(s.width) as i32, pad(s.height) as i32);
                if dest.width == 0 || dest.height == 0 {
                    failed = true;
                    break;
                }
                rects.push(PixelRect {
                    offset: PixelPoint {
                        x: dest.x as u32,
                        y: dest.y as u32,
                    },
                    size: PixelSize {
                        width: (dest.width - 1) as u32,
                        height: (dest.height - 1) as u32,
                    },
                    ..PixelRect::default()
                });
            }
            if !failed {
                break rects;
            }
            if width <= height {
                width += INCREMENT;
            } else {
                height += INCREMENT;
            }
            if width > MAX_DIMENSION {
                width -= INCREMENT;
                height += INCREMENT;
            }
            if height > MAX_DIMENSION {
                height -= INCREMENT;
                width += INCREMENT;
                if width > MAX_DIMENSION {
                    return Err(AtlasError::DoesNotFit);
                }
            }
        };

        // C++ `std::ranges::max` returns the first maximum; only the value matters.
        let size = PixelSize {
            width: rects
                .iter()
                .map(|r| r.offset.x + r.size.width)
                .max()
                .unwrap(),
            height: rects
                .iter()
                .map(|r| r.offset.y + r.size.height)
                .max()
                .unwrap(),
        };
        Ok(Self { size, rects })
    }

    pub fn size(&self) -> PixelSize {
        self.size
    }

    /// Panics if out of range, like C++ `.at()`.
    pub fn rect(&self, index: usize) -> PixelRect {
        self.rects[index]
    }
}

/// `Size<uint32_t>::ScaledToFit(container, ShrinkOrGrow)`: float math, then `lround`.
pub fn scaled_to_fit(size: PixelSize, container: PixelSize) -> PixelSize {
    let scale_x = container.width as f32 / size.width as f32;
    let scale_y = container.height as f32 / size.height as f32;
    let scale = scale_x.min(scale_y);
    PixelSize {
        width: (size.width as f32 * scale).round() as u32,
        height: (size.height as f32 * scale).round() as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sz(width: u32, height: u32) -> PixelSize {
        PixelSize { width, height }
    }

    #[test]
    fn single_sprite_gets_padding_column_and_row() {
        let atlas = SpriteAtlas::new(&[sz(100, 50)]).unwrap();
        let r = atlas.rect(0);
        assert_eq!((r.offset.x, r.offset.y), (0, 0));
        assert_eq!(r.size, sz(100, 50));
        assert_eq!(atlas.size(), sz(100, 50));
    }

    #[test]
    fn atlas_edge_is_at_least_64_and_power_of_two_before_trim() {
        // Two 10x10 sprites: the bin is 64x64 (not 11 or 16), so the second lands at a free
        // rect inside it, not in a tight 22-wide atlas.
        let atlas = SpriteAtlas::new(&[sz(10, 10), sz(10, 10)]).unwrap();
        assert_eq!(atlas.rect(0).offset, PixelPoint { x: 0, y: 0 });
        // First free rect after splitting 64x64 by an 11x11 node is the right strip (x=11,
        // 53x64): its short-side fit beats the bottom strip's tie via the long side.
        assert_eq!(atlas.rect(1).offset, PixelPoint { x: 0, y: 11 });
        assert_eq!(atlas.size(), sz(10, 21));
    }

    #[test]
    fn grows_by_256_until_everything_fits() {
        // 2x 1030x1030 (1031 padded): the initial bin is 2048x2048, where only one fits per
        // axis (2062 > 2048), so the second fails; width grows to 2304 and then it fits.
        let atlas = SpriteAtlas::new(&[sz(1030, 1030), sz(1030, 1030)]).unwrap();
        assert_eq!(atlas.rect(0).offset, PixelPoint { x: 0, y: 0 });
        assert_eq!(atlas.rect(1).offset, PixelPoint { x: 1031, y: 0 });
        assert_eq!(atlas.size(), sz(2061, 1030));
    }

    #[test]
    fn does_not_fit_beyond_16384() {
        // C++ never rejects a lone oversized sprite (the initial bin may exceed 16384); it
        // throws only when packing fails in a bin that is already past the limit.
        assert_eq!(
            SpriteAtlas::new(&[sz(17000, 17000), sz(17000, 17000)]).unwrap_err(),
            AtlasError::DoesNotFit
        );
        assert!(SpriteAtlas::new(&[sz(16384, 16384)]).is_ok());
    }

    #[test]
    fn scaled_to_fit_rounds_like_cpp() {
        assert_eq!(
            scaled_to_fit(sz(4096, 1024), MAX_VIEW_RENDER_SIZE),
            sz(2048, 512)
        );
        assert_eq!(
            scaled_to_fit(sz(100, 50), MAX_VIEW_RENDER_SIZE),
            sz(2048, 1024)
        );
        // 2048/3 * 2 = 1365.33 -> 1365; 1000*(2048/1500)=1365.33; lround rounds half away.
        assert_eq!(
            scaled_to_fit(sz(1500, 1000), MAX_VIEW_RENDER_SIZE),
            sz(2048, 1365)
        );
        assert_eq!(scaled_to_fit(sz(4, 1), sz(10, 10)), sz(10, 3)); // 2.5 -> 3
    }
}
