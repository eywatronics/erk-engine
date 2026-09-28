//! Erk Engine style: computes CSS styles for an [`erk_dom::Document`] with
//! Stylo, Servo's and Firefox's style engine.
//!
//! This crate is the only one that does not forbid `unsafe`: Stylo's
//! `TElement` trait declares five methods as `unsafe fn`, and implementing
//! them is an `unsafe_code` violation even with safe bodies. See
//! docs/design/p0-architecture.md §6.1.

mod node;
mod side;

use std::sync::Arc as StdArc;

use app_units::Au;
use erk_dom::{Document, NodeData, NodeId, local_name, ns};
use euclid::{Scale, Size2D};
use selectors::Element as _;
use style::context::{QuirksMode, SharedStyleContext};
use style::device::Device;
use style::dom::TDocument;
use style::font_metrics::FontMetrics;
use style::global_style_data::GLOBAL_STYLE_DATA;
use style::media_queries::{MediaList, MediaType};
use style::properties::style_structs::Font;
use style::queries::values::PrefersColorScheme;
use style::selector_parser::SnapshotMap;
use style::servo::media_features::PointerCapabilities;
use style::servo_arc::Arc;
use style::shared_lock::{SharedRwLock, StylesheetGuards};
use style::stylesheets::{AllowImportRules, DocumentStyleSheet, Origin, Stylesheet, UrlExtraData};
use style::stylist::Stylist;
use style::thread_state::{self, ThreadState};
use style::traversal::DomTraversal;
use style::traversal_flags::TraversalFlags;
use style::values::computed::font::{GenericFontFamily, QueryFontMetricsFlags};
use style::values::computed::{CSSPixelLength, Length};

pub use style;
pub use style::device::servo::FontMetricsProvider;
pub use style::font_metrics::FontMetrics as StyleFontMetrics;
pub use style::properties::ComputedValues;

use crate::node::{ErkNode, NoPainters, RecalcStyle};
use crate::side::StyledTree;

const UA_CSS: &str = include_str!("ua.css");

/// Computes styles for documents rendered into a viewport of a given size.
pub struct StyleEngine {
    viewport: Size2D<f32, style_traits::CSSPixel>,
    guard: SharedRwLock,
    url: UrlExtraData,
    user_agent: DocumentStyleSheet,
    font_metrics: StdArc<dyn FontMetricsProvider + Send>,
}

impl StyleEngine {
    /// `width` and `height` are the viewport size in CSS pixels.
    ///
    /// Font-relative units (`ex`, `ch`, `cap`, `ic`) use fixed fractions of
    /// the font size; use [`StyleEngine::with_font_metrics`] to measure them
    /// from real fonts.
    pub fn new(width: f32, height: f32) -> Self {
        Self::with_font_metrics(width, height, StdArc::new(FixedFontMetrics))
    }

    /// Like [`StyleEngine::new`], with font metrics answered by `font_metrics`.
    /// This crate does not load fonts; the renderer, which does, supplies them.
    pub fn with_font_metrics(
        width: f32,
        height: f32,
        font_metrics: StdArc<dyn FontMetricsProvider + Send>,
    ) -> Self {
        let guard = SharedRwLock::new();
        let url = UrlExtraData::from(url::Url::parse("about:blank").expect("valid URL"));
        let user_agent = stylesheet(UA_CSS, Origin::UserAgent, &guard, &url);
        Self {
            viewport: Size2D::new(width, height),
            guard,
            url,
            user_agent,
            font_metrics,
        }
    }

    /// Style every element in `doc`, using the UA stylesheet and the
    /// document's `<style>` elements.
    pub fn style(&self, doc: &Document) -> Styles {
        let mut stylist = Stylist::new(self.device(), QuirksMode::NoQuirks);
        let read = self.guard.read();
        stylist.append_stylesheet(self.user_agent.clone(), &read);
        for css in author_styles(doc) {
            stylist.append_stylesheet(
                stylesheet(&css, Origin::Author, &self.guard, &self.url),
                &read,
            );
        }

        let tree = StyledTree::new(doc, &self.guard);
        tree.populate(&self.url);
        let Some(root) = TDocument::as_node(&ErkNode::new(&tree, doc.root())).first_element_child()
        else {
            return Styles::default();
        };

        thread_state::enter(ThreadState::LAYOUT);
        let guards = StylesheetGuards {
            author: &read,
            ua_or_user: &read,
        };
        let snapshots = SnapshotMap::new();
        stylist.flush(&guards).process_style(root, Some(&snapshots));

        let context = SharedStyleContext {
            traversal_flags: TraversalFlags::empty(),
            stylist: &stylist,
            options: GLOBAL_STYLE_DATA.options.clone(),
            guards,
            visited_styles_enabled: false,
            animations: Default::default(),
            current_time_for_animations: 0.0,
            snapshot_map: &snapshots,
            registered_speculative_painters: &NoPainters,
        };
        let token = RecalcStyle::pre_traverse(root, &context);
        if token.should_traverse() {
            // No thread pool: styling runs sequentially until the DOM is
            // proven safe to share across threads.
            style::driver::traverse_dom(&RecalcStyle::new(context), token, None);
        }
        thread_state::exit(ThreadState::LAYOUT);

        // The styled tree borrows the document; keep only the results.
        Styles {
            computed: tree
                .nodes()
                .iter()
                .map(|node| node.borrow_data()?.styles.get_primary().cloned())
                .collect(),
        }
    }

