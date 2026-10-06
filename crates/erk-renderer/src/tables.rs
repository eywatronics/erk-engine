//! The raster's font and image tables: what the numbers in a display list
//! stand for (list.rs). They are filled from `TableUpdate`s only, so the
//! raster holds its own copy of every face and image it paints and shares
//! nothing with the engine.

use std::collections::HashMap;
use std::sync::Arc;

use parley::FontData;
use vello_cpu::Pixmap;
use vello_cpu::color::PremulRgba8;
use vello_cpu::peniko::Blob;

use crate::list::{FontId, ImageId, TableUpdate};

#[derive(Default)]
pub(crate) struct Tables {
    fonts: HashMap<FontId, FontData>,
    images: HashMap<ImageId, Arc<Pixmap>>,
}

impl Tables {
    pub(crate) fn apply(&mut self, updates: Vec<TableUpdate>) {
        for update in updates {
            match update {
                TableUpdate::Font { id, data, index } => {
                    self.fonts
                        .insert(id, FontData::new(Blob::new(Arc::new(data)), index));
                }
                TableUpdate::Image {
                    id,
                    width,
                    height,
                    rgba,
                } => {
                    let pixels = rgba
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|&[r, g, b, a]| PremulRgba8 { r, g, b, a })
                        .collect();
                    self.images
                        .insert(id, Arc::new(Pixmap::from_parts(pixels, width, height)));
                }
                TableUpdate::ForgetImage(id) => {
                    self.images.remove(&id);
                }
            }
        }
    }

    pub(crate) fn font(&self, id: FontId) -> Option<&FontData> {
        self.fonts.get(&id)
    }

    pub(crate) fn image(&self, id: ImageId) -> Option<&Arc<Pixmap>> {
        self.images.get(&id)
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> (usize, usize) {
        (self.fonts.len(), self.images.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::ResourceResponse;
    use crate::page::Page;
    use crate::resources::Resources;
    use crate::resources::tests::tiny_png;

    const TEXT: &str = r#"<p style="font-family: 'Noto Sans'">Erk <b>bir kez</b> gönderir.</p>"#;
    const IMAGE: &str = r#"<body style="margin: 0"><img src="a.png" style="display: block; width: 20px"><p>Erk</p>"#;

    fn kinds(updates: &[TableUpdate]) -> Vec<&'static str> {
        updates
            .iter()
            .map(|update| match update {
                TableUpdate::Font { .. } => "font",
                TableUpdate::Image { .. } => "image",
                TableUpdate::ForgetImage(_) => "forget",
            })
            .collect()
    }

    #[test]
    fn a_face_goes_to_the_raster_once() {
        let mut page = Page::parse(TEXT);
        let mut resources = Resources::default();
        let (list, _) = page.prepare(200, 100, 1.0, &mut resources);
        let first = resources.table_updates(&list);
        // Regular and bold.
        assert_eq!(kinds(&first), ["font", "font"]);
        let (list, _) = page.prepare(150, 100, 1.0, &mut resources);
        assert!(resources.table_updates(&list).is_empty());
        // A new document keeps the faces the raster has.
        page.load(TEXT);
        resources.new_document();
        let (list, _) = page.prepare(200, 100, 1.0, &mut resources);
        assert!(resources.table_updates(&list).is_empty());
    }

    #[test]
    fn a_replaced_documents_images_leave_the_raster() {
        let mut page = Page::parse(IMAGE);
        let mut resources = Resources::default();
        let (_, requests) = page.prepare(100, 100, 1.0, &mut resources);
        resources.complete(&ResourceResponse {
            id: requests[0].id,
            mime: "image/png".to_owned(),
            data: tiny_png(),
        });
        let mut tables = Tables::default();
        let (list, _) = page.prepare(100, 100, 1.0, &mut resources);
        let updates = resources.table_updates(&list);
        assert_eq!(kinds(&updates), ["font", "image"]);
        tables.apply(updates);
        assert_eq!(tables.len(), (1, 1));
        let (list, _) = page.prepare(100, 100, 1.0, &mut resources);
        assert!(resources.table_updates(&list).is_empty(), "sent once");
        page.load("<p>Erk</p>");
        resources.new_document();
        let (list, _) = page.prepare(100, 100, 1.0, &mut resources);
        let updates = resources.table_updates(&list);
        assert_eq!(kinds(&updates), ["forget"]);
        tables.apply(updates);
        assert_eq!(tables.len(), (1, 0));
    }

    #[test]
    fn the_raster_paints_from_its_own_copies() {
        // The image's first pixel is opaque red: the raster's copy holds
        // the engine's pixels.
        let mut page = Page::parse(IMAGE);
        let mut resources = Resources::default();
        let (_, requests) = page.prepare(60, 40, 1.0, &mut resources);
        resources.complete(&ResourceResponse {
            id: requests[0].id,
            mime: "image/png".to_owned(),
            data: tiny_png(),
        });
        let mut tables = Tables::default();
        let (frame, _) = page.render(60, 40, 1.0, &mut resources, &mut tables);
        assert_eq!(
            &frame.rgba()[..4],
            &[255, 0, 0, 255],
            "the image is painted"
        );
        // Without its table entry the image is left out, not painted wrong.
        let (list, _) = page.prepare(60, 40, 1.0, &mut resources);
        let pixmap = crate::paint::paint(&list, &Tables::default(), 60, 40, 1.0);
        assert_eq!(&pixmap.data_as_u8_slice()[..4], &[255, 255, 255, 255]);
    }
}
