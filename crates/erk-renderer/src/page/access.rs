//! The accessibility tree (M5.11's first step).
//!
//! What a screen reader is told about the page, read from the last frame:
//! the document, its elements and runs of text, each with a role, a name,
//! a value, states and the box it was laid out in (p1-contract §6.1). An
//! element's role is its `role` attribute when Erk knows the role, or else
//! the one its element implies (HTML-AAM). Its name follows a subset of
//! accname: `aria-labelledby`, `aria-label`, a control's `label`, an
//! image's `alt`, the text a button, link, heading, option or label holds,
//! and last `title`.
//!
//! What has no box (`display: none`), is `visibility: hidden` or
//! `aria-hidden="true"` is not in the tree, nor what it holds. A button or
//! an option is a leaf: its text is its name. The embedding layer turns the
//! tree into a platform's (AccessKit, the next step).

use erk_dom::{NodeData, NodeId, local_name};
use erk_style::style::computed_values::visibility::T as Visibility;
use erk_style::style::values::computed::Display;

use super::Page;
use crate::messages::{AccessNode, AccessRole};

/// Elements that never show and so are never in the tree.
const UNSHOWN: [&str; 8] = [
    "head", "script", "style", "template", "title", "meta", "link", "noscript",
];

impl Page {
    /// The accessibility tree of the last frame, root first, each node
    /// before what it holds. Only the document node before the first
    /// frame.
    pub(crate) fn access_tree(&self) -> Vec<AccessNode> {
        let mut out = Vec::new();
        self.access_node(self.doc.root(), &mut out);
        out
    }

    /// Add `node`, if it is in the tree, and what it holds to `out`; its id
    /// if it was added.
    fn access_node(&self, node: NodeId, out: &mut Vec<AccessNode>) -> Option<u64> {
        let found = self.doc.node(node)?;
        let role = match &found.data {
            NodeData::Document => AccessRole::Document,
            NodeData::Text(text) => {
                let text = collapse(text);
                if text.is_empty() {
                    return None;
                }
                out.push(AccessNode {
                    node: node.to_bits(),
                    role: AccessRole::Text,
                    name: Some(text),
                    value: None,
                    checked: None,
                    selected: None,
                    expanded: None,
                    level: None,
                    disabled: false,
                    focused: false,
                    bounds: None,
                    children: Vec::new(),
                });
                return Some(node.to_bits());
            }
            NodeData::Element(_) => {
                if !self.shown_to_access(node) {
                    return None;
                }
                self.access_role(node)
            }
            _ => return None,
        };
        let at = out.len();
        out.push(AccessNode {
            node: node.to_bits(),
            role,
            name: self.access_name(node, role),
            value: self.access_value(node, role),
            checked: matches!(role, AccessRole::CheckBox | AccessRole::RadioButton)
                .then(|| self.doc.checked(node) == Ok(Some(true))),
            selected: (role == AccessRole::Option)
                .then(|| self.doc.checked(node) == Ok(Some(true))),
            expanded: self.access_expanded(node, role),
            level: self.heading_level(node, role),
            disabled: self.doc.disabled(node) == Some(true)
                || self.attribute(node, "aria-disabled").as_deref() == Some("true"),
            focused: self.focus == Some(node),
            bounds: self.access_bounds(node),
            children: Vec::new(),
        });
        // A button and an option are leaves, their text their name; a text
        // field shows its value, never what it holds; a select holds its
        // options.
        let leaf = matches!(
            role,
            AccessRole::Button
                | AccessRole::Option
                | AccessRole::TextInput
                | AccessRole::MultilineTextInput
                | AccessRole::PasswordInput
                | AccessRole::CheckBox
                | AccessRole::RadioButton
                | AccessRole::Image
        );
        if !leaf {
            let children: Vec<NodeId> = if role == AccessRole::ComboBox {
                self.doc.options(node)
            } else {
                self.doc.children(node).collect()
            };
            let mut ids = Vec::new();
            for child in children {
                ids.extend(self.access_node(child, out));
            }
            out[at].children = ids;
        }
        Some(node.to_bits())
    }

    /// Whether element `node` can be in the tree: it is styled (nothing
    /// above it is `display: none`), shows, and is not hidden from
    /// assistive technology. An option shows through its select.
    fn shown_to_access(&self, node: NodeId) -> bool {
        let Some(element) = self.doc.node(node).and_then(|found| found.as_element()) else {
            return false;
        };
        if UNSHOWN.contains(&element.name.local.as_ref())
            || self.attribute(node, "aria-hidden").as_deref() == Some("true")
        {
            return false;
        }
        if element.name.local == local_name!("option") {
            return true;
        }
        self.styles.computed(node).is_some_and(|style| {
            style.get_box().clone_display() != Display::None
                && style.get_inherited_box().clone_visibility() == Visibility::Visible
        })
    }

