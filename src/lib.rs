//! Floem window entry for Tish: installs a [`tishlang_ui::Host`] that maps committed vnodes to Floem views.
//!
//! UI is **defined in Tish/JSX**; this crate maps intrinsics to Floem. Where possible, vnode props match
//! what a DOM backend would accept (`style` strings/objects, `id`, etc.) so the same JSX can target
//! multiple hosts; Floem-specific tuning stays inside this adapter.
//!
//! ## Resource model (scalability)
//! Each commit builds a **full** Floem subtree for the vnode tree: scrolling or `hide()` does **not** unmount
//! off-screen views. For large UIs, reduce what you mount (routing / conditional JSX when state is available),
//! use the `tabpanel` intrinsic so inactive panels skip building children (see `tabpanel_view`),
//! and prefer Floem’s virtualized primitives for huge lists (not yet wired as a Tish intrinsic).
//! Baseline RAM is dominated by Floem, GPU, and fonts—not the Tish runtime.

use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use floem::action;
use floem::event::{listener, EventPropagation};
use floem::peniko::Color;
use floem::peniko::color::palette::css;
use floem::peniko::kurbo::Point;
use floem::prelude::*;
use floem::style::CustomStylable;
use floem::text::{Attrs, AttrsList, FontWeight};
use floem::views::dropdown::Dropdown;
use floem::views::editor::text::{default_dark_color, default_light_theme};
use floem::views::slider::Slider;
use floem::views::RadioButton;
use floem::taffy::style::AlignItems;
use floem::unit::PxPctAuto;
use floem::ViewId;
use floem::AnyView;
use winit::window::Theme;
use lapce_xi_rope::Rope;
use tishlang_core::{ObjectMap, Value};
use tishlang_ui::{install_thread_local_host, Host, FRAGMENT_SENTINEL};

mod html_css;

// Latest OS dark-mode hint from the window (`None` until the first `ThemeChanged`).
thread_local! {
    static OS_THEME_IS_DARK: RefCell<Option<RwSignal<Option<bool>>>> = const { RefCell::new(None) };
}

// `window.scrollTo` / pixel scroll target for the vnode marked `scrollRoot` (CSSOM view `Window`).
thread_local! {
    static SCROLL_ROOT_PIXEL: RefCell<Option<RwSignal<Option<Point>>>> = const { RefCell::new(None) };
}

thread_local! {
    static SCROLL_ANCHOR_VIEWS: RefCell<HashMap<String, ViewId>> = RefCell::new(HashMap::new());
}

// Shared selection for `<input type="radio" name="…">` (cleared each commit).
thread_local! {
    static RADIO_GROUP_SEL: RefCell<HashMap<String, RwSignal<i32>>> = RefCell::new(HashMap::new());
}

pub(crate) fn register_scroll_anchor(key: String, id: ViewId) {
    SCROLL_ANCHOR_VIEWS.with(|m| {
        m.borrow_mut().insert(key, id);
    });
}

fn clear_scroll_anchors() {
    SCROLL_ANCHOR_VIEWS.with(|m| m.borrow_mut().clear());
}

fn clear_radio_groups() {
    RADIO_GROUP_SEL.with(|c| c.borrow_mut().clear());
}

/// Holds the latest root vnode; [`FloemHost::commit_root`] updates it so the UI can react.
pub struct FloemHost {
    root: RwSignal<Value>,
}

impl FloemHost {
    pub fn new(root: RwSignal<Value>) -> Self {
        Self { root }
    }
}

impl Host for FloemHost {
    fn commit_root(&mut self, vnode: &Value) {
        clear_scroll_anchors();
        clear_radio_groups();
        self.root.set(vnode.clone());
    }
}

fn vnode_children(obj: &ObjectMap) -> Vec<Value> {
    match obj.get("children") {
        Some(Value::Array(a)) => a.borrow().clone(),
        _ => vec![],
    }
}

fn vnode_props(obj: &ObjectMap) -> ObjectMap {
    match obj.get("props") {
        Some(Value::Object(p)) => p.borrow().clone(),
        _ => ObjectMap::default(),
    }
}

fn is_fragment_object(obj: &ObjectMap) -> bool {
    matches!(
        obj.get("tag"),
        Some(Value::String(s)) if s.as_ref() == FRAGMENT_SENTINEL
    )
}

pub(crate) fn props_string(props: &ObjectMap, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(Value::String(s)) = props.get(*k) {
            return Some(s.as_ref().to_string());
        }
    }
    None
}

/// `appearance` / `theme` on the vnode (including values merged from an ancestor [`ThemeProvider`]),
/// else `"system"`.
pub(crate) fn resolve_appearance(props: &ObjectMap) -> String {
    props_string(props, &["appearance", "theme"]).unwrap_or_else(|| "system".to_string())
}

