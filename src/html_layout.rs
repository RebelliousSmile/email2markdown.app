//! Email-specific HTML handlers plugged into the `htmd` converter.
//!
//! Email HTML is built from layout tables and inline CSS rather than semantic
//! markup. These handlers recover the structure `htmd` cannot see:
//! - layout tables: cells of a row stay on one line instead of one paragraph each
//! - `font-weight` CSS: rendered as Markdown bold
//! - links wrapping only a tracking pixel: dropped instead of leaving `[](url)`

use htmd::element_handler::{HandlerResult, Handlers};
use htmd::{Element, Node};
use markup5ever_rcdom::NodeData;
use std::rc::Rc;

fn tag_name(node: &Rc<Node>) -> Option<String> {
    match &node.data {
        NodeData::Element { name, .. } => Some(name.local.as_ref().to_string()),
        _ => None,
    }
}

fn parent(node: &Rc<Node>) -> Option<Rc<Node>> {
    let weak = node.parent.take();
    node.parent.set(weak.clone());
    weak.and_then(|w| w.upgrade())
}

fn attr_of(node: &Rc<Node>, attr: &str) -> Option<String> {
    match &node.data {
        NodeData::Element { attrs, .. } => attrs
            .borrow()
            .iter()
            .find(|a| a.name.local.as_ref() == attr)
            .map(|a| a.value.to_string()),
        _ => None,
    }
}

fn style_of(node: &Rc<Node>) -> Option<String> {
    attr_of(node, "style")
}

/// True when an ancestor is a mail-client signature block: an `id` or `class`
/// containing `signature` (Gmail `gmail_signature`, Outlook web `Signature`,
/// Thunderbird `moz-signature`, Apple Mail `AppleMailSignature`).
pub(crate) fn in_signature_container(node: &Rc<Node>) -> bool {
    let mut current = parent(node);
    while let Some(ancestor) = current {
        let marked = ["id", "class"].iter().any(|attr| {
            attr_of(&ancestor, attr)
                .is_some_and(|value| value.to_ascii_lowercase().contains("signature"))
        });
        if marked {
            return true;
        }
        current = parent(&ancestor);
    }
    false
}

/// True for a `font-weight` declaration of `bold`, `bolder` or a weight >= 600.
pub(crate) fn is_bold_style(style: &str) -> bool {
    style.to_ascii_lowercase().split(';').any(|decl| {
        let Some((key, value)) = decl.split_once(':') else {
            return false;
        };
        if key.trim() != "font-weight" {
            return false;
        }
        let value = value.trim().trim_end_matches("!important").trim();
        value == "bold"
            || value == "bolder"
            || value.parse::<u32>().is_ok_and(|weight| weight >= 600)
    })
}

fn has_bold_ancestor(node: &Rc<Node>) -> bool {
    let mut current = parent(node);
    while let Some(ancestor) = current {
        if matches!(tag_name(&ancestor).as_deref(), Some("b" | "strong")) {
            return true;
        }
        if style_of(&ancestor).is_some_and(|s| is_bold_style(&s)) {
            return true;
        }
        current = parent(&ancestor);
    }
    false
}

/// `<th>`/`<thead>` among the descendants, nested tables excluded.
fn has_explicit_headers(table: &Rc<Node>) -> bool {
    fn visit(node: &Rc<Node>) -> bool {
        node.children
            .borrow()
            .iter()
            .any(|child| match tag_name(child).as_deref() {
                Some("th" | "thead") => true,
                Some("table") => false,
                _ => visit(child),
            })
    }
    visit(table)
}

/// A table with no header row, or nested in another table's cell, only lays
/// content out. Mirrors the rule `htmd` uses to avoid emitting a Markdown table.
fn is_layout_table(table: &Rc<Node>) -> bool {
    if !has_explicit_headers(table) {
        return true;
    }
    let mut current = parent(table);
    while let Some(ancestor) = current {
        if matches!(tag_name(&ancestor).as_deref(), Some("td" | "th")) {
            return true;
        }
        current = parent(&ancestor);
    }
    false
}

fn in_layout_table(node: &Rc<Node>) -> bool {
    let mut current = parent(node);
    while let Some(ancestor) = current {
        if tag_name(&ancestor).as_deref() == Some("table") {
            return is_layout_table(&ancestor);
        }
        current = parent(&ancestor);
    }
    false
}