    /// Element `node`'s role: its `role` attribute's first role Erk knows,
    /// or else its implicit one.
    fn access_role(&self, node: NodeId) -> AccessRole {
        if let Some(role) = self
            .attribute(node, "role")
            .and_then(|value| value.split_ascii_whitespace().find_map(explicit_role))
        {
            return role;
        }
        let Some(element) = self.doc.node(node).and_then(|found| found.as_element()) else {
            return AccessRole::Generic;
        };
        if element.name.ns != erk_dom::ns!(html) {
            return AccessRole::Generic;
        }
        match element.name.local.as_ref() {
            "p" => AccessRole::Paragraph,
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => AccessRole::Heading,
            "a" if element.attr(&local_name!("href")).is_some() => AccessRole::Link,
            "button" | "summary" => AccessRole::Button,
            "textarea" => AccessRole::MultilineTextInput,
            "select" if crate::layout::is_dropdown(&self.doc, node) => AccessRole::ComboBox,
            "option" => AccessRole::Option,
            "img" => AccessRole::Image,
            "ul" | "ol" | "menu" => AccessRole::List,
            "li" => AccessRole::ListItem,
            "dialog" => AccessRole::Dialog,
            "fieldset" | "details" | "optgroup" => AccessRole::Group,
            "form" => AccessRole::Form,
            "label" => AccessRole::Label,
            "nav" => AccessRole::Navigation,
            "main" => AccessRole::Main,
            "article" => AccessRole::Article,
            "table" => AccessRole::Table,
            "tr" => AccessRole::TableRow,
            "td" | "th" => AccessRole::TableCell,
            "input" => match element
                .attr(&local_name!("type"))
                .unwrap_or("text")
                .to_ascii_lowercase()
                .as_str()
            {
                "checkbox" => AccessRole::CheckBox,
                "radio" => AccessRole::RadioButton,
                "submit" | "reset" | "button" | "image" => AccessRole::Button,
                "password" => AccessRole::PasswordInput,
                "hidden" => AccessRole::Generic,
                _ => AccessRole::TextInput,
            },
            _ => AccessRole::Generic,
        }
    }

    /// Element `node`'s name, as `role` takes one (a subset of accname).
    fn access_name(&self, node: NodeId, role: AccessRole) -> Option<String> {
        let non_empty = |text: String| {
            let text = collapse(&text);
            (!text.is_empty()).then_some(text)
        };
        if let Some(ids) = self.attribute(node, "aria-labelledby") {
            let text: Vec<String> = ids
                .split_ascii_whitespace()
                .filter_map(|id| self.element_with_id(id))
                .map(|labelling| self.text_within(labelling, None))
                .collect();
            if let Some(name) = non_empty(text.join(" ")) {
                return Some(name);
            }
        }
        if let Some(name) = self.attribute(node, "aria-label").and_then(non_empty) {
            return Some(name);
        }
        let name = match role {
            AccessRole::CheckBox
            | AccessRole::RadioButton
            | AccessRole::TextInput
            | AccessRole::MultilineTextInput
            | AccessRole::PasswordInput
            | AccessRole::ComboBox => self.label_text(node),
            AccessRole::Button => crate::layout::button_label(&self.doc, node)
                .or_else(|| Some(self.text_within(node, None))),
            AccessRole::Image => self.attribute(node, "alt"),
            AccessRole::Option => Some(self.doc.option_label(node)),
            AccessRole::Link | AccessRole::Heading | AccessRole::Label => {
                Some(self.text_within(node, None))
            }
            AccessRole::Group if self.is_named(node, "optgroup") => self.attribute(node, "label"),
            AccessRole::Document => self
                .doc
                .children(node)
                .find(|child| self.is_named(*child, "html"))
                .and_then(|html| self.title_text(html)),
            _ => None,
        };
        name.and_then(non_empty)
            .or_else(|| self.attribute(node, "title").and_then(non_empty))
    }

    /// The text of the labels of control `node`: those whose `for` names
    /// it, or that hold it with no `for`, each without the text of the
    /// control itself.
    fn label_text(&self, control: NodeId) -> Option<String> {
        let mut labels = Vec::new();
        let mut stack = vec![self.doc.root()];
        while let Some(id) = stack.pop() {
            let mut children: Vec<NodeId> = self.doc.children(id).collect();
            children.reverse();
            stack.extend(children);
            if self.labeled_control(id) == Some(control) {
                labels.push(self.text_within(id, Some(control)));
            }
        }
        (!labels.is_empty()).then(|| labels.join(" "))
    }

