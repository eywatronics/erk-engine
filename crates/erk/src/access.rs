//! A window's accessibility tree (M5.11's second step).
//!
//! The renderer gives Erk's own plain tree (`Engine::access_tree`); this
//! turns it into AccessKit's, which `accesskit_winit` hands to the
//! platform: UI Automation on Windows, AT-SPI on Linux, NSAccessibility on
//! macOS. Nothing is built until a screen reader asks: the first request
//! gets the whole tree, every turn that prepared a frame after it only the
//! nodes that changed and the focus. A change of scale sends the whole
//! tree again, every box being in device pixels.

use std::collections::{HashMap, HashSet};

use accesskit::{Action, Node, NodeId, Rect, Role, Toggled, TreeId, TreeInfo, TreeUpdate};
use erk_renderer::{AccessNode, AccessRole};

/// What was last sent to the platform, to send only what changed.
#[derive(Default)]
pub(crate) struct AccessSync {
    sent: HashMap<u64, AccessNode>,
    scale: f64,
    /// The app's count of prepared frames when the tree was taken.
    frame: u64,
}

impl AccessSync {
    /// The whole of `tree`, taken after the app's `frame`th prepared
    /// frame, at `scale` (device pixels per CSS pixel): what a screen
    /// reader that has just asked gets.
    pub(crate) fn full(&mut self, tree: Vec<AccessNode>, scale: f64, frame: u64) -> TreeUpdate {
        self.sent.clear();
        self.scale = scale;
        self.frame = frame;
        let root = tree.first().map(|root| TreeInfo::new(NodeId(root.node)));
        TreeUpdate {
            tree: root,
            ..self.update(tree)
        }
    }

    /// Whether the tree may differ from what was sent: a frame was
    /// prepared since, the tree being the last frame's, or the scale
    /// changed. A turn that did neither needs no tree at all.
    pub(crate) fn behind(&self, frame: u64, scale: f64) -> bool {
        frame != self.frame || scale != self.scale
    }

    /// The nodes of `tree` that differ from what was sent, with the
    /// focus: `None` if none does. The focus is a node's own state, so it
    /// cannot move without a node changing. A new scale sends everything.
    pub(crate) fn changes(
        &mut self,
        tree: Vec<AccessNode>,
        scale: f64,
        frame: u64,
    ) -> Option<TreeUpdate> {
        if scale != self.scale {
            return Some(self.full(tree, scale, frame));
        }
        self.frame = frame;
        let update = self.update(tree);
        (!update.nodes.is_empty()).then_some(update)
    }

    /// The update holding the nodes of `tree` that differ from what was
    /// sent, with the focus; what was sent becomes `tree`, kept node by
    /// node rather than copied: on a large page most of it is the same.
    fn update(&mut self, tree: Vec<AccessNode>) -> TreeUpdate {
        let focus = focus(&tree);
        let ids: Vec<u64> = tree.iter().map(|node| node.node).collect();
        let mut nodes = Vec::new();
        for node in tree {
            if self.sent.get(&node.node) == Some(&node) {
                continue;
            }
            nodes.push((NodeId(node.node), accesskit_node(&node, self.scale)));
            self.sent.insert(node.node, node);
        }
        // Nodes that left the tree: their parents changed and say so.
        if self.sent.len() > ids.len() {
            let kept: HashSet<u64> = ids.into_iter().collect();
            self.sent.retain(|id, _| kept.contains(id));
        }
        TreeUpdate {
            nodes,
            tree: None,
            tree_id: TreeId::ROOT,
            focus,
        }
    }

    /// The middle of `node`'s box in CSS pixels, as last sent: where a
    /// screen reader's click lands.
    pub(crate) fn middle(&self, node: u64) -> Option<(f32, f32)> {
        let (x, y, width, height) = self.sent.get(&node)?.bounds?;
        Some((x + width / 2.0, y + height / 2.0))
    }
}

/// The focused node of `tree`, or its root when nothing is focused.
fn focus(tree: &[AccessNode]) -> NodeId {
    let node = tree
        .iter()
        .find(|node| node.focused)
        .or_else(|| tree.first())
        .map_or(0, |node| node.node);
    NodeId(node)
}