/// Wrap `content` in `**` when it is a single inline run. Surrounding
/// whitespace stays outside the markers.
fn bolden(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() || trimmed.contains('\n') || trimmed.contains("**") {
        return None;
    }
    let start = content.len() - content.trim_start().len();
    let end = content.trim_end().len();
    Some(format!(
        "{}**{}**{}",
        &content[..start],
        trimmed,
        &content[end..]
    ))
}

fn styled_bold(element: &Element) -> bool {
    style_of(element.node).is_some_and(|s| is_bold_style(&s)) && !has_bold_ancestor(element.node)
}

/// `span` / `font` carrying bold CSS: inline, wrap the content.
pub(crate) fn inline_style_handler(
    handlers: &dyn Handlers,
    element: Element,
) -> Option<HandlerResult> {
    if !styled_bold(&element) {
        return handlers.fallback(element);
    }
    let content = handlers
        .walk_children(element.node)
        .content
        .trim_matches('\n')
        .to_string();
    Some(bolden(&content).unwrap_or(content).into())
}

/// `p` / `div` carrying bold CSS: block, wrap the single-line content.
pub(crate) fn block_style_handler(
    handlers: &dyn Handlers,
    element: Element,
) -> Option<HandlerResult> {
    let bold = styled_bold(&element);
    let result = handlers.fallback(element)?;
    if !bold {
        return Some(result);
    }
    match bolden(result.content.trim_matches('\n')) {
        Some(wrapped) => Some(format!("\n\n{wrapped}\n\n").into()),
        None => Some(result),
    }
}

/// Cell of a layout table: its content, trimmed. The row joins the cells.
pub(crate) fn cell_handler(handlers: &dyn Handlers, element: Element) -> Option<HandlerResult> {
    if !in_layout_table(element.node) {
        return handlers.fallback(element);
    }
    let bold = styled_bold(&element);
    let content = handlers.walk_children(element.node).content;
    let content = content.trim_matches(|c: char| c.is_whitespace());
    let content = match bold.then(|| bolden(content)).flatten() {
        Some(wrapped) => wrapped,
        None => content.to_string(),
    };
    Some(content.into())
}

/// Row of a layout table: single-line cells share one line, a multi-line cell
/// stays a block of its own.
pub(crate) fn row_handler(handlers: &dyn Handlers, element: Element) -> Option<HandlerResult> {
    if !in_layout_table(element.node) {
        return handlers.fallback(element);
    }
    let cells: Vec<Rc<Node>> = element
        .node
        .children
        .borrow()
        .iter()
        .filter(|child| matches!(tag_name(child).as_deref(), Some("td" | "th")))
        .cloned()
        .collect();
    let mut out = String::new();
    let mut inline_open = false;
    for cell in cells {
        let Some(result) = handlers.handle(&cell) else {
            continue;
        };
        let content = result.content.trim_matches(|c: char| c.is_whitespace());
        if content.is_empty() {
            continue;
        }
        if content.contains('\n') {
            out.push_str("\n\n");
            out.push_str(content);
            out.push_str("\n\n");
            inline_open = false;
        } else {
            if inline_open {
                out.push(' ');
            }
            out.push_str(content);
            inline_open = true;
        }
    }
    let out = out.trim_matches('\n');
    if out.is_empty() {
        return Some(String::new().into());
    }
    Some(format!("\n\n{out}\n\n").into())
}

/// A link whose only content is tracking pixels would render as `[](url)`.
pub(crate) fn anchor_handler(handlers: &dyn Handlers, element: Element) -> Option<HandlerResult> {
    let only_pixels = {
        let children = element.node.children.borrow();
        !children.is_empty()
            && children.iter().all(|child| match &child.data {
                NodeData::Text { contents } => contents.borrow().trim().is_empty(),
                NodeData::Element { name, attrs, .. } => {
                    name.local.as_ref() == "img" && {
                        let attrs = attrs.borrow();
                        let pairs: Vec<(&str, &str)> = attrs
                            .iter()
                            .map(|a| (a.name.local.as_ref(), a.value.as_ref()))
                            .collect();
                        crate::email_export::is_tracking_pixel(&pairs)
                    }
                }
                _ => true,
            })
    };
    if only_pixels {
        return Some(String::new().into());
    }
    handlers.fallback(element)
}
