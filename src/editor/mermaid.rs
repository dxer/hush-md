//! ```mermaid fences drawn in the page. The diagram source stays the only
//! copy on disk — rendering is a view, never a saved artifact.
//!
//! `mermaid-rs-renderer` turns the source into SVG without a browser; the
//! SVG is then rasterized by the resvg/usvg that GPUI already links. GPUI's
//! own `svg()` element paints a single-colour alpha mask, so a colour
//! diagram has to go through `img(ImageSource::Render(..))` instead.
//!
//! Rasters are cached in memory, keyed by the diagram text and the scale:
//! they are derived data, so nothing is written into the vault.

use gpui::{px, size, Hsla, Pixels, RenderImage, Size};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// The block's language (as `lang::resolve` stores it) that means "draw this
/// as a diagram".
pub const LANG: &str = "Mermaid";

/// A rasterized diagram, sized in logical pixels.
#[derive(Clone)]
pub struct Raster {
    pub image: Arc<RenderImage>,
    /// Logical size, for laying the diagram out on the page.
    pub size: Size<Pixels>,
    /// Device-pixel size of the raster itself (logical size × scale).
    #[cfg_attr(not(test), allow(dead_code))]
    pub pixels: (u32, u32),
}

/// Lubricant for the reuse check: a diagram whose source is unchanged keeps
/// its raster across rebuilds.
fn key(src: &str, scale: f32) -> (u64, u32) {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut h);
    h.finish().hash(&mut h);
    // Scale to 1/1000, so tiny f32 noise still hits the same raster.
    (h.finish(), (scale * 1000.).round() as u32)
}

thread_local! {
    static CACHE: RefCell<HashMap<(u64, u32), Option<Raster>>> = RefCell::new(HashMap::new());
}

/// The diagram for `src` at `scale`, or `None` when it does not parse.
/// Failures are cached too: a broken diagram is not retried every frame.
pub fn raster(src: &str, theme: &mermaid_rs_renderer::Theme, scale: f32) -> Option<Raster> {
    let k = key(src, scale);
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&k).cloned()) {
        return hit;
    }
    let made = render(src, theme, scale);
    CACHE.with(|c| c.borrow_mut().insert(k, made.clone()));
    made
}

fn render(src: &str, theme: &mermaid_rs_renderer::Theme, scale: f32) -> Option<Raster> {
    let options = mermaid_rs_renderer::RenderOptions { theme: theme.clone(), layout: Default::default() };
    let svg = mermaid_rs_renderer::render_with_options(src, options).ok()?;
    // usvg's default options carry no fonts, so every label would vanish;
    // GPUI's own SVG renderer loads the system ones the same way.
    let tree = usvg::Tree::from_str(&svg, svg_options()).ok()?;
    let (w, h) = (tree.size().width(), tree.size().height());
    if w <= 0. || h <= 0. {
        return None;
    }
    let scale = scale.max(1.);
    let pw = (w * scale).ceil() as u32;
    let ph = (h * scale).ceil() as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(pw, ph)?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let mut buf = image::RgbaImage::from_raw(pw, ph, pixmap.take())?;
    // GPUI images are BGRA.
    for px in buf.pixels_mut() {
        px.0.swap(0, 2);
    }
    Some(Raster { image: Arc::new(RenderImage::new(vec![image::Frame::new(buf)])), size: size(px(w), px(h)), pixels: (pw, ph) })
}

/// usvg options with the machine's fonts, built once (the database is large).
fn svg_options() -> &'static usvg::Options<'static> {
    static OPTIONS: std::sync::OnceLock<usvg::Options<'static>> = std::sync::OnceLock::new();
    OPTIONS.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        usvg::Options { fontdb: std::sync::Arc::new(db), ..Default::default() }
    })
}

/// The renderer's palette, from the document theme's semantic tokens.
pub fn theme() -> mermaid_rs_renderer::Theme {
    use crate::editor_theme::Theme;
    let hex = |c: Hsla| {
        let c: gpui::Rgba = c.into();
        format!("#{:02x}{:02x}{:02x}", (c.r * 255.).round() as u8, (c.g * 255.).round() as u8, (c.b * 255.).round() as u8)
    };
    let mut t = mermaid_rs_renderer::Theme::modern();
    // The page's own colour shows through, so the diagram sits on the note.
    t.background = "transparent".into();
    // The note's own face first; the CJK fallbacks matter because usvg keeps
    // only what the family list names.
    t.font_family = format!("{}, PingFang SC, Noto Sans CJK SC, Microsoft YaHei, sans-serif", crate::editor_theme::body_font());
    t.primary_color = hex(Theme::bg_overlay());
    t.primary_border_color = hex(Theme::line_border());
    t.primary_text_color = hex(Theme::text());
    t.text_color = hex(Theme::text());
    t.line_color = hex(Theme::text_caption());
    t.edge_label_background = hex(Theme::bg_body());
    t.cluster_background = hex(Theme::bg_overlay());
    t.cluster_border = hex(Theme::line_border());
    t
}

/// Drop cached rasters (the theme changed).
pub fn clear() {
    CACHE.with(|c| c.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn a_diagram_renders_and_an_unparsable_one_does_not() {
        let t = theme();
        let r = raster("flowchart LR; A --> B", &t, 1.).expect("a simple flowchart should render");
        assert!(f32::from(r.size.width) > 0. && f32::from(r.size.height) > 0.);
        // The size is logical; the image itself is made at device pixels, so
        // a 2x raster is twice as wide as the 1x one.
        let r2 = raster("flowchart LR; A --> B", &t, 2.).expect("2x");
        assert_eq!(r.size, r2.size);
        assert!((r.pixels.0 as f32 * 2. - r2.pixels.0 as f32).abs() <= 2., "{:?} vs {:?}", r.pixels, r2.pixels);
        // Prose is not a diagram; the block falls back to its code.
        assert!(raster("not a diagram at all", &t, 1.).is_none());
    }

    #[test]
    fn rasters_are_reused_and_cleared() {
        let t = theme();
        let a = raster("sequenceDiagram\n  A->>B: hi", &t, 1.).unwrap();
        let b = raster("sequenceDiagram\n  A->>B: hi", &t, 1.).unwrap();
        assert!(Arc::ptr_eq(&a.image, &b.image), "the same source keeps its raster");
        clear();
        let c = raster("sequenceDiagram\n  A->>B: hi", &t, 1.).unwrap();
        assert!(!Arc::ptr_eq(&a.image, &c.image), "clear() drops the cache");
    }
}