fn theme_provider_resolved_value(props: &ObjectMap) -> String {
    props_string(props, &["appearance", "theme"])
        .or_else(|| match props.get("value") {
            Some(Value::String(s)) => Some(s.as_ref().to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "system".to_string())
}

/// When a subtree is under [`ThemeProvider`], the resolved appearance is copied onto descendant
/// vnode props so `style { … }` closures (which run after the provider returns) still see it.
fn merge_inherited_appearance(mut props: ObjectMap, inherited: Option<&str>) -> ObjectMap {
    if let Some(inn) = inherited {
        if props_string(&props, &["appearance", "theme"]).is_none() {
            props.insert(Arc::from("appearance"), Value::String(inn.to_string().into()));
        }
    }
    props
}

pub(crate) fn props_f64(props: &ObjectMap, keys: &[&str], default: f64) -> f64 {
    for k in keys {
        if let Some(Value::Number(n)) = props.get(*k) {
            return *n;
        }
    }
    default
}

pub(crate) fn props_bool(props: &ObjectMap, keys: &[&str]) -> bool {
    for k in keys {
        match props.get(*k) {
            Some(Value::Bool(b)) => return *b,
            Some(Value::Number(n)) => return *n != 0.0,
            _ => {}
        }
    }
    false
}

#[derive(Clone, Copy)]
pub(crate) struct ThemePalette {
    pub(crate) bg: Color,
    pub(crate) fg: Color,
    pub(crate) border: Color,
    pub(crate) panel: Color,
    pub(crate) sidebar: Color,
}

fn palette_for_dark(dark: bool) -> ThemePalette {
    if dark {
        ThemePalette {
            bg: Color::from_rgb8(0x1e, 0x22, 0x2a),
            fg: Color::from_rgb8(0xe6, 0xe8, 0xef),
            border: Color::from_rgb8(0x3a, 0x40, 0x4d),
            panel: Color::from_rgb8(0x25, 0x2a, 0x34),
            sidebar: Color::from_rgb8(0x17, 0x1a, 0x21),
        }
    } else {
        ThemePalette {
            bg: Color::from_rgb8(0xf6, 0xf7, 0xfb),
            fg: Color::from_rgb8(0x22, 0x24, 0x2d),
            border: Color::from_rgb8(0xd4, 0xd7, 0xe3),
            panel: css::WHITE,
            sidebar: Color::from_rgb8(0xef, 0xf0, 0xf6),
        }
    }
}

/// Resolved [`ThemePalette`] for HTML/CSS elements (honours [`ThemeProvider`] / `appearance` on the vnode).
pub(crate) fn theme_palette_for_vnode(props: &ObjectMap) -> ThemePalette {
    let appearance = resolve_appearance(props);
    palette_for_dark(effective_dark_from_appearance(appearance.as_str()))
}

fn effective_dark_from_appearance(appearance: &str) -> bool {
    let os = OS_THEME_IS_DARK.with(|c| c.borrow().as_ref().and_then(|s| s.get()));
    match appearance {
        "light" => false,
        "dark" => true,
        _ => os.unwrap_or(false),
    }
}

#[derive(Clone, Copy)]
enum Intrinsic {
    VStack,
    HStack,
    Button,
    Divider,
    Caption,
    TextInput,
    Checkbox,
    Toggle,
    Container,
    ThemeProvider,
    Texteditor,
    Tooltip,
    Svgview,
    Imgdemo,
    Clip,
    Tabpanel,
    Richtext,
}

fn intrinsic_for_tag(tag: &str) -> Option<Intrinsic> {
    match tag {
        "vstack" | "v-stack" | "column" => Some(Intrinsic::VStack),
        "hstack" | "h-stack" | "row" => Some(Intrinsic::HStack),
        "button" => Some(Intrinsic::Button),
        "divider" | "separator" => Some(Intrinsic::Divider),
        "caption" | "subtitle" => Some(Intrinsic::Caption),
        "textinput" | "text-input" => Some(Intrinsic::TextInput),
        "checkbox" => Some(Intrinsic::Checkbox),
        "toggle" | "switch" => Some(Intrinsic::Toggle),
        "container" | "box" => Some(Intrinsic::Container),
        "themeprovider" => Some(Intrinsic::ThemeProvider),
        "texteditor" | "codeeditor" => Some(Intrinsic::Texteditor),
        "tooltip" => Some(Intrinsic::Tooltip),
        "svgview" | "svg" => Some(Intrinsic::Svgview),
        "imgdemo" => Some(Intrinsic::Imgdemo),
        "clip" => Some(Intrinsic::Clip),
        "tabpanel" => Some(Intrinsic::Tabpanel),
        "richtext" => Some(Intrinsic::Richtext),
        _ => None,
    }
}

fn props_fn(props: &ObjectMap, keys: &[&str]) -> Option<Rc<dyn Fn(&[Value]) -> Value>> {
    for k in keys {
        if let Some(Value::Function(f)) = props.get(*k) {
            return Some(Rc::clone(f));
        }
    }
    None
}

fn collect_visible_text(children: &[Value]) -> String {
    children
        .iter()
        .map(|c| match c {
            Value::String(s) => s.as_ref().to_string(),
            Value::Array(a) => collect_visible_text(&a.borrow()),
            Value::Object(o) => {
                let b = o.borrow();
                let is_text = matches!(
                    b.get("tag"),
                    Some(Value::String(t)) if t.as_ref() == "text"
                );
                let nested = vnode_children(&b);
                drop(b);
                if is_text {
                    collect_visible_text(&nested)
                } else {
                    String::new()
                }
            }
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .concat()
}

fn button_caption(children: &[Value], props: &ObjectMap) -> String {
    let t = collect_visible_text(children);
    if !t.is_empty() {
        return t;
    }
    props_string(props, &["label", "title", "text"]).unwrap_or_else(|| "Button".to_string())
}

/// Styled `<text>` for cases that need a `variant` without a block tag.
/// Prefer `<p>` / `<span style="...">` (see `html_css::html_element_view`); strings map to labels in `value_into_any_view`.
/// Use `<h1>`…`<h6>` for headings. Optional `variant`: caption|small|subtitle (smaller gray); otherwise body with `style` for color/size.
fn text_view(children: &[Value], props: &ObjectMap) -> floem::AnyView {
    let text = collect_visible_text(children);
    let variant = props_string(props, &["variant", "size"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    let style_props = props.clone();
    label(move || text.clone())
        .style(move |s| {
            let mut s = s;
            match variant.as_str() {
                "caption" | "small" | "subtitle" => {
                    s = s.font_size(12.0).color(css::GRAY)
                }
                _ => {
                    s = s.font_size(14.0).line_height(1.35);
                }
            }
            html_css::merge_style_from_props(s, &style_props)
        })
        .into_any()
}

fn heading_level_from_tag(tag: &str) -> Option<u8> {
    match tag {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        "title" => Some(1),
        "heading" => Some(2),
        _ => None,
    }
}

/// Semantic headings: `<h1>`…`<h6>`, plus legacy `<title>` (→ h1) and `<heading>` (→ h2).
/// Optional `id` registers an element id for `document.getElementById(..).scrollIntoView()` (DOM-compatible).
fn html_heading_view(level: u8, props: &ObjectMap, children: Vec<Value>) -> floem::AnyView {
    let text = collect_visible_text(&children);
    let style_props = props.clone();
    let anchor_key = props_string(props, &["id"]);
    let level = level.clamp(1, 6);
    let size: f32 = match level {
        1 => 22.0,
        2 => 18.0,
        3 => 16.0,
        4 => 15.0,
        5 => 14.0,
        _ => 13.0,
    };
    let lbl = label(move || text.clone()).style(move |s| {
        let p = theme_palette_for_vnode(&style_props);
        let s = s
            .font_size(size)
            .font_bold()
            .line_height(1.25)
            .width_full()
            .color(p.fg);
        html_css::merge_style_from_props(s, &style_props)
    });
    if let Some(key) = anchor_key {
        let c = container(lbl).style(|s| s.width_full());
        register_scroll_anchor(key, c.id());
        c.into_any()
    } else {
        lbl.into_any()
    }
}

fn caption_view(children: &[Value]) -> floem::AnyView {
    let text = collect_visible_text(children);
    label(move || text.clone())
        .style(|s| s.font_size(12.0).color(css::GRAY))
        .into_any()
}

fn input_type_normalized(props: &ObjectMap) -> String {
    props_string(props, &["type"])
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn html_input_text_view(props: &ObjectMap) -> AnyView {
    let initial = props_string(props, &["value", "defaultValue", "default"]).unwrap_or_default();
    let placeholder = props_string(props, &["placeholder", "hint"]).unwrap_or_default();
    let appearance = resolve_appearance(props);
    let buf = create_rw_signal(initial);
    let input_style_props = props.clone();
    let mut input = text_input(buf).style(move |s| {
        let dark = effective_dark_from_appearance(appearance.as_str());
        let p = palette_for_dark(dark);
        let s = s
            .width_full()
            .max_width(400.0)
            .padding_horiz(12.0)
            .padding_vert(8.0)
            .border(1.0)
            .border_color(p.border)
            .border_radius(6.0)
            .background(p.bg)
            .color(p.fg);
        html_css::merge_style_from_props(s, &input_style_props)
    });
    if !placeholder.is_empty() {
        input = input.placeholder(placeholder);
    }
    input.into_any()
}

fn html_input_checkbox_view(props: &ObjectMap) -> AnyView {
    let start = props_bool(props, &["checked", "defaultChecked"]);
    let on = create_rw_signal(start);
    Checkbox::labeled_rw(on, move || String::new()).into_any()
}

fn html_input_range_view(props: &ObjectMap) -> AnyView {
    let min = props_f64(props, &["min"], 0.0);
    let max = props_f64(props, &["max"], 100.0);
    let val = props_f64(props, &["value", "defaultValue"], min);
    let pct_f = if (max - min).abs() > f64::EPSILON {
        ((val - min) / (max - min) * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    let pct = create_rw_signal(pct_f.pct());
    Slider::new_rw(pct)
        .slider_style(|s| {
            s.bar_color(Color::from_rgb8(220, 225, 235))
                .accent_bar_color(Color::from_rgb8(59, 130, 246))
                .bar_radius(4.pct())
                .accent_bar_radius(4.pct())
        })
        .style(|s| {
            s.flex_grow(1.0)
                .flex_basis(0.0)
                .min_width(120.0)
                .max_width(420.0)
                .margin_vert(6.0)
        })
        .into_any()
}

fn html_input_radio_view(props: &ObjectMap) -> AnyView {
    let name = props_string(props, &["name"]).unwrap_or_default();
    let value = props_string(props, &["value"])
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0);
    let checked = props_bool(props, &["checked", "defaultChecked"]);

    let sel = if name.is_empty() {
        create_rw_signal(value)
    } else {
        RADIO_GROUP_SEL.with(|cell| {
            let mut m = cell.borrow_mut();
            match m.entry(name) {
                Entry::Occupied(e) => {
                    let s = *e.get();
                    if checked {
                        s.set(value);
                    }
                    s
                }
                Entry::Vacant(v) => *v.insert(create_rw_signal(value)),
            }
        })
    };

    RadioButton::new_labeled_rw(value, sel, move || String::new()).into_any()
}

fn html_input_view(props: &ObjectMap, _children: Vec<Value>) -> AnyView {
    match input_type_normalized(props).as_str() {
        "radio" => html_input_radio_view(props),
        "range" => html_input_range_view(props),
        "checkbox" => html_input_checkbox_view(props),
        _ => html_input_text_view(props),
    }
}

fn dropdown_options(children: &[Value]) -> Vec<i32> {
    let mut out = Vec::new();
    for c in children {
        let Value::Object(o) = c else {
            continue;
        };
        let m = o.borrow();
        let tag = match m.get("tag") {
            Some(Value::String(t)) => t.as_ref().to_string(),
            _ => {
                drop(m);
                continue;
            }
        };
        if tag != "option" {
            drop(m);
            continue;
        }
        let props = vnode_props(&m);
        drop(m);
        if let Some(v) = props_string(&props, &["value"])
            .and_then(|s| s.parse().ok())
        {
            out.push(v);
        }
    }
    out
}

fn theme_provider_view(props: &ObjectMap, children: Vec<Value>) -> AnyView {
    let eff = theme_provider_resolved_value(props);
    let pass = Some(eff.clone());
    let inner: AnyView = match children.len() {
        0 => empty().into_any(),
        1 => value_into_any_view_impl(children.into_iter().next().unwrap(), pass),
        _ => v_stack_from_iter(
            children
                .into_iter()
                .map(|c| value_into_any_view_impl(c, pass.clone()).into_view()),
        )
        .style(|s| s.size_full())
        .into_any(),
    };

    // Layout-transparent wrapper: fills the parent and establishes a flex-column context so
    // children can use either `flex: 1` (to grow along the column axis) or `width/height: 100%`
    // (which resolves against this container's definite size).
    container(inner)
        .style(|s| s.size_full().flex_col())
        .into_any()
}

fn richtext_demo_view() -> AnyView {
    let text0 = "Rich text with a highlighted span.".to_string();
    let initial = AttrsList::new(
        Attrs::new()
            .color(Color::from_rgb8(0x33, 0x36, 0x3f))
            .font_size(15.0),
    );
    let rich = rich_text(text0.clone(), initial, move || {
        let dark = OS_THEME_IS_DARK.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|s| s.get())
                .unwrap_or(false)
        });
        let base = if dark {
            Color::from_rgb8(0xc0, 0xc6, 0xd4)
        } else {
            Color::from_rgb8(0x33, 0x36, 0x3f)
        };
        let hi = if dark {
            Color::from_rgb8(0x7d, 0xae, 0xff)
        } else {
            Color::from_rgb8(0x1d, 0x4e, 0xd8)
        };
        let text = "Rich text with a highlighted span.";
        let mut attrs = AttrsList::new(Attrs::new().color(base).font_size(15.0));
        if let Some(start) = text.find("highlighted") {
            let end = start + "highlighted".len();
            attrs.add_span(
                start..end,
                Attrs::new()
                    .color(hi)
                    .weight(FontWeight::BOLD)
                    .font_size(15.0),
            );
        }
        (text.to_string(), attrs)
    });
    rich.style(|s| s.margin_top(6.0)).into_any()
}

fn texteditor_view(props: &ObjectMap) -> AnyView {
    let initial = props_string(props, &["value", "default", "defaultValue"])
        .unwrap_or_else(|| "// Tish + Floem\nfn hello() {\n    \"world\"\n}\n".to_string());
    let appearance = resolve_appearance(props);
    let appearance_editor = appearance.clone();
    let appearance_style = appearance;
    text_editor(Rope::from(initial))
        .editor_style(move |s| {
            let dark = effective_dark_from_appearance(appearance_editor.as_str());
            if dark {
                default_dark_color(s)
            } else {
                default_light_theme(s)
            }
        })
        .style(move |s| {
            let dark = effective_dark_from_appearance(appearance_style.as_str());
            let p = palette_for_dark(dark);
            s.width_full()
                .max_width(640.0)
                .min_height(200.0)
                .border(1.0)
                .border_color(p.border)
                .border_radius(8.0)
        })
        .into_any()
}

fn tooltip_view(props: &ObjectMap, children: Vec<Value>) -> AnyView {
    let tip = props_string(props, &["tip", "title", "label"]).unwrap_or_else(|| "Tooltip".to_string());
    let ch_inh = Some(resolve_appearance(props));
    let child = if children.len() == 1 {
        value_into_any_view_impl(children.into_iter().next().unwrap(), ch_inh.clone())
    } else {
        v_stack_dyn_children_inherit(children, ch_inh).into_any()
    };
    tooltip(
        child,
        move || static_label(tip.clone()).style(|s| s.padding(8.0).font_size(13.0)),
    )
    .into_any()
}

fn svg_view(props: &ObjectMap) -> AnyView {
    let data = props_string(props, &["data", "src"]).unwrap_or_else(|| {
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/></svg>"#
            .to_string()
    });
    svg(data).into_any()
}

fn tiny_png_bytes() -> Vec<u8> {
    vec![
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ]
}

fn imgdemo_view() -> AnyView {
    img(|| tiny_png_bytes())
        .style(|s| s.width(32.pt()).height(32.pt()))
        .into_any()
}

fn clip_view(props: &ObjectMap, children: Vec<Value>) -> AnyView {
    let ch_inh = Some(resolve_appearance(props));
    let inner = if children.len() == 1 {
        value_into_any_view_impl(children.into_iter().next().unwrap(), ch_inh.clone())
    } else {
        v_stack_dyn_children_inherit(children, ch_inh).into_any()
    };
    clip(container(inner).style(|s| s.padding(8.0)))
        .style(|s| s.width_full().max_width(320.0).height(48.0).border(1.0))
        .into_any()
}

fn tabpanel_view(props: &ObjectMap, children: Vec<Value>) -> AnyView {
    let active = props_f64(props, &["active"], f64::NAN) as i32;
    let id = props_f64(props, &["id", "panel"], -999.0) as i32;
    // Inactive panel: do not build children (editors, lists, etc. stay unallocated until this panel matches `active`).
    if active != id {
        return container(empty()).style(|s| s.hide()).into_any();
    }
    let ch_inh = Some(resolve_appearance(props));
    let body: AnyView = if children.len() == 1 {
        value_into_any_view_impl(children.into_iter().next().unwrap(), ch_inh.clone())
    } else {
        v_stack_dyn_children_inherit(children, ch_inh).into_any()
    };
    container(body)
        .style(|s| s.width_full().padding(8.0))
        .into_any()
}

/// `<select>` + `<option value="…">` → Floem dropdown (HTML pattern).
fn select_view(props: &ObjectMap, children: Vec<Value>) -> AnyView {
    let opts = dropdown_options(&children);
    let items: Vec<i32> = if opts.is_empty() {
        vec![1, 2, 3]
    } else {
        opts
    };
    let initial = props_f64(props, &["value", "defaultValue"], items[0] as f64) as i32;
    let sel = create_rw_signal(initial);
    let appearance = resolve_appearance(props);
    Dropdown::new_rw(sel, items.clone())
        .style(move |s| {
            let dark = effective_dark_from_appearance(appearance.as_str());
            let p = palette_for_dark(dark);
            s.width_full().max_width(240.0).color(p.fg)
        })
        .into_any()
}

fn apply_stack_style(s: floem::style::Style, props: &ObjectMap) -> floem::style::Style {
    let mut s = s;
    let w = props_f64(props, &["width", "widthPx"], 0.0);
    if w > 0.0 {
        s = s.width(w);
    }
    let mw = props_f64(props, &["maxWidth", "max_width"], 0.0);
    if mw > 0.0 {
        s = s.max_width(mw);
    }
    let fg = props_f64(props, &["flexGrow", "flex_grow"], 0.0);
    if fg > 0.0 {
        s = s
            .flex_grow(fg as f32)
            .flex_basis(0.0)
            .min_width(0.0)
            .min_height(0.0);
    }
    let mh = props_f64(props, &["minHeight", "min_height"], 0.0);
    if mh > 0.0 {
        s = s.min_height(mh);
    }
    if props_bool(props, &["fillHeight", "fill_height"]) {
        s = s.height_full().min_height(0.0);
    }
    s
}

pub(crate) fn value_into_any_view(v: Value) -> floem::AnyView {
    value_into_any_view_impl(v, None)
}

pub(crate) fn value_into_any_view_impl(
    v: Value,
    inherited_appearance: Option<String>,
) -> floem::AnyView {
    match v {
        Value::String(s) => {
            let t = s.as_ref().trim().to_string();
            if t.is_empty() {
                label(|| "").into_any()
            } else {
                let inh = inherited_appearance.clone();
                label(move || t.clone())
                    .style(move |s| {
                        let mut m = ObjectMap::default();
                        if let Some(ref a) = inh {
                            m.insert(Arc::from("appearance"), Value::String(a.clone().into()));
                        }
                        let p = theme_palette_for_vnode(&m);
                        s.font_size(14.0).color(p.fg)
                    })
                    .into_any()
            }
        }
        Value::Object(rc) => {
            let map = rc.borrow();
            if is_fragment_object(&map) {
                let ch = vnode_children(&map);
                drop(map);
                return v_stack_dyn_children_inherit(ch, inherited_appearance).into_any();
            }
            let tag = map.get("tag").cloned();
            let props_raw = vnode_props(&map);
            let children = vnode_children(&map);
            drop(map);
            let props = merge_inherited_appearance(props_raw, inherited_appearance.as_deref());

            match tag {
                Some(Value::String(t)) if t.as_ref() == "text" => text_view(&children, &props),
                Some(Value::String(t)) => {
                    let name_ref = t.as_ref();
                    if matches!(
                        name_ref,
                        "div" | "span" | "p" | "section" | "article" | "aside" | "nav" | "ul" | "ol"
                            | "li" | "label" | "fieldset"
                    ) {
                        return html_css::html_element_view(name_ref, &props, children);
                    }
                    if name_ref == "select" {
                        return select_view(&props, children);
                    }
                    if name_ref == "input" {
                        return html_input_view(&props, children);
                    }
                    if let Some(lvl) = heading_level_from_tag(name_ref) {
                        return html_heading_view(lvl, &props, children);
                    }
                    let name = name_ref.to_string();
                    if let Some(kind) = intrinsic_for_tag(name.as_str()) {
                        return intrinsic_view(kind, &props, children);
                    }
                    let pass = Some(resolve_appearance(&props));
                    v_stack((
                        label(move || format!("{}", name))
                            .style(|s| s.font_size(11.0).color(css::GRAY)),
                        v_stack_dyn_children_inherit(children, pass),
                    ))
                    .into_any()
                }
                _ => label(|| "(?)").into_any(),
            }
        }
        Value::Null => label(|| "").into_any(),
        Value::Array(a) => {
            let items = a.borrow().clone();
            match items.len() {
                0 => empty().into_any(),
                1 => value_into_any_view_impl(items.into_iter().next().unwrap(), inherited_appearance),
                _ => v_stack_dyn_children_inherit(items, inherited_appearance).into_any(),
            }
        }
        _ => label(|| "").into_any(),
    }
}

const SCROLL_ROOT_PROP_KEYS: &[&str] = &[
    "data-scroll-root",
    "dataScrollRoot",
    "scrollRoot",
    "scroll-root",
];

fn scroll_root_from_props(props: &ObjectMap) -> bool {
    if props_bool(props, SCROLL_ROOT_PROP_KEYS) {
        return true;
    }
    for k in SCROLL_ROOT_PROP_KEYS {
        if let Some(Value::String(s)) = props.get(*k) {
            if s.trim().eq_ignore_ascii_case("true") {
                return true;
            }
        }
    }
    false
}

/// Floem scroll viewport (wheel, flex fill, `window.scrollTo` target). Inner node is already wrapped like CSS `overflow` content.
pub(crate) fn scroll_host_viewport(props: &ObjectMap, wrapped_inner: AnyView) -> AnyView {
    let fill = html_css::scroll_fill_from_style(props);
    let scroll_root = scroll_root_from_props(props);
    let scroll_style_outer = props.clone();
    let mut sc = Scroll::new(wrapped_inner);
    if scroll_root {
        if let Some(sig) = SCROLL_ROOT_PIXEL.with(|c| c.borrow().clone()) {
            sc = sc.scroll_to(move || sig.get());
        }
    }
    // Floem layout examples (e.g. `examples/layout/left_sidebar`) use `flex_col` + `flex_grow` on
    // `Scroll` only — not `ScrollCustomStyle::shrink_to_fit`. Combining `shrink_to_fit` with
    // `align_items: stretch` was collapsing the scroll content to zero visible height on main.
    sc = sc.custom_style(move |s| {
        let mut st = s;
        if fill {
            st = st.propagate_pointer_wheel(false);
        }
        st
    });
    sc.style(move |s| {
        // Column flex: cross axis is horizontal. `items_start` keeps children at content width;
        // `Stretch` makes the scroll subtree use the full viewport width (fill mode only — see
        // custom_style note above re shrink_to_fit + stretch height).
        let mut s = s.width_full().flex_col();
        s = if fill {
            s.align_items(AlignItems::Stretch)
        } else {
            s.items_start()
        };
        if fill {
            s = s
                .flex_basis(0.0)
                .min_width(0.0)
                .min_height(0.0)
                .flex_grow(1.0)
                .flex_shrink(1.0)
                .border(0.0);
            s = html_css::merge_scroll_host_style_from_props(s, &scroll_style_outer);
            // Authored `height: 100%` is applied above as a real percent, but in this flex chain
            // the percentage base is often indefinite, so Taffy can resolve it to 0 and collapse
            // the scroll viewport (blank main column). Vertical fill is already expressed by
            // `flex: 1` + `flex-basis: 0` + `min-height: 0` on the host.
            s.height(PxPctAuto::Auto)
        } else {
            s = s
                .height(220.0)
                .border(1.0)
                .border_color(Color::from_rgb8(210, 210, 220));
            html_css::merge_style_from_props(s, &scroll_style_outer)
        }
    })
    .into_any()
}

fn stack_style_v() -> impl Fn(floem::style::Style) -> floem::style::Style + Copy {
    |s| s.width_full().items_start().justify_start()
}

fn stack_style_h() -> impl Fn(floem::style::Style) -> floem::style::Style + Copy {
    |s| {
        s.width_full()
            // Stretch so a flex sibling with `flex: 1; min-height: 0` gets a real viewport height.
            .align_items(AlignItems::Stretch)
            .justify_start()
    }
}

fn intrinsic_view(kind: Intrinsic, props: &ObjectMap, children: Vec<Value>) -> floem::AnyView {
    let stack_props = props.clone();
    let ch_inh = Some(resolve_appearance(props));
    match kind {
        Intrinsic::VStack => {
            let stack = v_stack_from_iter(
                children
                    .into_iter()
                    .map(|c| value_into_any_view_impl(c, ch_inh.clone()).into_view()),
            );
            stack
                .style(move |s| {
                    html_css::merge_style_from_props(
                        apply_stack_style(stack_style_v()(s), &stack_props),
                        &stack_props,
                    )
                })
                .into_any()
        }
        Intrinsic::HStack => {
            let stack = h_stack_from_iter(
                children
                    .into_iter()
                    .map(|c| value_into_any_view_impl(c, ch_inh.clone()).into_view()),
            );
            stack
                .style(move |s| {
                    html_css::merge_style_from_props(
                        apply_stack_style(stack_style_h()(s), &stack_props),
                        &stack_props,
                    )
                })
                .into_any()
        }
        Intrinsic::Button => {
            let cap = button_caption(&children, props);
            let handler = props_fn(props, &["onClick", "onclick", "onTap", "ontap"]);
            let label_style_props = props.clone();
            let button_style_props = props.clone();
            // Vnode surface is HTML-like: `style` / `class` (when wired) drive visuals for any backend.
            // Floem applies the same declarations a DOM host would map to CSS.
            let b = button(
                label(move || cap.clone()).style(move |s| {
                    let p = theme_palette_for_vnode(&label_style_props);
                    html_css::merge_style_from_props(s.font_size(14.0).font_bold().color(p.fg), &label_style_props)
                }),
            )
            .style(move |s| {
                html_css::merge_style_from_props(s, &button_style_props)
            });
            match handler {
                Some(f) => b
                    .action(move || {
                        let _ = f(&[]);
                    })
                    .into_any(),
                None => b.into_any(),
            }
        }
        Intrinsic::Divider => {
            let appearance = resolve_appearance(props);
            let divider_style_props = props.clone();
            empty()
                .style(move |s| {
                    let dark = effective_dark_from_appearance(appearance.as_str());
                    let c = if dark {
                        Color::from_rgb8(0x45, 0x4d, 0x5c)
                    } else {
                        Color::from_rgb8(200, 200, 210)
                    };
                    let s = s.height(1.0).width_full().background(c);
                    html_css::merge_style_from_props(s, &divider_style_props)
                })
                .into_any()
        }
        Intrinsic::Caption => caption_view(&children),
        Intrinsic::TextInput => {
            let initial = props_string(props, &["value", "defaultValue", "default"]).unwrap_or_default();
            let placeholder = props_string(props, &["placeholder", "hint"]).unwrap_or_default();
            let appearance = resolve_appearance(props);
            let buf = create_rw_signal(initial);
            let input_style_props = props.clone();
            let mut input = text_input(buf).style(move |s| {
                let dark = effective_dark_from_appearance(appearance.as_str());
                let p = palette_for_dark(dark);
                let s = s
                    .width_full()
                    .max_width(400.0)
                    .padding_horiz(12.0)
                    .padding_vert(8.0)
                    .border(1.0)
                    .border_color(p.border)
                    .border_radius(6.0)
                    .background(p.bg)
                    .color(p.fg);
                html_css::merge_style_from_props(s, &input_style_props)
            });
            if !placeholder.is_empty() {
                input = input.placeholder(placeholder);
            }
            input.into_any()
        }
        Intrinsic::Checkbox => {
            let checked = create_rw_signal(false);
            let label_txt = collect_visible_text(&children);
            let lbl = if label_txt.is_empty() {
                "Checkbox".to_string()
            } else {
                label_txt
            };
            Checkbox::labeled_rw(checked, move || lbl.clone()).into_any()
        }
        Intrinsic::Toggle => {
            let on = create_rw_signal(false);
            toggle_button(move || on.get())
                .on_event_stop(ToggleChanged::listener(), move |_cx, v| on.set(*v))
                .into_any()
        }
        Intrinsic::Container => {
            let body: floem::AnyView = if children.len() == 1 {
                value_into_any_view_impl(children.into_iter().next().unwrap(), ch_inh.clone())
            } else {
                v_stack_dyn_children_inherit(children, ch_inh.clone()).into_any()
            };
            let container_style_props = props.clone();
            container(body)
                .style(move |s| {
                    html_css::merge_style_from_props(s.width_full(), &container_style_props)
                })
                .into_any()
        }
        Intrinsic::ThemeProvider => theme_provider_view(props, children),
        Intrinsic::Texteditor => texteditor_view(props),
        Intrinsic::Tooltip => tooltip_view(props, children),
        Intrinsic::Svgview => svg_view(props),
        Intrinsic::Imgdemo => imgdemo_view(),
        Intrinsic::Clip => clip_view(props, children),
        Intrinsic::Tabpanel => tabpanel_view(props, children),
        Intrinsic::Richtext => richtext_demo_view(),
    }
}

fn v_stack_dyn_children_inherit(children: Vec<Value>, inherited: Option<String>) -> impl IntoView {
    v_stack_from_iter(children.into_iter().map(|child| {
        value_into_any_view_impl(child, inherited.clone()).into_view()
    }))
    .style(|s| s.width_full().items_start().justify_start())
}

/// Run the user `update` callback once (so `createRoot` / hooks run), then open a Floem window.
///
/// **Floem inspector:** press **`~`** while this window is focused.
pub fn floem_run(update: Rc<dyn Fn(&[Value]) -> Value>) {
    let root = RwSignal::new(Value::Null);
    install_thread_local_host(Box::new(FloemHost::new(root)));
    update(&[]);
    floem::launch(move || {
        let os_dark = create_rw_signal(None::<bool>);
        OS_THEME_IS_DARK.with(|c| {
            *c.borrow_mut() = Some(os_dark);
        });

        let root_pixel = create_rw_signal(None::<Point>);
        SCROLL_ROOT_PIXEL.with(|c| *c.borrow_mut() = Some(root_pixel));

        dyn_container(
            move || root.get(),
            move |v| value_into_any_view(v),
        )
        .style(|s| {
            s.size_full()
                .font_family("Helvetica Neue".to_owned())
                .keyboard_navigable()
        })
        .on_event(listener::ThemeChanged, move |_cx, t| {
            os_dark.set(Some(*t == Theme::Dark));
            EventPropagation::Continue
        })
        .on_event(listener::KeyUp, |_cx, ev| {
            let is_tilde = match &ev.key {
                Key::Character(s) => s == "~",
                _ => false,
            };
            if is_tilde {
                action::inspect();
                return EventPropagation::Stop;
            }
            EventPropagation::Continue
        })
    });
}

fn value_as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        _ => None,
    }
}

fn set_root_scroll_viewport_pixel(origin: Point) {
    SCROLL_ROOT_PIXEL.with(|c| {
        if let Some(sig) = c.borrow().as_ref() {
            sig.set(Some(origin));
        }
    });
}

/// [`Window.scrollTo`](https://drafts.csswg.org/cssom-view/#dom-window-scrollto): `(x, y)` or `({ top, left })`.
fn window_scroll_to_impl(args: &[Value]) -> Value {
    match args.len() {
        2 => {
            let x = value_as_f64(&args[0]).unwrap_or(0.0);
            let y = value_as_f64(&args[1]).unwrap_or(0.0);
            set_root_scroll_viewport_pixel(Point::new(x, y));
        }
        1 => match &args[0] {
            Value::Object(o) => {
                let m = o.borrow();
                let left = m.get("left").and_then(value_as_f64).unwrap_or(0.0);
                let top = m.get("top").and_then(value_as_f64).unwrap_or(0.0);
                set_root_scroll_viewport_pixel(Point::new(left, top));
            }
            Value::Number(n) => set_root_scroll_viewport_pixel(Point::new(0.0, *n)),
            _ => {}
        },
        _ => {}
    }
    Value::Null
}

fn element_scroll_into_view_by_id(element_id: &str) {
    if let Some(vid) = SCROLL_ANCHOR_VIEWS.with(|m| m.borrow().get(element_id).copied()) {
        vid.scroll_to(None);
    }
}

/// [`Location.assign`](https://html.spec.whatwg.org/#dom-location-assign) with a URL containing `#fragment`
/// scrolls the element with that id into view (same effect as changing `location.hash` in a browser).
fn location_assign_impl(args: &[Value]) -> Value {
    let Some(Value::String(s)) = args.first() else {
        return Value::Null;
    };
    let url = s.as_ref();
    if let Some((_, frag)) = url.split_once('#') {
        if !frag.is_empty() {
            element_scroll_into_view_by_id(frag.trim());
        }
    }
    Value::Null
}

fn document_get_element_by_id_impl(args: &[Value]) -> Value {
    let id = match args.first() {
        Some(Value::String(s)) => s.as_ref().to_string(),
        _ => return Value::Null,
    };
    let mut m = ObjectMap::default();
    let id_for_fn = id;
    m.insert(
        Arc::from("scrollIntoView"),
        Value::Function(Rc::new(move |_a: &[Value]| {
            element_scroll_into_view_by_id(id_for_fn.as_str());
            Value::Null
        })),
    );
    Value::Object(Rc::new(RefCell::new(m)))
}

fn floem_location_object() -> Value {
    tishlang_core::tish_module! {
        "assign" => |args: &[Value]| location_assign_impl(args),
    }
}

fn floem_window_object() -> Value {
    tishlang_core::tish_module! {
        "scrollTo" => |args: &[Value]| window_scroll_to_impl(args),
        "location" => |_args: &[Value]| floem_location_object(),
    }
}

fn floem_document_object() -> Value {
    tishlang_core::tish_module! {
        "getElementById" => |args: &[Value]| document_get_element_by_id_impl(args),
    }
}

/// JSX `<ThemeProvider value="dark">` → vnode; `h(ThemeProvider, props, children)` uses this factory.
fn theme_provider_component(args: &[Value]) -> Value {
    let Some(Value::Object(rc)) = args.first() else {
        return Value::Null;
    };
    let merged = rc.borrow().clone();
    let children = merged
        .get("children")
        .and_then(|v| match v {
            Value::Array(a) => Some(a.borrow().clone()),
            _ => None,
        })
        .unwrap_or_default();
    let mut props_only = merged;
    props_only.remove(&Arc::from("children"));
    let mut m = ObjectMap::default();
    m.insert(Arc::from("tag"), Value::String("themeprovider".into()));
    m.insert(
        Arc::from("props"),
        Value::Object(Rc::new(RefCell::new(props_only))),
    );
    m.insert(
        Arc::from("children"),
        Value::Array(Rc::new(RefCell::new(children))),
    );
    m.insert(Arc::from("_el"), Value::Null);
    Value::Object(Rc::new(RefCell::new(m)))
}

/// JSX component → vnode with the given intrinsic `tag` (lowercase, matches [`intrinsic_for_tag`]).
fn make_vnode_factory(tag: &'static str) -> Value {
    Value::Function(Rc::new(move |args: &[Value]| {
        let Some(Value::Object(rc)) = args.first() else {
            return Value::Null;
        };
        let merged = rc.borrow().clone();
        let children = merged
            .get("children")
            .and_then(|v| match v {
                Value::Array(a) => Some(a.borrow().clone()),
                _ => None,
            })
            .unwrap_or_default();
        let mut props_only = merged;
        props_only.remove(&Arc::from("children"));
        let mut m = ObjectMap::default();
        m.insert(Arc::from("tag"), Value::String(tag.into()));
        m.insert(
            Arc::from("props"),
            Value::Object(Rc::new(RefCell::new(props_only))),
        );
        m.insert(
            Arc::from("children"),
            Value::Array(Rc::new(RefCell::new(children))),
        );
        m.insert(Arc::from("_el"), Value::Null);
        Value::Object(Rc::new(RefCell::new(m)))
    }))
}

/// The `floem` named export: `{ run }`.
fn floem_api_object() -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("run"), Value::Function(Rc::new(|args: &[Value]| {
        if let Some(Value::Function(f)) = args.first() {
            floem_run(Rc::clone(f));
        }
        Value::Null
    })));
    Value::Object(Rc::new(RefCell::new(m)))
}

/// Namespace returned by `import { … } from 'tish:floem'`.
///
/// Each named import is a direct key on this object so the generic codegen
/// (`ns.get(export_name)`) works for every binding without special-casing:
///
/// ```tish
/// import { floem, document, window, ThemeProvider } from "tish:floem"
/// ```
pub fn floem_object() -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("floem"), floem_api_object());
    m.insert(Arc::from("window"), floem_window_object());
    m.insert(Arc::from("document"), floem_document_object());
    m.insert(Arc::from("ThemeProvider"), Value::Function(Rc::new(theme_provider_component)));
    // Platform UI primitives (`tish:floem`); HTML-like controls use native tags (`select`, `input`, `div` overflow).
    m.insert(Arc::from("Caption"), make_vnode_factory("caption"));
    m.insert(Arc::from("RichText"), make_vnode_factory("richtext"));
    m.insert(Arc::from("Toggle"), make_vnode_factory("toggle"));
    m.insert(Arc::from("TextInput"), make_vnode_factory("textinput"));
    m.insert(Arc::from("TextEditor"), make_vnode_factory("texteditor"));
    m.insert(Arc::from("Checkbox"), make_vnode_factory("checkbox"));
    m.insert(Arc::from("TabPanel"), make_vnode_factory("tabpanel"));
    m.insert(Arc::from("Clip"), make_vnode_factory("clip"));
    m.insert(Arc::from("Tooltip"), make_vnode_factory("tooltip"));
    m.insert(Arc::from("SvgView"), make_vnode_factory("svgview"));
    m.insert(Arc::from("ImgDemo"), make_vnode_factory("imgdemo"));
    Value::Object(Rc::new(RefCell::new(m)))
}

