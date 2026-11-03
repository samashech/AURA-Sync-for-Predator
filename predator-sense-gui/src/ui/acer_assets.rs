//! Optional artwork from the official Windows PredatorSense app.
//!
//! None of it is in this repository. `tools/import_windows_assets.py` copies
//! it from the user's own Windows installation into
//! `~/.local/share/predator-sense/acer-assets/` (SVGs pre-rendered to 2x
//! PNGs), for personal use on that machine. Every caller treats a missing
//! file as "not imported" and keeps its own drawn fallback, so a machine
//! without the import looks exactly like it did before.

use gtk4 as gtk;
use gtk4::prelude::*;
use std::path::PathBuf;
use std::sync::OnceLock;

fn dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = dirs::data_dir()?.join("predator-sense").join("acer-assets");
        dir.is_dir().then_some(dir)
    })
    .as_ref()
}

/// Absolute path of `rel` inside the imported asset folder, if it is there.
pub fn file(rel: &str) -> Option<String> {
    let path = dir()?.join(rel);
    path.is_file().then(|| path.to_string_lossy().into_owned())
}

/// Whether the import has been run at all.
pub fn available() -> bool {
    dir().is_some()
}

/// A picture of `rel` at `width` x `height` - the size the Windows layout
/// gives that piece, in logical pixels. Kept at its own aspect ratio, so a
/// container that allocates more room (a dial in a wider column) centres it
/// instead of stretching it.
pub fn picture(rel: &str, width: i32, height: i32) -> Option<gtk::Picture> {
    let pic = gtk::Picture::for_filename(file(rel)?);
    pic.set_can_shrink(true);
    pic.set_content_fit(gtk::ContentFit::Contain);
    pic.set_size_request(width, height);
    pic.set_halign(gtk::Align::Center);
    pic.set_valign(gtk::Align::Center);
    Some(pic)
}

/// Header-style flat button showing `rel` as its icon.
pub fn icon_button(rel: &str, size: i32) -> Option<gtk::Button> {
    let pic = picture(rel, size, size)?;
    let button = gtk::Button::new();
    button.set_child(Some(&pic));
    button.add_css_class("acer-icon-btn");
    Some(button)
}