/// `node` as AccessKit's, its box in device pixels at `scale`.
fn accesskit_node(node: &AccessNode, scale: f64) -> Node {
    let role = role(node.role);
    let mut out = Node::new(role);
    if let Some(name) = &node.name {
        // AccessKit reads a label's name from its value (static text,
        // `label_comes_from_value`); a label's own label is never read.
        if role == Role::Label {
            out.set_value(name.as_str());
        } else {
            out.set_label(name.as_str());
        }
    }
    if let Some(value) = &node.value {
        out.set_value(value.as_str());
    }
    if let Some(checked) = node.checked {
        out.set_toggled(Toggled::from(checked));
    }
    if let Some(selected) = node.selected {
        out.set_selected(selected);
    }
    if let Some(expanded) = node.expanded {
        out.set_expanded(expanded);
    }
    if let Some(level) = node.level {
        out.set_level(usize::from(level));
    }
    if node.disabled {
        out.set_disabled();
    }
    if let Some((x, y, width, height)) = node.bounds {
        let (x, y, width, height) = (
            f64::from(x) * scale,
            f64::from(y) * scale,
            f64::from(width) * scale,
            f64::from(height) * scale,
        );
        out.set_bounds(Rect {
            x0: x,
            y0: y,
            x1: x + width,
            y1: y + height,
        });
    }
    if clickable(node.role) && !node.disabled {
        out.add_action(Action::Click);
    }
    out.set_children(
        node.children
            .iter()
            .map(|child| NodeId(*child))
            .collect::<Vec<_>>(),
    );
    out
}

/// Whether a screen reader can click a node of `role`.
fn clickable(role: AccessRole) -> bool {
    matches!(
        role,
        AccessRole::Button
            | AccessRole::Link
            | AccessRole::CheckBox
            | AccessRole::RadioButton
            | AccessRole::ComboBox
            | AccessRole::Option
    )
}

