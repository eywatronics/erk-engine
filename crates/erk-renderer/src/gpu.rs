//! The GPU path (M2.5): vello_hybrid paints the display list through wgpu,
//! into the surface of the host's window (p1-contract §7: the host gives
//! its window by raw-window-handle), or into a texture for the tests.
//!
//! The raster is the renderer thread's (p1-contract §1.1), so it owns the
//! device and the surface. When there is no adapter or the surface cannot
//! be made, the thread falls back to vello_cpu and sends frames as before.
//! Golden images, the Chrome reference and the WPT runner stay on vello_cpu.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use vello_cpu::kurbo::{Affine, BezPath, Rect};
use vello_cpu::peniko::color::PremulRgba8;
use vello_cpu::peniko::{Fill, Gradient as GradientPaint};
use vello_cpu::{ImageSource, Pixmap};
use vello_hybrid::{
    AtlasConfig, MemorySettings, RenderSettings, RenderSize, RenderTargetConfig, Renderer,
    Resources, Scene, TextureBindings,
};
use wgpu::rwh::{HasDisplayHandle, HasWindowHandle};

use crate::color::Rgba;
use crate::display::{DisplayItem, DisplayList, GlyphRun};
use crate::list::ImageId;
use crate::paint::{Canvas, color, glyphs, image_paint, paint_list};
use crate::tables::Tables;

/// A window the GPU path can draw into: what winit's windows and a host's
/// raw-window-handle wrapper both are.
pub trait Window:
    HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + 'static
{
}
impl<T: HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + 'static> Window for T {}

/// Where the GPU path draws.
enum Output {
    /// A window's surface, configured at its size.
    Surface {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    },
    /// A texture the tests read back, made at the frame's size.
    Texture(Option<wgpu::Texture>),
}

pub(crate) struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    output: Output,
    format: wgpu::TextureFormat,
    renderer: Renderer,
    resources: Resources,
    /// The images uploaded to the atlas, by the number the display list
    /// names them by.
    images: HashMap<ImageId, Uploaded>,
    /// The images the atlas had no room for: not drawn, and not tried
    /// again while the display list paints them.
    unplaceable: HashSet<ImageId>,
    /// The largest image an atlas holds, `(width, height)`: a larger one is
    /// uploaded smaller ([`fitted`]).
    atlas_limit: (u32, u32),
    /// What the adapter is, for the host's information.
    pub(crate) adapter: String,
}

/// An image in the atlas, and how much smaller than the image it is
/// there, `(x, y)`: 1 unless it was larger than an atlas.
#[derive(Clone, Copy)]
struct Uploaded {
    id: vello_common::paint::ImageId,
    shrunk: (f64, f64),
}

/// The image atlas: vello_hybrid's defaults (4096 × 4096 layers, made
/// when the first image arrives, up to eight), named here because the
/// layer's size bounds an image.
fn atlas_config() -> AtlasConfig {
    AtlasConfig::default()
}

/// `image` made small enough for an atlas of `limit`, averaging the pixels
/// each new one covers; `None` when it fits. vello_hybrid panics on an
/// image larger than its atlas (M5.13: a 5000-pixel-wide image stopped the
/// GPU path), where vello_cpu paints it.
fn fitted(image: &Pixmap, limit: (u32, u32)) -> Option<Pixmap> {
    let (width, height) = (u32::from(image.width()), u32::from(image.height()));
    if width <= limit.0 && height <= limit.1 {
        return None;
    }
    let factor = (f64::from(width) / f64::from(limit.0.max(1)))
        .max(f64::from(height) / f64::from(limit.1.max(1)));
    let to_width = ((f64::from(width) / factor) as u32).clamp(1, limit.0.max(1));
    let to_height = ((f64::from(height) / factor) as u32).clamp(1, limit.1.max(1));
    let source = image.data();
    let span = |at: u32, to: u32, from: u32| {
        let start = (u64::from(at) * u64::from(from) / u64::from(to)) as u32;
        let end = ((u64::from(at) + 1) * u64::from(from) / u64::from(to)) as u32;
        start..end.max(start + 1).min(from)
    };
    let mut pixels = Vec::with_capacity((to_width * to_height) as usize);
    for y in 0..to_height {
        let rows = span(y, to_height, height);
        for x in 0..to_width {
            let columns = span(x, to_width, width);
            let mut sum = [0u64; 4];
            let mut count = 0u64;
            for row in rows.clone() {
                for column in columns.clone() {
                    let pixel = source[(row * width + column) as usize];
                    sum[0] += u64::from(pixel.r);
                    sum[1] += u64::from(pixel.g);
                    sum[2] += u64::from(pixel.b);
                    sum[3] += u64::from(pixel.a);
                    count += 1;
                }
            }
            let average = |channel: u64| ((channel + count / 2) / count) as u8;
            pixels.push(PremulRgba8 {
                r: average(sum[0]),
                g: average(sum[1]),
                b: average(sum[2]),
                a: average(sum[3]),
            });
        }
    }
    Some(Pixmap::from_parts(
        pixels,
        to_width as u16,
        to_height as u16,
    ))
}

