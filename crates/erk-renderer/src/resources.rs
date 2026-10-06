//! Resources the content asks for and the host provides (p1-contract §6).
//!
//! Erk reads no file. A document names images (`<img src>`, CSS
//! `background-image: url()`); each new URL becomes a request with an id,
//! the URL as written and its kind. The host answers with a MIME type and
//! bytes, or not at all; the page renders without what is missing. A
//! response is checked against what was asked for: an image request
//! answered with something that is not a PNG or a JPEG is refused, whatever
//! its bytes are.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use erk_dom::{Document, NodeData, local_name};
use erk_style::Styles;
use vello_cpu::Pixmap;
use vello_cpu::color::PremulRgba8;

use crate::fonts::HostFonts;
use crate::list::{DisplayItem, DisplayList, FontId, ImageId, TableUpdate};
use crate::messages::{FontCatalog, ResourceKind, ResourceRequest, ResourceResponse};

/// The largest image side Erk decodes. Larger ones are refused rather than
/// allocated: a host is trusted, its files are not necessarily sane.
const MAX_SIDE: u32 = 16_384;

/// The most pixels Erk holds for one image: an 8K screen's worth and a bit,
/// 128 MiB as RGBA. Checked from the header, before the pixels are
/// allocated, so a small file that claims a huge image costs nothing.
const MAX_PIXELS: u64 = 1 << 25;

/// Whether an image whose header claims `width` × `height` may be decoded.
fn within_budget(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(format!("the header claims {width}x{height}, out of range"));
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(format!(
            "the header claims {width}x{height}, over the pixel budget"
        ));
    }
    Ok(())
}

/// A decoded image, premultiplied, and the number the raster knows it by.
pub(crate) struct Image {
    pub(crate) id: ImageId,
    pub(crate) pixmap: Pixmap,
}

impl Image {
    pub(crate) fn size(&self) -> (u16, u16) {
        (self.pixmap.width(), self.pixmap.height())
    }

    pub(crate) fn width(&self) -> f32 {
        f32::from(self.pixmap.width())
    }

    pub(crate) fn height(&self) -> f32 {
        f32::from(self.pixmap.height())
    }
}

enum State {
    Pending,
    Ready(Arc<Image>),
    Missing,
}

/// What a request id was for.
enum Requested {
    Image(String),
    Font(String),
}

/// What is known about every URL a document has named, and about the
/// host's fonts, which outlive the document.
#[derive(Default)]
pub(crate) struct Resources {
    by_url: HashMap<String, State>,
    by_id: HashMap<u64, Requested>,
    next_id: u64,
    fonts: HostFonts,
    /// The number the next decoded image gets.
    next_image: u32,
    /// The font faces display lists have named, by number.
    faces: RefCell<FontIds>,
    /// What the raster has been sent.
    sent: Sent,
}

/// The font faces a display list names, numbered the first time one does.
/// A face is its font file and its index in it.
#[derive(Default)]
struct FontIds {
    by_face: HashMap<(u64, u32), FontId>,
    faces: Vec<parley::FontData>,
}

/// What the raster's tables hold: the faces numbered below `fonts`, and
/// these images.
#[derive(Default)]
struct Sent {
    fonts: usize,
    images: HashSet<ImageId>,
}

impl Resources {
    /// The decoded image for `url`, if it has arrived.
    pub(crate) fn image(&self, url: &str) -> Option<&Arc<Image>> {
        match self.by_url.get(url) {
            Some(State::Ready(image)) => Some(image),
            _ => None,
        }
    }

    pub(crate) fn fonts(&self) -> &HostFonts {
        &self.fonts
    }

    /// The number a display list names `font` by. Numbering a face changes
    /// nothing a frame shows, so it happens while the list is built from a
    /// shared borrow.
    pub(crate) fn font_id(&self, font: &parley::FontData) -> FontId {
        let mut ids = self.faces.borrow_mut();
        let ids = &mut *ids;
        let key = (font.data.id(), font.index);
        *ids.by_face.entry(key).or_insert_with(|| {
            ids.faces.push(font.clone());
            FontId(u32::try_from(ids.faces.len() - 1).expect("fewer than 2^32 faces"))
        })
    }

