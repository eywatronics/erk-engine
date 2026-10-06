//! The GPU path (M2.5): vello_hybrid paints the display list through wgpu,
//! into the surface of the host's window (p1-contract §7: the host gives
//! its window by raw-window-handle), or into a texture for the tests.
//!
//! The raster is the renderer thread's (p1-contract §1.1), so it owns the
//! device and the surface. When there is no adapter or the surface cannot
//! be made, the thread falls back to vello_cpu and sends frames as before.
//! Golden images, the Chrome reference and the WPT runner stay on vello_cpu.

use std::collections::HashMap;
use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use vello_cpu::kurbo::{Affine, BezPath, Rect};
use vello_cpu::peniko::Fill;
use vello_cpu::{ImageSource, Pixmap};
use vello_hybrid::{RenderSize, RenderTargetConfig, Renderer, Resources, Scene, TextureBindings};
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
    images: HashMap<ImageId, vello_common::paint::ImageId>,
    /// What the adapter is, for the host's information.
    pub(crate) adapter: String,
}

/// The backends a window is drawn with. On Windows DX12 alone: its instance
/// takes some 0.1 s to make on the window's thread, where one with Vulkan
/// too took 1.8 s (measured 2026-10-05).
#[cfg(windows)]
pub(crate) const WINDOW_BACKENDS: wgpu::Backends = wgpu::Backends::DX12;
#[cfg(not(windows))]
pub(crate) const WINDOW_BACKENDS: wgpu::Backends =
    wgpu::Backends::PRIMARY.union(wgpu::Backends::GL);

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
            Self::new(&adapter, format, Output::Surface { surface, config })
        })
    }

    /// The GPU path into a texture, or why there is none.
    pub(crate) fn offscreen(backends: wgpu::Backends) -> Result<Self, String> {
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
        )
    }

    fn new(
        adapter: &wgpu::Adapter,
        format: wgpu::TextureFormat,
        output: Output,
    ) -> Result<Self, String> {
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .map_err(|error| format!("no device: {error}"))?;
        let (renderer, resources) = Renderer::new(
            &device,
            &RenderTargetConfig {
                format,
                width: 1,
                height: 1,
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
            };
            paint_list(&mut canvas, list, tables, w as u16, h as u16, scale);
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
            if !self.images.contains_key(key) {
                let id = self.renderer.upload_image(
                    &mut self.resources,
                    &self.device,
                    &self.queue,
                    encoder,
                    image,
                );
                self.images.insert(*key, id);
            }
        }
        let gone: Vec<ImageId> = self
            .images
            .keys()
            .filter(|key| !used.contains(key))
            .copied()
            .collect();
        for key in gone {
            if let Some(id) = self.images.remove(&key) {
                self.renderer
                    .destroy_image(&mut self.resources, encoder, id);
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
    images: &'a HashMap<ImageId, vello_common::paint::ImageId>,
}

impl Canvas for Hybrid<'_> {
    fn set_transform(&mut self, transform: Affine) {
        self.scene.set_transform(transform);
    }
    fn set_color(&mut self, rgba: Rgba) {
        self.scene.set_paint(color(rgba));
    }
    fn set_image(&mut self, key: ImageId, _: &Arc<Pixmap>, repeat: (bool, bool)) {
        if let Some(id) = self.images.get(&key) {
            let source = ImageSource::OpaqueId {
                id: *id,
                may_have_transparency: true,
            };
            self.scene.set_paint(image_paint(source, repeat));
        }
    }
    fn set_paint_transform(&mut self, transform: Affine) {
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

    const LOGO: &[u8] = include_bytes!("../tests/reference/images/logo.png");
    const CHECKER: &[u8] = include_bytes!("../tests/reference/images/checker.png");

    /// A page with what the display list holds: backgrounds, rounded
    /// borders of several colours, a shadow, translucency, a clip, text,
    /// images (one repeated) and the highlight.
    const PAGE: &str = r#"<body style="margin: 0; background: #f1f5f9; font-family: 'Noto Sans'; font-size: 16px">
      <div style="margin: 10px; padding: 8px; background: #fff; border: 3px solid; border-color: #2563eb #16a34a #dc2626 #f59e0b; border-radius: 12px; box-shadow: 0 4px 10px rgba(0,0,0,0.3)">Merhaba, <b>dünya</b>! Çizim GPU'da.</div>
      <div style="opacity: 0.5; margin: 10px; height: 30px; background: #7c3aed"></div>
      <div style="overflow: hidden; margin: 10px; width: 120px; height: 30px"><div style="width: 300px; height: 300px; background: #0ea5e9"></div></div>
      <img src="logo.png" style="margin: 10px; width: 64px">
      <div style="margin: 10px; height: 40px; background: url(checker.png) repeat"></div>
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