    fn device(&self) -> Device {
        Device::new(
            MediaType::screen(),
            QuirksMode::NoQuirks,
            self.viewport,
            Size2D::new(self.viewport.width, self.viewport.height),
            Scale::new(1.0),
            Box::new(SharedFontMetrics(self.font_metrics.clone())),
            ComputedValues::initial_values_with_font_override(Font::initial_values()),
            PrefersColorScheme::Light,
            PointerCapabilities::default(),
            PointerCapabilities::default(),
        )
    }
}

/// The result of styling one document.
#[derive(Default)]
pub struct Styles {
    computed: Vec<Option<Arc<ComputedValues>>>,
}

impl Styles {
    /// The computed style of an element, or `None` for non-elements and
    /// elements that were not styled (for example inside `display: none`).
    ///
    /// `id` must come from the document these styles were computed for.
    pub fn computed(&self, id: NodeId) -> Option<Arc<ComputedValues>> {
        self.computed.get(id.index() as usize)?.clone()
    }
}

/// The text of every `<style>` element, in tree order.
fn author_styles(doc: &Document) -> Vec<String> {
    let mut sheets = Vec::new();
    let mut stack = vec![doc.root()];
    while let Some(id) = stack.pop() {
        let mut children: Vec<_> = doc.children(id).collect();
        children.reverse();
        stack.extend(children);
        let Some(element) = doc.node(id).and_then(|node| node.as_element()) else {
            continue;
        };
        if element.name.ns == ns!(html) && element.name.local == local_name!("style") {
            let css: String = doc
                .children(id)
                .filter_map(|child| match &doc.node(child)?.data {
                    NodeData::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            sheets.push(css);
        }
    }
    sheets
}

fn stylesheet(
    css: &str,
    origin: Origin,
    guard: &SharedRwLock,
    url: &UrlExtraData,
) -> DocumentStyleSheet {
    DocumentStyleSheet(Arc::new(Stylesheet::from_str(
        css,
        url.clone(),
        origin,
        Arc::new(guard.wrap(MediaList::empty())),
        guard.clone(),
        None,
        None,
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
    )))
}

/// Stylo's `Device` wants to own its provider, but one engine styles many
/// documents; this lets every `Device` share the engine's provider.
#[derive(Debug)]
struct SharedFontMetrics(StdArc<dyn FontMetricsProvider + Send>);

impl FontMetricsProvider for SharedFontMetrics {
    fn query_font_metrics(
        &self,
        vertical: bool,
        font: &Font,
        font_size: CSSPixelLength,
        flags: QueryFontMetricsFlags,
    ) -> FontMetrics {
        self.0.query_font_metrics(vertical, font, font_size, flags)
    }

    fn base_size_for_generic(&self, generic: GenericFontFamily) -> Length {
        self.0.base_size_for_generic(generic)
    }
}

/// Font metrics as fixed fractions of the font size, for when no fonts are
/// loaded (this crate's own tests). They only affect font-relative units
/// such as `ex` and `ch`.
#[derive(Debug)]
struct FixedFontMetrics;

impl FontMetricsProvider for FixedFontMetrics {
    fn query_font_metrics(
        &self,
        _vertical: bool,
        _font: &Font,
        font_size: CSSPixelLength,
        _flags: QueryFontMetricsFlags,
    ) -> FontMetrics {
        let size = font_size.px();
        FontMetrics {
            ascent: CSSPixelLength::new(size * 0.8),
            x_height: Some(CSSPixelLength::new(size * 0.5)),
            cap_height: Some(CSSPixelLength::new(size * 0.7)),
            zero_advance_measure: Some(CSSPixelLength::new(size * 0.5)),
            ic_width: Some(CSSPixelLength::new(size)),
            script_percent_scale_down: None,
            script_script_percent_scale_down: None,
        }
    }

    fn base_size_for_generic(&self, generic: GenericFontFamily) -> Length {
        let px = match generic {
            GenericFontFamily::Monospace => 13.0,
            _ => 16.0,
        };
        Length::from(Au::from_f32_px(px))
    }
}
