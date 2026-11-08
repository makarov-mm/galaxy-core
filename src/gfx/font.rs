//! Bitmap font atlas: `DejaVu Sans Mono`, ASCII 32..=126 in a 16-column grid of
//! 9x18 cells, 8-bit coverage stored as raw row-major bytes.

pub const FIRST_CHAR: char = ' ';
pub const LAST_CHAR: char = '~';
pub const COLUMNS: usize = 16;
pub const GLYPH_W: usize = 9;
pub const GLYPH_H: usize = 18;
pub const ATLAS_W: usize = COLUMNS * GLYPH_W;
pub const ATLAS_H: usize = 6 * GLYPH_H;

/// The declared length makes a wrong-sized atlas file a compile error.
pub static ATLAS: &[u8; ATLAS_W * ATLAS_H] = include_bytes!("font_atlas.r8");