    /// What the raster's tables need before they can paint `list`: the
    /// faces numbered since the last call, the images it paints that were
    /// not sent yet, and the sent images whose document is gone.
    pub(crate) fn table_updates(&mut self, list: &DisplayList) -> Vec<TableUpdate> {
        let mut updates = Vec::new();
        let ids = self.faces.borrow();
        for (at, face) in ids.faces.iter().enumerate().skip(self.sent.fonts) {
            updates.push(TableUpdate::Font {
                id: FontId(u32::try_from(at).expect("fewer than 2^32 faces")),
                data: face.data.data().to_vec(),
                index: face.index,
            });
        }
        self.sent.fonts = ids.faces.len();
        drop(ids);
        let ready: HashMap<ImageId, &Image> = self
            .by_url
            .values()
            .filter_map(|state| match state {
                State::Ready(image) => Some((image.id, &**image)),
                _ => None,
            })
            .collect();
        let gone: Vec<ImageId> = self
            .sent
            .images
            .iter()
            .filter(|id| !ready.contains_key(id))
            .copied()
            .collect();
        for id in gone {
            self.sent.images.remove(&id);
            updates.push(TableUpdate::ForgetImage(id));
        }
        for item in &list.items {
            if let DisplayItem::Image { image, .. } = item
                && !self.sent.images.contains(image)
                && let Some(ready) = ready.get(image)
            {
                self.sent.images.insert(*image);
                let (width, height) = ready.size();
                updates.push(TableUpdate::Image {
                    id: *image,
                    width,
                    height,
                    rgba: ready.pixmap.data_as_u8_slice().to_vec(),
                });
            }
        }
        updates
    }

    /// The host's fonts, from now on (p1-contract §6.2).
    pub(crate) fn set_fonts(&mut self, catalogue: FontCatalog) {
        self.fonts.set_catalogue(catalogue);
    }

    /// A request for every image URL and every font face the document uses
    /// that is not known yet.
    pub(crate) fn requests(&mut self, doc: &Document, styles: &Styles) -> Vec<ResourceRequest> {
        let mut requests = Vec::new();
        for url in wanted(doc, styles) {
            if self.by_url.contains_key(&url) {
                continue;
            }
            let id = self.next_id();
            self.by_url.insert(url.clone(), State::Pending);
            self.by_id.insert(id, Requested::Image(url.clone()));
            requests.push(ResourceRequest {
                id,
                url,
                kind: ResourceKind::Image,
            });
        }
        for url in self.fonts.wanted(doc, styles) {
            if self.fonts.is_known(&url) {
                continue;
            }
            let id = self.next_id();
            self.fonts.requested(&url);
            self.by_id.insert(id, Requested::Font(url.clone()));
            requests.push(ResourceRequest {
                id,
                url,
                kind: ResourceKind::Font,
            });
        }
        requests
    }

    fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Forget the document's images for a new document. The ids go on from
    /// where they were, so a late answer to the previous document's request
    /// cannot be taken for one of this document's; fonts, and the requests
    /// for them, are kept.
    pub(crate) fn new_document(&mut self) {
        self.by_url.clear();
        self.by_id
            .retain(|_, requested| matches!(requested, Requested::Font(_)));
    }

    /// The host's answer to request `response.id`. A response that is not an
    /// image Erk can decode, or not a font, leaves the URL missing. A
    /// request is answered once: a second answer, or one to a request never
    /// made, is ignored.
    pub(crate) fn complete(&mut self, response: &ResourceResponse) {
        match self.by_id.remove(&response.id) {
            Some(Requested::Image(url)) => {
                let state = match decode(&response.mime, &response.data) {
                    Ok(pixmap) => {
                        let id = ImageId(self.next_image);
                        self.next_image += 1;
                        State::Ready(Arc::new(Image { id, pixmap }))
                    }
                    Err(_) => State::Missing,
                };
                self.by_url.insert(url, state);
            }
            Some(Requested::Font(url)) => {
                self.fonts.complete(&url, &response.mime, &response.data);
            }
            None => {}
        }
    }