/// The backends a window is drawn with. On Windows DX12 alone: its instance
/// takes some 0.1 s to make on the window's thread, where one with Vulkan
/// too took 1.8 s (measured 2026-10-05).
#[cfg(windows)]
pub(crate) const WINDOW_BACKENDS: wgpu::Backends = wgpu::Backends::DX12;
#[cfg(not(windows))]
pub(crate) const WINDOW_BACKENDS: wgpu::Backends =
    wgpu::Backends::PRIMARY.union(wgpu::Backends::GL);

/// The device Erk paints with. wgpu's default memory hint, `Performance`,
/// lets its allocator take GPU memory in big blocks up front. Measured on
/// Windows (DX12, the shell's window, 2026-11-23), the process held 343 MB
/// private on a small page and 641 MB on a page of 10 000 cards, against
/// 181 and 476 MB with `MemoryUsage`, with painting as fast (M5.13): Erk's
/// GPU path paints a viewport-sized frame from strips.
///
/// It also asks for [`VIEW_BINDINGS`] non-sampler bindings, or the
/// adapter's limit if lower, where wgpu's default limits ask for a million.
fn device_descriptor(adapter: &wgpu::Limits) -> wgpu::DeviceDescriptor<'static> {
    wgpu::DeviceDescriptor {
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        required_limits: wgpu::Limits {
            max_non_sampler_bindings: adapter.max_non_sampler_bindings.min(VIEW_BINDINGS),
            ..wgpu::Limits::default()
        },
        ..wgpu::DeviceDescriptor::default()
    }
}

/// How many non-sampler bindings (views of buffers and textures) the
/// device may hold at once. On DX12 wgpu makes a shader-visible descriptor
/// heap that large when the device is made: with wgpu's default, a
/// million, the shell's window held 180–199 MB on a small page and 370 MB
/// on a page of 10 000 cards; with this, 152–156 and 344–351 MB (two runs,
/// 2026-11-24, M5.13; 4096 saved no more than the noise). vello_hybrid
/// binds a few textures and buffers per pipeline; the room left is for
/// images, many to a page, and costs 2 MB.
const VIEW_BINDINGS: u32 = 65_536;

/// The formats vello_hybrid writes as they are: its colours are already
/// sRGB-encoded, so an `…Srgb` format would encode them twice.
const FORMATS: [wgpu::TextureFormat; 2] = [
    wgpu::TextureFormat::Bgra8Unorm,
    wgpu::TextureFormat::Rgba8Unorm,
];