    /// The text `node` holds, in tree order, without what `skip` holds or
    /// what does not show. A control in it counts by its value, as accname
    /// has it: a select by its selected option, a text field by what is
    /// typed in it.
    fn text_within(&self, node: NodeId, skip: Option<NodeId>) -> String {
        let mut text = String::new();
        let mut stack = vec![node];
        while let Some(id) = stack.pop() {
            if Some(id) == skip {
                continue;
            }
            if id != node {
                let embedded = match self.access_role(id) {
                    AccessRole::ComboBox => Some(self.access_value(id, AccessRole::ComboBox)),
                    AccessRole::TextInput | AccessRole::MultilineTextInput => {
                        Some(self.doc.value(id).ok().flatten())
                    }
                    _ => None,
                };
                if let Some(value) = embedded {
                    if self.shown_to_access(id) {
                        text.extend(value);
                        text.push(' ');
                    }
                    continue;
                }
            }
            match self.doc.node(id).map(|found| &found.data) {
                Some(NodeData::Text(own)) => {
                    text.push_str(own);
                    text.push(' ');
                }
                Some(NodeData::Element(_)) if id == node || self.shown_to_access(id) => {
                    let mut children: Vec<NodeId> = self.doc.children(id).collect();
                    children.reverse();
                    stack.extend(children);
                }
                _ => {}
            }
        }
        collapse(&text)
    }

    /// The text of the document's `title` under `html`, if there is one.
    fn title_text(&self, html: NodeId) -> Option<String> {
        let head = self
            .doc
            .children(html)
            .find(|child| self.is_named(*child, "head"))?;
        let title = self
            .doc
            .children(head)
            .find(|child| self.is_named(*child, "title"))?;
        Some(self.text_within(title, None))
    }

    /// A text field's value, a select's selected option's label.
    fn access_value(&self, node: NodeId, role: AccessRole) -> Option<String> {
        match role {
            AccessRole::TextInput | AccessRole::MultilineTextInput => {
                self.doc.value(node).ok().flatten()
            }
            AccessRole::ComboBox => self
                .doc
                .selected_options(node)
                .first()
                .map(|option| self.doc.option_label(*option)),
            _ => None,
        }
    }

    /// Whether a `details`, its summary, or a select's list is open.
    fn access_expanded(&self, node: NodeId, role: AccessRole) -> Option<bool> {
        if self.is_named(node, "details") {
            return Some(self.attribute(node, "open").is_some());
        }
        if let Some(details) = self.details_of_summary(node) {
            return Some(self.attribute(details, "open").is_some());
        }
        (role == AccessRole::ComboBox)
            .then(|| self.open_picker().is_some_and(|(select, _)| select == node))
    }

    /// A heading's level: its element's number, or `aria-level`.
    fn heading_level(&self, node: NodeId, role: AccessRole) -> Option<u8> {
        if role != AccessRole::Heading {
            return None;
        }
        if let Some(level) = self
            .attribute(node, "aria-level")
            .and_then(|level| level.trim().parse::<u8>().ok())
            .filter(|level| (1..=6).contains(level))
        {
            return Some(level);
        }
        let element = self.doc.node(node)?.as_element()?;
        element
            .name
            .local
            .strip_prefix('h')
            .and_then(|digit| digit.parse::<u8>().ok())
            .filter(|level| (1..=6).contains(level))
            .or(Some(2))
    }

    /// `node`'s border box on the viewport in the last frame.
    fn access_bounds(&self, node: NodeId) -> Option<(f32, f32, f32, f32)> {
        let found = self.node_box(node.to_bits()).ok().flatten()?;
        Some((found.x, found.y, found.width, found.height))
    }

    /// Attribute `name` of element `node`.
    fn attribute(&self, node: NodeId, name: &str) -> Option<String> {
        let local = erk_dom::LocalName::from(name);
        self.doc
            .node(node)?
            .as_element()?
            .attr(&local)
            .map(str::to_owned)
    }

    /// Whether `node` is an HTML element named `name`.
    fn is_named(&self, node: NodeId, name: &str) -> bool {
        self.doc
            .node(node)
            .and_then(|found| found.as_element())
            .is_some_and(|element| {
                element.name.ns == erk_dom::ns!(html) && element.name.local.as_ref() == name
            })
    }
}

/// The role a `role` attribute names, if Erk knows it; `none` and
/// `presentation` are generic.
fn explicit_role(name: &str) -> Option<AccessRole> {
    Some(match name.to_ascii_lowercase().as_str() {
        "button" => AccessRole::Button,
        "checkbox" | "switch" => AccessRole::CheckBox,
        "radio" => AccessRole::RadioButton,
        "textbox" => AccessRole::TextInput,
        "combobox" => AccessRole::ComboBox,
        "option" => AccessRole::Option,
        "img" | "image" => AccessRole::Image,
        "list" => AccessRole::List,
        "listitem" => AccessRole::ListItem,
        "dialog" | "alertdialog" => AccessRole::Dialog,
        "group" => AccessRole::Group,
        "form" => AccessRole::Form,
        "navigation" => AccessRole::Navigation,
        "main" => AccessRole::Main,
        "article" => AccessRole::Article,
        "heading" => AccessRole::Heading,
        "link" => AccessRole::Link,
        "paragraph" => AccessRole::Paragraph,
        "table" => AccessRole::Table,
        "row" => AccessRole::TableRow,
        "cell" | "gridcell" => AccessRole::TableCell,
        "generic" | "none" | "presentation" => AccessRole::Generic,
        _ => return None,
    })
}

/// `text` with its whitespace stripped and collapsed.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
