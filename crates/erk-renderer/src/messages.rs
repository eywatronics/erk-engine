//! Everything that crosses between the shell and the renderer.
//!
//! These types become IPC messages when the renderer moves into its own
//! process (M3), so they hold only plain owned data: strings, integers and
//! byte vectors, nothing shared, nothing borrowed, no engine types. Making
//! them serializable should then be one derive line. CI keeps this file to
//! prelude types only (.github/scripts/check-renderer-surface.sh).

/// Messages from the shell to the renderer.
#[derive(Debug)]
pub enum ToRenderer {
    /// Show this document. The shell reads files, not the renderer: in the
    /// target architecture the renderer is sandboxed and has no disk access.
    Load { html: String },
    /// The viewport is now `width` × `height` device pixels.
    Resize { width: u16, height: u16 },
    /// Stop the renderer thread.
    Shutdown,
}

/// Messages from the renderer to the shell.
pub enum FromRenderer {
    /// A newly painted frame of the current document at the current size.
    Frame(Frame),
}

/// A rendered page.
pub struct Frame {
    width: u16,
    height: u16,
    rgba: Vec<u8>,
    display_list: String,
}

impl Frame {
    /// `rgba` holds `width` × `height` pixels, four bytes each.
    pub(crate) fn new(width: u16, height: u16, rgba: Vec<u8>, display_list: String) -> Self {
        Self {
            width,
            height,
            rgba,
            display_list,
        }
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    /// Pixels as premultiplied RGBA8, row by row.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// The display list the frame was painted from, as text.
    pub fn display_list(&self) -> &str {
        &self.display_list
    }
}