impl Gpu {
    /// The surface of `window`, made on the calling thread: some platforms
    /// (Windows, macOS) give a window's handle only on the thread that runs
    /// its event loop. The surface then goes to the renderer thread, which
    /// finishes the start with [`Gpu::for_surface`].
    pub(crate) fn surface(
        window: Arc<dyn Window>,
        backends: wgpu::Backends,
    ) -> Result<WindowSurface, String> {
        if let Err(error) = window.window_handle() {
            return Err(format!("the window gives no handle: {error}"));
        }
        if let Err(error) = window.display_handle() {
            return Err(format!("the window gives no display handle: {error}"));
        }
        caught(|| {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends,
                ..wgpu::InstanceDescriptor::new_with_display_handle(Box::new(window.clone()))
            });
            let surface = instance
                .create_surface(window)
                .map_err(|error| format!("no surface: {error}"))?;
            Ok(WindowSurface { instance, surface })
        })
    }

    /// The GPU path into `surface`: an adapter that can draw into it, and a
    /// device, or why there is none.
    pub(crate) fn for_surface(surface: WindowSurface) -> Result<Self, String> {
        caught(|| {
            let WindowSurface { instance, surface } = surface;
            let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            }))
            .map_err(|error| format!("no adapter: {error}"))?;
            let capabilities = surface.get_capabilities(&adapter);
            let format = capabilities
                .formats
                .iter()
                .copied()
                .find(|format| FORMATS.contains(format))
                .ok_or("the surface takes no format vello_hybrid writes")?;
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: 1,
                height: 1,
                present_mode: wgpu::PresentMode::AutoVsync,
                desired_maximum_frame_latency: 2,
                alpha_mode: capabilities.alpha_modes[0],
                view_formats: Vec::new(),
            };
            Self::new(
                &adapter,
                format,
                Output::Surface { surface, config },
                atlas_config(),
            )
        })
    }

    /// The GPU path into a texture, or why there is none.
    pub(crate) fn offscreen(backends: wgpu::Backends) -> Result<Self, String> {
        Self::offscreen_with(backends, atlas_config())
    }

    /// [`Gpu::offscreen`] with images in an atlas of `atlas`.
    fn offscreen_with(backends: wgpu::Backends, atlas: AtlasConfig) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .map_err(|error| format!("no adapter: {error}"))?;
        Self::new(
            &adapter,
            wgpu::TextureFormat::Rgba8Unorm,
            Output::Texture(None),
            atlas,
        )
    }

    fn new(
        adapter: &wgpu::Adapter,
        format: wgpu::TextureFormat,
        output: Output,
        atlas: AtlasConfig,
    ) -> Result<Self, String> {
        let (device, queue) =
            block_on(adapter.request_device(&device_descriptor(&adapter.limits())))
                .map_err(|error| format!("no device: {error}"))?;
        // vello_hybrid keeps each side of a layer within the device's limit.
        let largest = device.limits().max_texture_dimension_2d;
        let atlas_limit = (
            atlas.atlas_size.0.min(largest),
            atlas.atlas_size.1.min(largest),
        );
        let (renderer, resources) = Renderer::new_with(
            &device,
            &RenderTargetConfig {
                format,
                width: 1,
                height: 1,
            },
            RenderSettings {
                memory_settings: MemorySettings {
                    image_atlas_config: atlas,
                    ..MemorySettings::default()
                },
                ..RenderSettings::default()
            },
        );
        let info = adapter.get_info();
        Ok(Self {
            device,
            queue,
            output,
            format,
            renderer,
            resources,
            images: HashMap::new(),
            unplaceable: HashSet::new(),
            atlas_limit,
            adapter: format!("{} ({:?})", info.name, info.backend),
        })
    }

    /// Paint `list` at `scale` into the output, `width` × `height` device
    /// pixels, and present it. The texture output keeps it for
    /// [`Gpu::read_back`]. `Err` when the surface is gone.
    pub(crate) fn render(
        &mut self,
        list: &DisplayList,
        tables: &Tables,
        width: u16,
        height: u16,
        scale: f32,
    ) -> Result<(), String> {
        let (w, h) = (u32::from(width).max(1), u32::from(height).max(1));
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.upload_images(list, tables, &mut encoder);
        let mut scene = Scene::new(w as u16, h as u16);
        {
            let mut canvas = Hybrid {
                scene: &mut scene,
                resources: &mut self.resources,
                images: &self.images,
                shrunk: None,
            };
            paint_list(&mut canvas, list, tables, w as u16, h as u16, scale, None);
        }
        let size = RenderSize {
            width: w,
            height: h,
        };
        let device = &self.device;
        let (view, frame) = match &mut self.output {
            Output::Surface { surface, config } => {
                if (config.width, config.height) != (w, h) {
                    config.width = w;
                    config.height = h;
                    surface.configure(device, config);
                }
                let texture = match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(texture)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
                    wgpu::CurrentSurfaceTexture::Timeout
                    | wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
                    wgpu::CurrentSurfaceTexture::Outdated => {
                        surface.configure(device, config);
                        return Ok(());
                    }
                    other => return Err(format!("the surface is gone: {other:?}")),
                };
                let view = texture.texture.create_view(&Default::default());
                (view, Some(texture))
            }
            Output::Texture(kept) => {
                let fits = kept
                    .as_ref()
                    .is_some_and(|texture| (texture.width(), texture.height()) == (w, h));
                if !fits {
                    *kept = Some(target_texture(device, self.format, w, h));
                }
                let texture = kept.as_ref().expect("made above");
                (texture.create_view(&Default::default()), None)
            }
        };
        self.renderer
            .render(
                &scene,
                &mut self.resources,
                &self.device,
                &self.queue,
                &mut encoder,
                &size,
                &view,
                &TextureBindings::default(),
            )
            .map_err(|error| format!("vello_hybrid: {error:?}"))?;
        self.queue.submit([encoder.finish()]);
        match frame {
            Some(frame) => frame.present(),
            // Offscreen, the frame is done when this returns: for the
            // read-back and for timing it.
            None => {
                let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
            }
        }
        Ok(())
    }

    /// Upload the images `list` paints that are not on the GPU yet, and let
    /// go of those it no longer paints.
    fn upload_images(
        &mut self,
        list: &DisplayList,
        tables: &Tables,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let mut used = std::collections::HashSet::new();
        for item in &list.items {
            let DisplayItem::Image { image: key, .. } = item else {
                continue;
            };
            let Some(image) = tables.image(*key) else {
                continue;
            };
            used.insert(*key);
            if self.images.contains_key(key) || self.unplaceable.contains(key) {
                continue;
            }
            let smaller = fitted(image, self.atlas_limit);
            let upload: &Pixmap = smaller.as_ref().unwrap_or(image);
            // vello_hybrid unwraps the atlas's allocation: with no room
            // left in it, it panics before it changes anything. The image
            // is not drawn; the page and the GPU path go on.
            let (renderer, resources, device, queue) = (
                &mut self.renderer,
                &mut self.resources,
                &self.device,
                &self.queue,
            );
            let uploaded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                renderer.upload_image(resources, device, queue, encoder, upload)
            }));
            match uploaded {
                Ok(id) => {
                    let shrunk = (
                        f64::from(image.width()) / f64::from(upload.width()),
                        f64::from(image.height()) / f64::from(upload.height()),
                    );
                    self.images.insert(*key, Uploaded { id, shrunk });
                }
                Err(_) => {
                    self.unplaceable.insert(*key);
                }
            }
        }
        self.unplaceable.retain(|key| used.contains(key));
        let gone: Vec<ImageId> = self
            .images
            .keys()
            .filter(|key| !used.contains(key))
            .copied()
            .collect();
        for key in gone {
            if let Some(uploaded) = self.images.remove(&key) {
                self.renderer
                    .destroy_image(&mut self.resources, encoder, uploaded.id);
            }
        }
    }

    /// The last frame of the texture output, as RGBA rows.
    #[cfg(test)]
    pub(crate) fn read_back(&self) -> Vec<u8> {
        let Output::Texture(Some(texture)) = &self.output else {
            return Vec::new();
        };
        let (width, height) = (texture.width(), texture.height());
        // Rows of a buffer copy are padded to 256 bytes.
        let row = (width * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("erk read back"),
            size: u64::from(row * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            texture.size(),
        );
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the copy finishes");
        let mapped = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            let start = (y * row) as usize;
            pixels.extend_from_slice(&mapped[start..start + (width * 4) as usize]);
        }
        pixels
    }
}

