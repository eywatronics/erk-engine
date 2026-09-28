use std::borrow::Cow;
use std::cell::{Ref, RefCell};

use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{Attribute as HtmlAttribute, ParseOpts, QualName, parse_document};

use crate::{Document, ElementData, NodeData, NodeId};

impl Document {
    /// Parse a complete HTML document.
    pub fn parse_html(html: &str) -> Self {
        parse_document(Sink::default(), ParseOpts::default()).one(html)
    }
}

// html5ever's TreeSink methods take &self, so the document sits behind a
// RefCell. RefCell is fine here; what the DOM forbids is owning nodes through
// reference counting.
#[derive(Default)]
struct Sink {
    doc: RefCell<Document>,
}

impl Sink {
    fn append_text(doc: &mut Document, parent: NodeId, text: &str) {
        if let Some(last) = doc.get(parent).last_child
            && let NodeData::Text(existing) = &mut doc.get_mut(last).data
        {
            existing.push_str(text);
            return;
        }
        let id = doc.create(NodeData::Text(text.to_owned()));
        doc.append(parent, id);
    }
}

impl TreeSink for Sink {
    type Handle = NodeId;
    type Output = Document;
    type ElemName<'a> = Ref<'a, QualName>;

    fn finish(self) -> Document {
        self.doc.into_inner()
    }

    // Parse errors are recoverable by definition; the tree builder has
    // already applied the spec's recovery. Nothing to surface yet.
    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> NodeId {
        self.doc.borrow().root()
    }

    fn elem_name<'a>(&'a self, target: &'a NodeId) -> Ref<'a, QualName> {
        Ref::map(self.doc.borrow(), |doc| match &doc.get(*target).data {
            NodeData::Element(element) => &element.name,
            _ => panic!("elem_name called on a non-element node"),
        })
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<HtmlAttribute>,
        flags: ElementFlags,
    ) -> NodeId {
        let mut doc = self.doc.borrow_mut();
        let mut element = ElementData::new(name, attrs.into_iter().map(Into::into).collect());
        element.mathml_annotation_xml_integration_point =
            flags.mathml_annotation_xml_integration_point;
        if flags.template {
            element.template_contents = Some(doc.create(NodeData::DocumentFragment));
        }
        doc.create(NodeData::Element(element))
    }

    fn create_comment(&self, text: StrTendril) -> NodeId {
        self.doc
            .borrow_mut()
            .create(NodeData::Comment(text.to_string()))
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> NodeId {
        self.doc
            .borrow_mut()
            .create(NodeData::ProcessingInstruction {
                target: target.to_string(),
                data: data.to_string(),
            })
    }

    fn append(&self, parent: &NodeId, child: NodeOrText<NodeId>) {
        let mut doc = self.doc.borrow_mut();
        match child {
            NodeOrText::AppendNode(node) => doc.append(*parent, node),
            NodeOrText::AppendText(text) => Self::append_text(&mut doc, *parent, &text),
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &NodeId,
        prev_element: &NodeId,
        child: NodeOrText<NodeId>,
    ) {
        let has_parent = self.doc.borrow().get(*element).parent.is_some();
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        let mut doc = self.doc.borrow_mut();
        let doctype = doc.create(NodeData::Doctype {
            name: name.to_string(),
            public_id: public_id.to_string(),
            system_id: system_id.to_string(),
        });
        let root = doc.root();
        doc.append(root, doctype);
    }

    fn get_template_contents(&self, target: &NodeId) -> NodeId {
        self.doc
            .borrow()
            .get(*target)
            .as_element()
            .and_then(|element| element.template_contents)
            .expect("get_template_contents called on a non-template element")
    }

    fn same_node(&self, x: &NodeId, y: &NodeId) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.doc.borrow_mut().quirks_mode = mode;
    }

    fn append_before_sibling(&self, sibling: &NodeId, new_node: NodeOrText<NodeId>) {
        let mut doc = self.doc.borrow_mut();
        match new_node {
            NodeOrText::AppendNode(node) => doc.insert_before(*sibling, node),
            NodeOrText::AppendText(text) => {
                // Merge with a text node that already precedes the sibling,
                // as append() does with a trailing one.
                if let Some(prev) = doc.get(*sibling).prev_sibling
                    && let NodeData::Text(existing) = &mut doc.get_mut(prev).data
                {
                    existing.push_str(&text);
                    return;
                }
                let id = doc.create(NodeData::Text(text.to_string()));
                doc.insert_before(*sibling, id);
            }
        }
    }

    fn add_attrs_if_missing(&self, target: &NodeId, attrs: Vec<HtmlAttribute>) {
        let mut doc = self.doc.borrow_mut();
        let NodeData::Element(element) = &mut doc.get_mut(*target).data else {
            panic!("add_attrs_if_missing called on a non-element node");
        };
        for attr in attrs {
            if !element
                .attrs
                .iter()
                .any(|existing| existing.name == attr.name)
            {
                element.attrs.push(attr.into());
            }
        }
    }

    fn remove_from_parent(&self, target: &NodeId) {
        self.doc.borrow_mut().detach(*target);
    }

    fn reparent_children(&self, node: &NodeId, new_parent: &NodeId) {
        let mut doc = self.doc.borrow_mut();
        while let Some(child) = doc.get(*node).first_child {
            doc.append(*new_parent, child);
        }
    }

    fn is_mathml_annotation_xml_integration_point(&self, handle: &NodeId) -> bool {
        self.doc
            .borrow()
            .get(*handle)
            .as_element()
            .is_some_and(|element| element.mathml_annotation_xml_integration_point)
    }
}