    /// The host has no resource for request `id`.
    pub(crate) fn missing(&mut self, id: u64) {
        match self.by_id.remove(&id) {
            Some(Requested::Image(url)) => {
                self.by_url.insert(url, State::Missing);
            }
            Some(Requested::Font(url)) => self.fonts.missing(&url),
            None => {}
        }
    }

    /// Whether any request is still unanswered.
    pub(crate) fn pending(&self) -> bool {
        self.fonts.pending()
            || self
                .by_url
                .values()
                .any(|state| matches!(state, State::Pending))
    }
}

/// Every image URL the document names, in document order, each once:
/// `<img src>` and the `url()` layers of `background-image`.
fn wanted(doc: &Document, styles: &Styles) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    let mut add = |url: &str| {
        let url = url.trim();
        if !url.is_empty() && !urls.iter().any(|known| known == url) {
            urls.push(url.to_owned());
        }
    };
    let mut stack = vec![doc.root()];
    while let Some(id) = stack.pop() {
        if let Some(NodeData::Element(element)) = doc.node(id).map(|node| &node.data)
            && let Some(style) = styles.computed(id)
        {
            if element.name.local == local_name!("img")
                && let Some(src) = element.attr(&local_name!("src"))
            {
                add(src);
            }
            for layer in style.get_background().background_image.0.iter() {
                if let Some(url) = image_url(layer) {
                    add(&url);
                }
            }
        }
        let mut children: Vec<_> = doc.children(id).collect();
        children.reverse();
        stack.extend(children);
    }
    urls
}

/// The URL of a `url()` image as the content wrote it. Stylo resolves URLs
/// against the stylesheet's base, `about:blank`: an absolute URL comes back
/// resolved, a relative one unresolved and as written; either way it is
/// what the host is asked for.
pub(crate) fn image_url(image: &erk_style::style::values::computed::Image) -> Option<String> {
    use erk_style::style::values::computed::url::ComputedUrl;
    use erk_style::style::values::generics::image::GenericImage;
    match image {
        GenericImage::Url(ComputedUrl::Valid(url)) => Some(url.as_str().to_owned()),
        GenericImage::Url(ComputedUrl::Invalid(text)) => Some(text.as_str().to_owned()),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Png,
    Jpeg,
}

/// Decode an image response. The MIME type names the format; an empty one
/// lets the bytes decide. The bytes must then be that format.
pub(crate) fn decode(mime: &str, data: &[u8]) -> Result<Pixmap, String> {
    let sniffed = sniff(data);
    let format = match mime.split(';').next().unwrap_or("").trim() {
        "image/png" => Format::Png,
        "image/jpeg" | "image/jpg" => Format::Jpeg,
        "" => sniffed.ok_or("not a PNG or JPEG image")?,
        other => return Err(format!("{other} is not an image type Erk decodes")),
    };
    if sniffed.as_ref() != Some(&format) {
        return Err(format!("the data is not the {format:?} its MIME type says"));
    }
    let (width, height, rgba) = match format {
        Format::Png => decode_png(data)?,
        Format::Jpeg => decode_jpeg(data)?,
    };
    // Both decoders checked the header's size before allocating.
    if rgba.len() as u64 != u64::from(width) * u64::from(height) * 4 {
        return Err("the decoded pixels do not match the image size".to_owned());
    }
    let pixels = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&[r, g, b, a]| {
            let premultiply = |channel: u8| ((u16::from(channel) * u16::from(a) + 127) / 255) as u8;
            PremulRgba8 {
                r: premultiply(r),
                g: premultiply(g),
                b: premultiply(b),
                a,
            }
        })
        .collect();
    let width = u16::try_from(width).map_err(|_| "image too wide")?;
    let height = u16::try_from(height).map_err(|_| "image too tall")?;
    Ok(Pixmap::from_parts(pixels, width, height))
}

fn sniff(data: &[u8]) -> Option<Format> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Format::Png)
    } else if data.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(Format::Jpeg)
    } else {
        None
    }
}