/// A texture to draw a `width` × `height` frame into and copy out of.
fn target_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("erk offscreen frame"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// vello_hybrid's scene, with the images uploaded for it.
struct Hybrid<'a> {
    scene: &'a mut Scene,
    resources: &'a mut Resources,
    images: &'a HashMap<ImageId, Uploaded>,
    /// How much smaller than the image the paint's atlas copy is, while
    /// the paint is an image: its paint transform, which maps the image's
    /// own pixels, is scaled by it.
    shrunk: Option<(f64, f64)>,
}

impl Canvas for Hybrid<'_> {
    fn set_transform(&mut self, transform: Affine) {
        self.scene.set_transform(transform);
    }
    fn set_color(&mut self, rgba: Rgba) {
        self.shrunk = None;
        self.scene.set_paint(color(rgba));
    }
    fn set_image(&mut self, key: ImageId, _: &Arc<Pixmap>, repeat: (bool, bool)) {
        match self.images.get(&key) {
            Some(uploaded) => {
                let source = ImageSource::OpaqueId {
                    id: uploaded.id,
                    may_have_transparency: true,
                };
                self.shrunk = Some(uploaded.shrunk);
                self.scene.set_paint(image_paint(source, repeat));
            }
            // Not in the atlas: nothing is drawn, not the paint before.
            None => {
                self.shrunk = None;
                self.scene.set_paint(color([0, 0, 0, 0]));
            }
        }
    }
    fn set_gradient(&mut self, gradient: GradientPaint) {
        self.shrunk = None;
        self.scene.set_paint(gradient);
    }
    fn set_paint_transform(&mut self, transform: Affine) {
        let transform = match self.shrunk {
            Some((x, y)) => transform * Affine::scale_non_uniform(x, y),
            None => transform,
        };
        self.scene.set_paint_transform(transform);
    }
    fn reset_paint_transform(&mut self) {
        self.scene.reset_paint_transform();
    }
    fn set_fill_rule(&mut self, rule: Fill) {
        self.scene.set_fill_rule(rule);
    }
    fn fill_rect(&mut self, rect: &Rect) {
        self.scene.fill_rect(rect);
    }
    fn fill_path(&mut self, path: &BezPath) {
        self.scene.fill_path(path);
    }
    fn fill_blurred_rounded_rect(&mut self, rect: &Rect, radius: f32, std_dev: f32) {
        self.scene
            .fill_blurred_rounded_rect(rect, radius, std_dev, false);
    }
    fn push_clip_layer(&mut self, path: &BezPath) {
        self.scene.push_clip_layer(path);
    }
    fn push_opacity_layer(&mut self, opacity: f32) {
        self.scene.push_opacity_layer(opacity);
    }
    fn pop_layer(&mut self) {
        self.scene.pop_layer();
    }
    fn glyphs(&mut self, run: &GlyphRun, font: &parley::FontData) {
        self.scene
            .glyph_run(self.resources, font)
            .font_size(run.size)
            .hint(true)
            // No glyph atlas: with it the GPU painted nodes-1000 in 9.1 ms
            // instead of 12.8, but 0.24% of the parity page's pixels moved
            // away from vello_cpu's (measured 2026-10-05).
            .fill_glyphs(glyphs(run));
    }
}

