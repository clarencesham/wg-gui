use gtk::prelude::*;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets"]
pub struct Assets;

// GtkWidget::destroy is unsafe in gtk-rs because Rust references to the widget
// must not be used afterwards; all call sites only destroy widgets they are
// done with.
pub fn destroy<T: IsA<gtk::Widget>>(widget: &T) {
    unsafe {
        widget.destroy();
    }
}

pub fn asset(path: &str) -> Option<Vec<u8>> {
    Assets::get(path).map(|f| f.data.into_owned())
}

pub fn asset_string(path: &str) -> Option<String> {
    Assets::get(path).map(|f| String::from_utf8_lossy(&f.data).into_owned())
}

pub fn pixbuf_from_bytes(data: &[u8]) -> Result<gtk::gdk_pixbuf::Pixbuf, String> {
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    loader.write(data).map_err(|e| e.to_string())?;
    loader.close().map_err(|e| e.to_string())?;
    loader
        .pixbuf()
        .ok_or_else(|| "pixbuf loader returned no pixbuf".to_string())
}

pub fn load_embedded_pixbuf(path: &str) -> Result<gtk::gdk_pixbuf::Pixbuf, String> {
    let data = asset(path).ok_or_else(|| format!("asset not found: {}", path))?;
    pixbuf_from_bytes(&data)
}

pub fn load_embedded_image(path: &str) -> Result<gtk::Image, String> {
    let pixbuf = load_embedded_pixbuf(path)?;
    Ok(gtk::Image::from_pixbuf(Some(&pixbuf)))
}