/// RGBA8, straight alpha.
fn decode_png(data: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let (width, height) = reader.info().size();
    within_budget(width, height)?;
    let size = reader.output_buffer_size().ok_or("image too large")?;
    let mut buffer = vec![0; size];
    let frame = reader
        .next_frame(&mut buffer)
        .map_err(|error| error.to_string())?;
    let pixels = &buffer[..frame.buffer_size()];
    let rgba = match frame.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => pixels
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|&[l, a]| [l, l, l, a])
            .collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|&l| [l, l, l, 255]).collect(),
        png::ColorType::Indexed => return Err("palette not expanded".to_owned()),
    };
    Ok((width, height, rgba))
}

/// RGBA8, opaque.
fn decode_jpeg(data: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .set_max_width(MAX_SIDE as usize)
        .set_max_height(MAX_SIDE as usize);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(data), options);
    decoder
        .decode_headers()
        .map_err(|error| format!("{error:?}"))?;
    let (width, height) = decoder.dimensions().ok_or("no image size")?;
    within_budget(
        u32::try_from(width).map_err(|_| "image too wide")?,
        u32::try_from(height).map_err(|_| "image too tall")?,
    )?;
    let rgba = decoder.decode().map_err(|error| format!("{error:?}"))?;
    let info = decoder.info().ok_or("no image information")?;
    Ok((u32::from(info.width), u32::from(info.height), rgba))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 2 × 1 PNG: one opaque red pixel, one half-transparent blue one.
    pub(crate) fn tiny_png() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, 2, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[255, 0, 0, 255, 0, 0, 255, 128])
                .unwrap();
        }
        out
    }

    #[test]
    fn a_png_decodes_premultiplied() {
        let image = decode("image/png", &tiny_png()).unwrap();
        assert_eq!((image.width(), image.height()), (2, 1));
        let pixels = image.data();
        assert_eq!((pixels[0].r, pixels[0].a), (255, 255));
        assert_eq!((pixels[1].b, pixels[1].a), (128, 128));
    }

    #[test]
    fn the_bytes_decide_when_there_is_no_mime_type() {
        assert!(decode("", &tiny_png()).is_ok());
        assert!(decode("", b"GIF89a").is_err());
    }

    #[test]
    fn a_response_of_the_wrong_type_is_refused() {
        // A stylesheet sent for an image, and PNG bytes labelled JPEG.
        assert!(decode("text/css", &tiny_png()).is_err());
        assert!(decode("image/jpeg", &tiny_png()).is_err());
    }

    #[test]
    fn broken_image_data_is_refused_without_panicking() {
        let mut png = tiny_png();
        png.truncate(30);
        assert!(decode("image/png", &png).is_err());
        assert!(decode("image/jpeg", &[0xff, 0xd8, 0xff, 0, 1, 2]).is_err());
    }

    /// A header that claims 16000 × 16000 pixels, with no pixel data: within
    /// the side limit, but a gigabyte of RGBA.
    fn huge_png_header() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, 16_000, 16_000);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            // Only the start of a zlib stream.
            writer.write_chunk(png::chunk::IDAT, &[0x78, 0x9c]).unwrap();
            drop(writer);
        }
        out
    }

    /// The reference photo with its frame header changed to 16000 × 16000.
    fn huge_jpeg_header() -> Vec<u8> {
        let mut data = include_bytes!("../tests/reference/images/photo.jpg").to_vec();
        let frame = data
            .windows(2)
            .position(|marker| marker == [0xff, 0xc0])
            .expect("a baseline frame header");
        // Marker, length, precision, then height and width, big-endian.
        data[frame + 5..frame + 9].copy_from_slice(&[0x3e, 0x80, 0x3e, 0x80]);
        data
    }

    #[test]
    fn an_image_too_large_to_hold_is_refused_before_it_is_allocated() {
        for (mime, data) in [
            ("image/png", huge_png_header()),
            ("image/jpeg", huge_jpeg_header()),
        ] {
            let error = decode(mime, &data).expect_err("refused");
            assert!(error.starts_with("the header claims"), "{mime}: {error}");
        }
    }
}