/// AccessKit's role for Erk's.
fn role(role: AccessRole) -> Role {
    match role {
        AccessRole::Document => Role::Document,
        AccessRole::Generic => Role::GenericContainer,
        AccessRole::Text => Role::Label,
        AccessRole::Paragraph => Role::Paragraph,
        AccessRole::Heading => Role::Heading,
        AccessRole::Link => Role::Link,
        AccessRole::Button => Role::Button,
        AccessRole::CheckBox => Role::CheckBox,
        AccessRole::RadioButton => Role::RadioButton,
        AccessRole::TextInput => Role::TextInput,
        AccessRole::MultilineTextInput => Role::MultilineTextInput,
        AccessRole::PasswordInput => Role::PasswordInput,
        AccessRole::ComboBox => Role::ComboBox,
        AccessRole::Option => Role::ListBoxOption,
        AccessRole::Image => Role::Image,
        AccessRole::List => Role::List,
        AccessRole::ListItem => Role::ListItem,
        AccessRole::Dialog => Role::Dialog,
        AccessRole::Group => Role::Group,
        AccessRole::Form => Role::Form,
        AccessRole::Label => Role::Label,
        AccessRole::Navigation => Role::Navigation,
        AccessRole::Main => Role::Main,
        AccessRole::Article => Role::Article,
        AccessRole::Table => Role::Table,
        AccessRole::TableRow => Role::Row,
        AccessRole::TableCell => Role::Cell,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A node of `role` with `children`, otherwise empty.
    fn node(id: u64, role: AccessRole, children: &[u64]) -> AccessNode {
        AccessNode {
            node: id,
            role,
            name: None,
            value: None,
            checked: None,
            selected: None,
            expanded: None,
            level: None,
            disabled: false,
            focused: false,
            bounds: None,
            children: children.to_vec(),
        }
    }

    /// A document holding a button and a checkbox.
    fn page() -> Vec<AccessNode> {
        let mut button = node(2, AccessRole::Button, &[]);
        button.name = Some("Kaydet".to_owned());
        button.bounds = Some((10.0, 20.0, 40.0, 10.0));
        let mut check = node(3, AccessRole::CheckBox, &[]);
        check.checked = Some(false);
        vec![node(1, AccessRole::Document, &[2, 3]), button, check]
    }

    fn ids(update: &TreeUpdate) -> Vec<u64> {
        let mut ids: Vec<u64> = update.nodes.iter().map(|(id, _)| id.0).collect();
        ids.sort_unstable();
        ids
    }

    #[test]
    fn a_screen_reader_that_asks_gets_the_whole_tree_and_its_root() {
        let mut sync = AccessSync::default();
        let update = sync.full(page(), 1.0, 0);
        assert_eq!(ids(&update), [1, 2, 3]);
        assert_eq!(update.tree.map(|tree| tree.root), Some(NodeId(1)));
        // Nothing focused: the root has the focus.
        assert_eq!(update.focus, NodeId(1));
        let button = &update.nodes.iter().find(|(id, _)| id.0 == 2).unwrap().1;
        assert_eq!(button.role(), Role::Button);
        assert_eq!(button.label(), Some("Kaydet"));
        let root = &update.nodes.iter().find(|(id, _)| id.0 == 1).unwrap().1;
        assert_eq!(root.children(), [NodeId(2), NodeId(3)]);
    }

    #[test]
    fn text_reaches_a_screen_reader_as_the_value_its_name_is_read_from() {
        // AccessKit names a label from its value, not its label (Windows'
        // and the other adapters' `label_comes_from_value`): text kept in
        // the label is never read.
        for role in [AccessRole::Text, AccessRole::Label] {
            let mut text = node(1, role, &[]);
            text.name = Some("Kayıt formu".to_owned());
            let out = accesskit_node(&text, 1.0);
            assert_eq!(out.role(), Role::Label, "{role:?}");
            assert_eq!(out.value(), Some("Kayıt formu"), "{role:?}");
        }
        // A control keeps its name in the label and its value apart.
        let mut field = node(2, AccessRole::TextInput, &[]);
        field.name = Some("Adınız".to_owned());
        field.value = Some("Ece".to_owned());
        let out = accesskit_node(&field, 1.0);
        assert_eq!((out.label(), out.value()), (Some("Adınız"), Some("Ece")));
    }

    #[test]
    fn after_the_first_tree_only_what_changed_is_sent() {
        let mut sync = AccessSync::default();
        sync.full(page(), 1.0, 0);
        // The same tree: nothing.
        assert_eq!(sync.changes(page(), 1.0, 1), None);
        // A checkbox checked: that node alone.
        let mut tree = page();
        tree[2].checked = Some(true);
        let update = sync.changes(tree.clone(), 1.0, 2).expect("a change");
        assert_eq!(ids(&update), [3]);
        assert_eq!(update.nodes[0].1.toggled(), Some(Toggled::True));
        assert!(update.tree.is_none());
        // A node added: it and the parent that now holds it.
        tree[0].children.push(4);
        tree.push(node(4, AccessRole::Text, &[]));
        assert_eq!(ids(&sync.changes(tree.clone(), 1.0, 3).unwrap()), [1, 4]);
        // The focus moving: the node that took it, the focus with it.
        tree[1].focused = true;
        let update = sync.changes(tree.clone(), 1.0, 4).expect("the focus moved");
        assert_eq!(ids(&update), [2]);
        assert_eq!(update.focus, NodeId(2));
        // A node removed: its parent, and it is no longer known.
        tree[0].children = vec![3, 4];
        tree.remove(1);
        assert_eq!(ids(&sync.changes(tree.clone(), 1.0, 5).unwrap()), [1]);
        assert_eq!(sync.middle(2), None);
        // Put back as it was: sent again, though it was sent before.
        assert_eq!(ids(&sync.changes(page(), 1.0, 6).unwrap()), [1, 2, 3]);
    }

    #[test]
    fn the_tree_is_taken_again_only_after_a_frame_or_a_new_scale() {
        let mut sync = AccessSync::default();
        sync.full(page(), 1.0, 7);
        assert!(!sync.behind(7, 1.0));
        assert!(sync.behind(8, 1.0));
        assert!(sync.behind(7, 2.0));
        sync.changes(page(), 1.0, 8);
        assert!(!sync.behind(8, 1.0));
        sync.changes(page(), 2.0, 9);
        assert!(!sync.behind(9, 2.0));
    }

    #[test]
    fn an_app_counts_the_frames_it_prepares_and_only_those() {
        let mut app = crate::App::headless(crate::Config {
            system_fonts: false,
            ..crate::Config::default()
        })
        .unwrap();
        app.load_html("<p id=p>bir</p>");
        app.tick(0);
        let first = app.prepared;
        assert!(first > 0);
        // Nothing changed: no frame, and a window's screen reader is not
        // told again.
        app.tick(1);
        assert_eq!(app.prepared, first);
        let p = app.query(None, "#p").unwrap().unwrap();
        app.set_text(p, "iki").unwrap();
        app.tick(2);
        assert_eq!(app.prepared, first + 1);
    }

    #[test]
    fn a_new_scale_sends_every_box_again_in_device_pixels() {
        let mut sync = AccessSync::default();
        sync.full(page(), 1.0, 0);
        let update = sync.changes(page(), 2.0, 0).expect("a new scale");
        assert_eq!(ids(&update), [1, 2, 3]);
        assert!(update.tree.is_some());
        let button = &update.nodes.iter().find(|(id, _)| id.0 == 2).unwrap().1;
        assert_eq!(
            button.bounds(),
            Some(Rect {
                x0: 20.0,
                y0: 40.0,
                x1: 100.0,
                y1: 60.0
            })
        );
        // A click lands in the middle of the box, in CSS pixels.
        assert_eq!(sync.middle(2), Some((30.0, 25.0)));
        assert_eq!(sync.middle(3), None);
    }

    #[test]
    fn a_screen_reader_can_click_what_takes_a_click_unless_it_is_disabled() {
        let mut tree = page();
        tree[2].disabled = true;
        let update = AccessSync::default().full(tree, 1.0, 0);
        let actions = |id: u64| {
            let node = &update
                .nodes
                .iter()
                .find(|(node, _)| node.0 == id)
                .unwrap()
                .1;
            (node.supports_action(Action::Click), node.is_disabled())
        };
        assert_eq!(actions(1), (false, false));
        assert_eq!(actions(2), (true, false));
        assert_eq!(actions(3), (false, true));
    }
}
