use html5ever::{LocalName, QualName, ns};

use crate::NodeId;

/// A node and its links to the rest of the tree.
///
/// Links are ids into the owning document's arena, never pointers, so the
/// tree can be cyclic (parent ↔ child) without reference counting.
pub struct Node {
    pub(crate) parent: Option<NodeId>,
    pub(crate) first_child: Option<NodeId>,
    pub(crate) last_child: Option<NodeId>,
    pub(crate) prev_sibling: Option<NodeId>,
    pub(crate) next_sibling: Option<NodeId>,
    pub data: NodeData,
}

impl Node {
    pub(crate) fn new(data: NodeData) -> Self {
        Self {
            parent: None,
            first_child: None,
            last_child: None,
            prev_sibling: None,
            next_sibling: None,
            data,
        }
    }

    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    pub fn first_child(&self) -> Option<NodeId> {
        self.first_child
    }

    pub fn last_child(&self) -> Option<NodeId> {
        self.last_child
    }

    pub fn prev_sibling(&self) -> Option<NodeId> {
        self.prev_sibling
    }

    pub fn next_sibling(&self) -> Option<NodeId> {
        self.next_sibling
    }

    pub fn as_element(&self) -> Option<&ElementData> {
        match &self.data {
            NodeData::Element(element) => Some(element),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match &self.data {
            NodeData::Text(text) => Some(text),
            _ => None,
        }
    }
}

// Text is stored as String rather than html5ever's StrTendril: tendrils are
// not Send, and Stylo's parallel traversal needs a DOM it can share across
// threads.
pub enum NodeData {
    Document,
    DocumentFragment,
    Doctype {
        name: String,
        public_id: String,
        system_id: String,
    },
    Element(ElementData),
    Text(String),
    Comment(String),
    ProcessingInstruction {
        target: String,
        data: String,
    },
}

pub struct ElementData {
    pub name: QualName,
    pub attrs: Vec<Attribute>,
    /// The document fragment holding a `<template>`'s contents.
    pub(crate) template_contents: Option<NodeId>,
    pub(crate) mathml_annotation_xml_integration_point: bool,
}

impl ElementData {
    pub(crate) fn new(name: QualName, attrs: Vec<Attribute>) -> Self {
        Self {
            name,
            attrs,
            template_contents: None,
            mathml_annotation_xml_integration_point: false,
        }
    }

    /// The value of an attribute in no namespace, such as `id` or `class`.
    pub fn attr(&self, local: &LocalName) -> Option<&str> {
        self.attrs
            .iter()
            .find(|attr| attr.name.ns == ns!() && attr.name.local == *local)
            .map(|attr| attr.value.as_str())
    }

    pub fn template_contents(&self) -> Option<NodeId> {
        self.template_contents
    }
}

pub struct Attribute {
    pub name: QualName,
    pub value: String,
}

impl From<html5ever::Attribute> for Attribute {
    fn from(attr: html5ever::Attribute) -> Self {
        Self {
            name: attr.name,
            value: attr.value.to_string(),
        }
    }
}