/// A window's surface, on its way to the renderer thread.
pub(crate) struct WindowSurface {
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
}

/// `start`'s result, a panic turned into an error. wgpu panics on some
/// failures (a window without a display handle, some drivers) where it
/// could return one: a panic while starting is a reason to fall back too,
/// not the end of the renderer.
fn caught<T>(start: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(start)).unwrap_or_else(|panic| {
        let message = panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_default();
        Err(format!("starting the GPU path panicked: {message}"))
    })
}

/// Run a wgpu future to its end. On native backends adapter and device
/// requests are ready at once; this only polls.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        std::thread::yield_now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::ResourceResponse;
    use crate::page::Page;
    use crate::resources::Resources as PageResources;

    #[test]
    fn the_device_asks_for_memory_before_speed() {
        // wgpu's default hint takes GPU memory in big blocks up front: a
        // small page's window held 343 MB, 181 MB with this one, painting
        // as fast (M5.13).
        assert!(matches!(
            device_descriptor(&wgpu::Limits::default()).memory_hints,
            wgpu::MemoryHints::MemoryUsage
        ));
    }

    #[test]
    fn the_device_asks_for_few_view_bindings() {
        // DX12's shader-visible view heap is as large as this limit: wgpu's
        // default, a million, held 32 MB in every window (M5.13).
        // An adapter with more than the defaults, as a desktop GPU has.
        let limits = |adapter: u32| {
            device_descriptor(&wgpu::Limits {
                max_non_sampler_bindings: adapter,
                max_texture_dimension_2d: 32_768,
                ..wgpu::Limits::default()
            })
            .required_limits
        };
        assert_eq!(limits(1_000_000).max_non_sampler_bindings, VIEW_BINDINGS);
        // Never more than the adapter has: the device would not be made.
        assert_eq!(limits(1_000).max_non_sampler_bindings, 1_000);
        // The rest as before.
        assert_eq!(
            wgpu::Limits {
                max_non_sampler_bindings: wgpu::Limits::default().max_non_sampler_bindings,
                ..limits(1_000_000)
            },
            wgpu::Limits::default()
        );
    }

    const LOGO: &[u8] = include_bytes!("../tests/reference/images/logo.png");
    const CHECKER: &[u8] = include_bytes!("../tests/reference/images/checker.png");

    /// A page with what the display list holds: backgrounds, rounded
    /// borders of several colours, a shadow, translucency, a clip, text,
    /// images (one repeated), gradients (linear, repeating, an elliptic
    /// radial one), a transform and the highlight.
    const PAGE: &str = r#"<body style="margin: 0; background: #f1f5f9; font-family: 'Noto Sans'; font-size: 16px">
      <div style="margin: 10px; padding: 8px; background: #fff; border: 3px solid; border-color: #2563eb #16a34a #dc2626 #f59e0b; border-radius: 12px; box-shadow: 0 4px 10px rgba(0,0,0,0.3)">Merhaba, <b>dünya</b>! Çizim GPU'da.</div>
      <div style="opacity: 0.5; margin: 10px; height: 30px; background: #7c3aed"></div>
      <div style="overflow: hidden; margin: 10px; width: 120px; height: 30px"><div style="width: 300px; height: 300px; background: #0ea5e9"></div></div>
      <img src="logo.png" style="margin: 10px; width: 64px">
      <div style="margin: 10px; height: 40px; background: url(checker.png) repeat"></div>
      <div style="display: flex; gap: 10px; margin: 10px; height: 50px">
        <div style="flex: 1; transform: rotate(8deg) scale(0.9); background: linear-gradient(30deg, #7c3aed, #22d3ee 70%, #fde047)"></div>
        <div style="flex: 1; background: repeating-linear-gradient(45deg, #334155 0 6px, #e2e8f0 6px 12px)"></div>
        <div style="flex: 1; border-radius: 12px; background: radial-gradient(ellipse at 30% 40%, #fff, #059669 60%, #064e3b)"></div>
      </div>
    </body>"#;

    /// The page through both rasterizers: CPU pixels, GPU pixels.
    fn both(width: u16, height: u16, scale: f32) -> (Vec<u8>, Vec<u8>) {
        let mut page = Page::parse(PAGE);
        let mut resources = PageResources::default();
        let (_, requests) = page.prepare(width, height, scale, &mut resources, &mut |_| {});
        for request in requests {
            let data = if request.url.contains("logo") {
                LOGO
            } else {
                CHECKER
            };
            resources.complete(&ResourceResponse {
                id: request.id,
                mime: "image/png".to_owned(),
                data: data.to_vec(),
            });
        }
        let (list, _) = page.prepare(width, height, scale, &mut resources, &mut |_| {});
        let mut tables = Tables::default();
        tables.apply(resources.table_updates(&list));
        let cpu = crate::paint::paint(&list, &tables, width, height, crate::device_scale(scale));
        let mut gpu =
            Gpu::offscreen(wgpu::Backends::all()).expect("a GPU adapter, or WARP or lavapipe");
        eprintln!("adapter: {}", gpu.adapter);
        gpu.render(&list, &tables, width, height, crate::device_scale(scale))
            .unwrap();
        (cpu.data_as_u8_slice().to_vec(), gpu.read_back())
    }

    /// A PNG of `width` × `height`, its left half `left` and the rest
    /// `right`.
    fn halves_png(width: u32, height: u32, left: [u8; 4], right: [u8; 4]) -> Vec<u8> {
        let mut data = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut data, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            let pixels: Vec<u8> = (0..width * height)
                .flat_map(|at| if at % width < width / 2 { left } else { right })
                .collect();
            writer.write_image_data(&pixels).unwrap();
        }
        data
    }

    /// `html` with its images (by a word in their URL) through both
    /// rasterizers, the GPU's images in a 64 × 64 atlas of one layer.
    fn small_atlas(html: &str, images: &[(&str, Vec<u8>)], size: (u16, u16)) -> Painted {
        let (width, height) = size;
        let mut page = Page::parse(html);
        let mut resources = PageResources::default();
        let (_, requests) = page.prepare(width, height, 1.0, &mut resources, &mut |_| {});
        for request in requests {
            let (_, data) = images
                .iter()
                .find(|(word, _)| request.url.contains(word))
                .expect("an image the test has");
            resources.complete(&ResourceResponse {
                id: request.id,
                mime: "image/png".to_owned(),
                data: data.clone(),
            });
        }
        let (list, _) = page.prepare(width, height, 1.0, &mut resources, &mut |_| {});
        let mut tables = Tables::default();
        tables.apply(resources.table_updates(&list));
        let cpu = crate::paint::paint(&list, &tables, width, height, 1.0);
        let atlas = AtlasConfig {
            initial_atlas_count: 1,
            max_atlases: 1,
            atlas_size: (64, 64),
            auto_grow: false,
            ..AtlasConfig::default()
        };
        let mut gpu = Gpu::offscreen_with(wgpu::Backends::all(), atlas)
            .expect("a GPU adapter, or WARP or lavapipe");
        gpu.render(&list, &tables, width, height, 1.0).unwrap();
        Painted {
            cpu: cpu.data_as_u8_slice().to_vec(),
            frame: gpu.read_back(),
            gpu,
            list,
            tables,
        }
    }

    /// What [`small_atlas`] painted, and with what.
    struct Painted {
        cpu: Vec<u8>,
        frame: Vec<u8>,
        gpu: Gpu,
        list: DisplayList,
        tables: Tables,
    }

    /// The pixel at `(x, y)` of a frame `width` wide.
    fn at(frame: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
        let at = (y * width + x) * 4;
        frame[at..at + 4].try_into().unwrap()
    }

    fn near(a: [u8; 4], b: [u8; 4]) -> bool {
        a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 12)
    }

    #[test]
    fn an_image_larger_than_the_atlas_is_drawn_smaller() {
        // vello_hybrid panicked on it (M5.13). Red, then blue: drawn from a
        // smaller copy at the image's own scale, the halves stay halves.
        let (red, blue) = ([220, 38, 38, 255], [37, 99, 235, 255]);
        let wide = halves_png(200, 20, red, blue);
        let Painted {
            cpu, frame: gpu, ..
        } = small_atlas(
            "<body style='margin: 0; background: #fff'><img src='wide.png'></body>",
            &[("wide", wide)],
            (300, 60),
        );
        for (x, colour) in [(20, red), (90, red), (110, blue), (180, blue)] {
            assert!(near(at(&cpu, 300, x, 10), colour), "the CPU at {x}");
            assert!(
                near(at(&gpu, 300, x, 10), colour),
                "the GPU at {x}: {:?}",
                at(&gpu, 300, x, 10)
            );
        }
    }

    #[test]
    fn an_image_the_atlas_has_no_room_for_is_not_drawn() {
        // The first fills the atlas; the second is not drawn, and neither
        // is the colour painted before it.
        let (green, red) = ([22, 163, 74, 255], [220, 38, 38, 255]);
        let Painted {
            cpu,
            frame,
            mut gpu,
            list,
            tables,
        } = small_atlas(
            "<body style='margin: 0; background: #fff'>\
             <div style='height: 10px; background: #2563eb'></div>\
             <img src='first.png'><img src='second.png'></body>",
            &[
                ("first", halves_png(64, 64, green, green)),
                ("second", halves_png(64, 64, red, red)),
            ],
            (200, 100),
        );
        assert!(near(at(&cpu, 200, 96, 40), red), "the CPU draws both");
        assert!(near(at(&frame, 200, 32, 40), green), "the first is drawn");
        assert!(
            near(at(&frame, 200, 96, 40), [255, 255, 255, 255]),
            "the second is not: {:?}",
            at(&frame, 200, 96, 40)
        );
        // Not tried again each frame while the page paints it, and
        // forgotten once it does not.
        assert_eq!(gpu.unplaceable.len(), 1);
        gpu.render(&list, &tables, 200, 100, 1.0).unwrap();
        assert_eq!(gpu.unplaceable.len(), 1);
        let without = DisplayList {
            canvas: [255, 255, 255, 255],
            items: Vec::new(),
        };
        gpu.render(&without, &tables, 200, 100, 1.0).unwrap();
        assert!(gpu.unplaceable.is_empty());
    }

    #[test]
    fn fitting_averages_what_it_shrinks() {
        let pixel = |v: u8| PremulRgba8 {
            r: v,
            g: v,
            b: v,
            a: 255,
        };
        // 4 × 2: the left 2 × 2 block four greys, the right one white.
        let image = Pixmap::from_parts(
            vec![
                pixel(0),
                pixel(101),
                pixel(255),
                pixel(255),
                pixel(50),
                pixel(151),
                pixel(255),
                pixel(255),
            ],
            4,
            2,
        );
        assert!(fitted(&image, (4, 2)).is_none(), "it fits");
        let smaller = fitted(&image, (2, 2)).expect("too wide");
        assert_eq!((smaller.width(), smaller.height()), (2, 1));
        // (0 + 101 + 50 + 151) / 4 = 75.5, rounded.
        assert_eq!(smaller.data()[0].r, 76);
        assert_eq!(smaller.data()[1].r, 255);
        // The shape is kept.
        let long = Pixmap::from_parts(vec![pixel(9); 300 * 30], 300, 30);
        let fit = fitted(&long, (100, 100)).expect("too wide");
        assert_eq!((fit.width(), fit.height()), (100, 10));
    }

    #[test]
    fn a_panic_while_starting_is_a_reason_to_fall_back() {
        let error = caught(|| -> Result<(), String> { panic!("the driver gave up") })
            .expect_err("an error, not a panic");
        assert!(error.contains("the driver gave up"), "{error}");
        assert_eq!(caught(|| Ok(7)), Ok(7));
    }

    #[test]
    fn without_an_adapter_there_is_no_gpu_path() {
        let Err(error) = Gpu::offscreen(wgpu::Backends::empty()) else {
            panic!("an adapter without backends");
        };
        assert!(error.contains("adapter"), "{error}");
    }

    #[test]
    fn the_gpu_paints_what_the_cpu_paints() {
        for (width, height, scale) in [(400, 360, 1.0), (500, 400, 2.0)] {
            let (cpu, gpu) = both(width, height, scale);
            assert_eq!(cpu.len(), gpu.len());
            let pixels = cpu.len() / 4;
            let off = cpu
                .chunks(4)
                .zip(gpu.chunks(4))
                .filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > 12))
                .count();
            let share = off as f64 / pixels as f64;
            eprintln!(
                "{width}x{height}@{scale}: {off} of {pixels} pixels differ ({:.3}%)",
                share * 100.0
            );
            // Measured 2026-10-05: 14 of 144000 at scale 1, none at 2.
            assert!(
                share < 0.001,
                "{width}x{height}@{scale}: {off} pixels differ"
            );
        }
    }
}
