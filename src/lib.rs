//! Floem window entry for Tish: installs a [`tishlang_ui::Host`] that maps committed vnodes to Floem views.
//!
//! UI is **defined in Tish/JSX**; this crate only maps intrinsics to Floem widgets.

use std::cell::RefCell;
use std::rc::Rc;

use floem::action;
use floem::event::{Event, EventListener, EventPropagation};
use floem::keyboard::Key;
use floem::peniko::Color;
use floem::prelude::*;
use floem::text::{Attrs, AttrsList, TextLayout, Weight};
use floem::views::dropdown::Dropdown;
use floem::views::editor::text::{default_dark_color, default_light_theme};
use floem::views::slider::Slider;
use floem::views::RadioButton;
use floem::AnyView;
use floem_winit::window::Theme;
use lapce_xi_rope::Rope;
use tishlang_core::{ObjectMap, Value};
use tishlang_ui::{install_thread_local_host, Host, FRAGMENT_SENTINEL};

mod html_css;

// Latest OS dark-mode hint from the window (`None` until the first `ThemeChanged`).
thread_local! {
    static OS_THEME_IS_DARK: RefCell<Option<RwSignal<Option<bool>>>> = const { RefCell::new(None) };
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

fn props_string(props: &ObjectMap, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(Value::String(s)) = props.get(*k) {
            return Some(s.as_ref().to_string());
        }
    }
    None
}

fn props_f64(props: &ObjectMap, keys: &[&str], default: f64) -> f64 {
    for k in keys {
        if let Some(Value::Number(n)) = props.get(*k) {
            return *n;
        }
    }
    default
}

fn props_bool(props: &ObjectMap, keys: &[&str]) -> bool {
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
struct ThemePalette {
    bg: Color,
    fg: Color,
    sidebar: Color,
    border: Color,
    accent: Color,
    panel: Color,
    ghost_hover: Color,
}

fn palette_for_dark(dark: bool) -> ThemePalette {
    if dark {
        ThemePalette {
            bg: Color::rgb8(0x1e, 0x22, 0x2a),
            fg: Color::rgb8(0xe6, 0xe8, 0xef),
            sidebar: Color::rgb8(0x17, 0x1a, 0x21),
            border: Color::rgb8(0x3a, 0x40, 0x4d),
            accent: Color::rgb8(0x61, 0x8c, 0xff),
            panel: Color::rgb8(0x25, 0x2a, 0x34),
            ghost_hover: Color::rgb8(0x2c, 0x33, 0x3f),
        }
    } else {
        ThemePalette {
            bg: Color::rgb8(0xf6, 0xf7, 0xfb),
            fg: Color::rgb8(0x22, 0x24, 0x2d),
            sidebar: Color::rgb8(0xef, 0xf0, 0xf6),
            border: Color::rgb8(0xd4, 0xd7, 0xe3),
            accent: Color::rgb8(0x3b, 0x82, 0xf6),
            panel: Color::WHITE,
            ghost_hover: Color::rgb8(0xe8, 0xea, 0xf2),
        }
    }
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
    Scroll,
    Spacer,
    Divider,
    Panel,
    Caption,
    TextInput,
    Checkbox,
    Slider,
    Toggle,
    Radiogroup,
    Container,
    Themebox,
    Dropdown,
    Texteditor,
    Tooltip,
    Svgview,
    Imgdemo,
    Clip,
    Tabpanel,
    List,
    Richtext,
}

fn intrinsic_for_tag(tag: &str) -> Option<Intrinsic> {
    match tag {
        "vstack" | "v-stack" | "column" => Some(Intrinsic::VStack),
        "hstack" | "h-stack" | "row" => Some(Intrinsic::HStack),
        "button" => Some(Intrinsic::Button),
        "scroll" => Some(Intrinsic::Scroll),
        "spacer" => Some(Intrinsic::Spacer),
        "divider" | "separator" => Some(Intrinsic::Divider),
        "panel" | "card" | "section" => Some(Intrinsic::Panel),
        "caption" | "subtitle" => Some(Intrinsic::Caption),
        "textinput" | "text-input" | "input" => Some(Intrinsic::TextInput),
        "checkbox" => Some(Intrinsic::Checkbox),
        "slider" => Some(Intrinsic::Slider),
        "toggle" | "switch" => Some(Intrinsic::Toggle),
        "radiogroup" | "radio-group" => Some(Intrinsic::Radiogroup),
        "container" | "box" => Some(Intrinsic::Container),
        "themebox" => Some(Intrinsic::Themebox),
        "dropdown" => Some(Intrinsic::Dropdown),
        "texteditor" | "codeeditor" => Some(Intrinsic::Texteditor),
        "tooltip" => Some(Intrinsic::Tooltip),
        "svgview" | "svg" => Some(Intrinsic::Svgview),
        "imgdemo" => Some(Intrinsic::Imgdemo),
        "clip" => Some(Intrinsic::Clip),
        "tabpanel" => Some(Intrinsic::Tabpanel),
        "list" => Some(Intrinsic::List),
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

/// Styled `<text>` for cases that need props (`variant`, `muted`) without a block tag.
/// Prefer raw JSX text children or `<p>` / `<span>` (see `html_css::html_element_view`); strings map to labels in `value_into_any_view`.
/// Use `<h1>`…`<h6>` for headings. Props: `variant` (caption|body), optional `muted` (bool).
fn text_view(children: &[Value], props: &ObjectMap) -> floem::AnyView {
    let text = collect_visible_text(children);
    let variant = props_string(props, &["variant", "size"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    let muted = props_bool(props, &["muted", "dim"]);
    let style_props = props.clone();
    label(move || text.clone())
        .style(move |s| {
            let mut s = s;
            match variant.as_str() {
                "caption" | "small" | "subtitle" => {
                    s = s.font_size(12.0).color(Color::GRAY)
                }
                _ => {
                    s = s.font_size(14.0).line_height(1.35);
                    if muted {
                        s = s.color(Color::GRAY);
                    }
                }
            }
            html_css::merge_style_from_props(s, &style_props)
        })
        .into_any()
}

fn radiogroup_view(children: Vec<Value>) -> floem::AnyView {
    let mut items: Vec<(i32, String)> = Vec::new();
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
        if tag != "radio" && tag != "option" {
            drop(m);
            continue;
        }
        let props = vnode_props(&m);
        let ch = vnode_children(&m);
        drop(m);
        let v = props_string(&props, &["value"])
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| items.len() as i32);
        let lbl = collect_visible_text(&ch);
        items.push((
            v,
            if lbl.is_empty() {
                format!("Option {}", v)
            } else {
                lbl
            },
        ));
    }
    if items.is_empty() {
        return label(|| "(empty radiogroup)").into_any();
    }
    let initial = items[0].0;
    let sel = create_rw_signal(initial);
    v_stack_from_iter(items.into_iter().map(|(val, lbl)| {
        let cap = lbl.clone();
        RadioButton::new_labeled_rw(val, sel, move || cap.clone()).into_view()
    }))
    .style(stack_style_v())
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
fn html_heading_view(level: u8, props: &ObjectMap, children: Vec<Value>) -> floem::AnyView {
    let text = collect_visible_text(&children);
    let style_props = props.clone();
    let level = level.clamp(1, 6);
    let (size, margin_bottom) = match level {
        1 => (22.0_f32, 8.0_f32),
        2 => (18.0, 6.0),
        3 => (16.0, 4.0),
        4 => (15.0, 4.0),
        5 => (14.0, 2.0),
        _ => (13.0, 2.0),
    };
    label(move || text.clone())
        .style(move |s| {
            let s = s
                .font_size(size)
                .font_bold()
                .margin_bottom(margin_bottom)
                .line_height(1.25)
                .width_full();
            html_css::merge_style_from_props(s, &style_props)
        })
        .into_any()
}

fn caption_view(children: &[Value]) -> floem::AnyView {
    let text = collect_visible_text(children);
    label(move || text.clone())
        .style(|s| s.font_size(12.0).color(Color::GRAY).margin_bottom(8.0))
        .into_any()
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

fn themebox_view(props: &ObjectMap, children: Vec<Value>) -> AnyView {
    let zone = props_string(props, &["zone", "role"])
        .unwrap_or_else(|| "main".to_string())
        .to_ascii_lowercase();
    let appearance = props_string(props, &["appearance", "theme"]).unwrap_or_else(|| "system".to_string());
    let width_px = props_f64(props, &["width", "widthPx"], 0.0);

    let body: AnyView = if children.len() == 1 {
        value_into_any_view(children.into_iter().next().unwrap())
    } else {
        v_stack_dyn_children(children).into_any()
    };

    container(body).style(move |s| {
        let dark = effective_dark_from_appearance(appearance.as_str());
        let p = palette_for_dark(dark);
        let mut s = s.width_full();
        if width_px > 0.0 {
            s = s.width(width_px);
        }
        match zone.as_str() {
            "sidebar" => s
                .height_full()
                .min_height(0.0)
                .flex_shrink(0.0)
                .padding(12.0)
                .border_right(1.0)
                .border_color(p.border)
                .background(p.sidebar),
            "header" => s
                .padding_horiz(16.0)
                .padding_vert(12.0)
                .border_bottom(1.0)
                .border_color(p.border)
                .background(if dark {
                    Color::rgb8(0x14, 0x16, 0x1c)
                } else {
                    Color::rgb8(0xf0, 0xf1, 0xf6)
                }),
            "card" | "panel" => s
                .padding(16.0)
                .margin_bottom(14.0)
                .border(1.0)
                .border_color(p.border)
                .border_radius(10.0)
                .background(p.panel),
            _ => s.padding_horiz(8.0).padding_vert(8.0).background(p.bg).min_height_full(),
        }
    })
    .into_any()
}

fn richtext_demo_view() -> AnyView {
    let rich = rich_text(move || {
        let dark = OS_THEME_IS_DARK.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|s| s.get())
                .unwrap_or(false)
        });
        let base = if dark {
            Color::rgb8(0xc0, 0xc6, 0xd4)
        } else {
            Color::rgb8(0x33, 0x36, 0x3f)
        };
        let hi = if dark {
            Color::rgb8(0x7d, 0xae, 0xff)
        } else {
            Color::rgb8(0x1d, 0x4e, 0xd8)
        };
        let text = "Rich text with a highlighted span.";
        let mut attrs = AttrsList::new(Attrs::new().color(base).font_size(15.0));
        if let Some(start) = text.find("highlighted") {
            let end = start + "highlighted".len();
            attrs.add_span(
                start..end,
                Attrs::new()
                    .color(hi)
                    .weight(Weight::BOLD)
                    .font_size(15.0),
            );
        }
        let mut tl = TextLayout::new();
        tl.set_text(text, attrs);
        tl
    });
    rich.style(|s| s.margin_top(6.0)).into_any()
}

fn texteditor_view(props: &ObjectMap) -> AnyView {
    let initial = props_string(props, &["value", "default", "defaultValue"])
        .unwrap_or_else(|| "// Tish + Floem\nfn hello() {\n    \"world\"\n}\n".to_string());
    let appearance = props_string(props, &["appearance", "theme"]).unwrap_or_else(|| "system".to_string());
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
    let child = if children.len() == 1 {
        value_into_any_view(children.into_iter().next().unwrap())
    } else {
        v_stack_dyn_children(children).into_any()
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
        .style(|s| s.width(32.px()).height(32.px()))
        .into_any()
}

fn clip_view(children: Vec<Value>) -> AnyView {
    let inner = if children.len() == 1 {
        value_into_any_view(children.into_iter().next().unwrap())
    } else {
        v_stack_dyn_children(children).into_any()
    };
    clip(container(inner).style(|s| s.padding(8.0)))
        .style(|s| s.width_full().max_width(320.0).height(48.0).border(1.0))
        .into_any()
}

fn tabpanel_view(props: &ObjectMap, children: Vec<Value>) -> AnyView {
    let active = props_f64(props, &["active"], f64::NAN) as i32;
    let id = props_f64(props, &["id", "panel"], -999.0) as i32;
    let body: AnyView = if children.len() == 1 {
        value_into_any_view(children.into_iter().next().unwrap())
    } else {
        v_stack_dyn_children(children).into_any()
    };
    container(body).style(move |s| {
        if active == id {
            s.width_full().padding(8.0)
        } else {
            s.hide()
        }
    })
    .into_any()
}

fn list_intrinsic_view(children: Vec<Value>) -> AnyView {
    let rows: Vec<_> = children
        .into_iter()
        .map(|c| value_into_any_view(c).into_view())
        .collect();
    list(rows)
        .style(|s| {
            s.width_full()
                .max_width(420.0)
                .border(1.0)
                .border_color(Color::rgb8(200, 200, 210))
                .border_radius(6.0)
        })
        .into_any()
}

fn dropdown_view(props: &ObjectMap, children: Vec<Value>) -> AnyView {
    let opts = dropdown_options(&children);
    let items: Vec<i32> = if opts.is_empty() {
        vec![1, 2, 3]
    } else {
        opts
    };
    let initial = props_f64(props, &["value", "defaultValue"], items[0] as f64) as i32;
    let sel = create_rw_signal(initial);
    let appearance = props_string(props, &["appearance", "theme"]).unwrap_or_else(|| "system".to_string());
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
    match v {
        Value::String(s) => {
            let t = s.as_ref().trim().to_string();
            if t.is_empty() {
                label(|| "").into_any()
            } else {
                label(move || t.clone())
                    .style(|s| s.font_size(14.0))
                    .into_any()
            }
        }
        Value::Object(rc) => {
            let map = rc.borrow();
            if is_fragment_object(&map) {
                let ch = vnode_children(&map);
                drop(map);
                return v_stack_dyn_children(ch).into_any();
            }
            let tag = map.get("tag").cloned();
            let props = vnode_props(&map);
            let children = vnode_children(&map);
            drop(map);

            match tag {
                Some(Value::String(t)) if t.as_ref() == "text" => text_view(&children, &props),
                Some(Value::String(t)) => {
                    let name_ref = t.as_ref();
                    if matches!(name_ref, "div" | "span" | "p") {
                        return html_css::html_element_view(name_ref, &props, children);
                    }
                    if let Some(lvl) = heading_level_from_tag(name_ref) {
                        return html_heading_view(lvl, &props, children);
                    }
                    let name = name_ref.to_string();
                    if let Some(kind) = intrinsic_for_tag(name.as_str()) {
                        return intrinsic_view(kind, &props, children);
                    }
                    v_stack((
                        label(move || format!("{}", name))
                            .style(|s| s.font_size(11.0).color(Color::GRAY)),
                        v_stack_dyn_children(children),
                    ))
                    .into_any()
                }
                _ => label(|| "(?)").into_any(),
            }
        }
        Value::Null => label(|| "").into_any(),
        _ => label(|| "").into_any(),
    }
}

fn stack_style_v() -> impl Fn(floem::style::Style) -> floem::style::Style + Copy {
    |s| {
        s.width_full()
            .row_gap(12.0)
            .items_start()
            .justify_start()
    }
}

fn stack_style_h() -> impl Fn(floem::style::Style) -> floem::style::Style + Copy {
    |s| {
        s.width_full()
            .column_gap(12.0)
            .items_center()
            .justify_start()
    }
}

fn intrinsic_view(kind: Intrinsic, props: &ObjectMap, children: Vec<Value>) -> floem::AnyView {
    let stack_props = props.clone();
    match kind {
        Intrinsic::VStack => {
            let stack = v_stack_from_iter(
                children
                    .into_iter()
                    .map(|c| value_into_any_view(c).into_view()),
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
                    .map(|c| value_into_any_view(c).into_view()),
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
            let variant = props_string(props, &["variant", "tone"])
                .unwrap_or_default()
                .to_ascii_lowercase();
            let active = props_bool(props, &["active", "selected"]);
            let appearance = props_string(props, &["appearance", "theme"]).unwrap_or_else(|| "system".to_string());
            let appearance_lbl = appearance.clone();
            let appearance_btn = appearance.clone();
            let variant_lbl = variant.clone();
            let variant_btn = variant.clone();
            let button_style_props = props.clone();
            let b = button(
                label(move || cap.clone()).style(move |s| {
                    let dark = effective_dark_from_appearance(appearance_lbl.as_str());
                    let p = palette_for_dark(dark);
                    s.font_size(14.0).font_bold().color(if active {
                        Color::WHITE
                    } else if variant_lbl == "ghost" {
                        p.fg
                    } else {
                        Color::WHITE
                    })
                }),
            )
            .style(move |s| {
                let dark = effective_dark_from_appearance(appearance_btn.as_str());
                let p = palette_for_dark(dark);
                let mut s = s
                    .padding_horiz(14.0)
                    .padding_vert(8.0)
                    .border_radius(8.0);
                if active {
                    s = s.background(p.accent).color(Color::WHITE).border(0.0);
                } else if variant_btn == "ghost" {
                    s = s
                        .background(Color::TRANSPARENT)
                        .color(p.fg)
                        .border(0.0)
                        .hover(|st| st.background(p.ghost_hover));
                } else {
                    s = s
                        .background(Color::rgb8(59, 130, 246))
                        .color(Color::WHITE)
                        .border(0.0);
                }
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
        Intrinsic::Scroll => {
            let inner: floem::AnyView = if children.len() == 1 {
                value_into_any_view(children.into_iter().next().unwrap())
            } else {
                v_stack_dyn_children(children).into_any()
            };
            let min_h = props_f64(props, &["minHeight", "min_height"], 360.0);
            let fill = props_string(props, &["variant", "layout"])
                .map(|v| v.to_ascii_lowercase() == "fill")
                .unwrap_or(false);
            let appearance = props_string(props, &["appearance", "theme"]).unwrap_or_else(|| "system".to_string());
            let scroll_style_inner = props.clone();
            let scroll_style_outer = props.clone();
            scroll(
                container(inner).style(move |s| {
                    let dark = effective_dark_from_appearance(appearance.as_str());
                    let p = palette_for_dark(dark);
                    let mut s = s.padding(12.0).width_full().background(p.bg);
                    if !fill {
                        s = s.min_height(min_h);
                    } else {
                        s = s.min_height(0.0).height_full();
                    }
                    html_css::merge_style_from_props(s, &scroll_style_inner)
                }),
            )
            .style(move |s| {
                let mut s = s.width_full().border_radius(8.0);
                if fill {
                    s = s.flex_grow(1.0).min_height(0.0).border(0.0);
                } else {
                    s = s
                        .height(220.0)
                        .border(1.0)
                        .border_color(Color::rgb8(210, 210, 220));
                }
                html_css::merge_style_from_props(s, &scroll_style_outer)
            })
            .into_any()
        }
        Intrinsic::Spacer => empty()
            .style(|s| {
                s.flex_grow(1.0)
                    .flex_basis(0.0)
                    .min_width(0.0)
                    .min_height(0.0)
            })
            .into_any(),
        Intrinsic::Divider => {
            let appearance = props_string(props, &["appearance", "theme"]).unwrap_or_else(|| "system".to_string());
            let divider_style_props = props.clone();
            empty()
                .style(move |s| {
                    let dark = effective_dark_from_appearance(appearance.as_str());
                    let c = if dark {
                        Color::rgb8(0x45, 0x4d, 0x5c)
                    } else {
                        Color::rgb8(200, 200, 210)
                    };
                    let s = s.height(1.0).width_full().background(c).margin_vert(12.0);
                    html_css::merge_style_from_props(s, &divider_style_props)
                })
                .into_any()
        }
        Intrinsic::Panel => {
            let appearance = props_string(props, &["appearance", "theme"]).unwrap_or_else(|| "system".to_string());
            let body: floem::AnyView = if children.len() == 1 {
                value_into_any_view(children.into_iter().next().unwrap())
            } else {
                v_stack_from_iter(
                    children
                        .into_iter()
                        .map(|c| value_into_any_view(c).into_view()),
                )
                .style(stack_style_v())
                .into_any()
            };
            let panel_style_props = props.clone();
            container(body)
                .style(move |s| {
                    let dark = effective_dark_from_appearance(appearance.as_str());
                    let p = palette_for_dark(dark);
                    let s = s
                        .width_full()
                        .padding(16.0)
                        .margin_bottom(14.0)
                        .border(1.0)
                        .border_color(p.border)
                        .border_radius(10.0)
                        .background(p.panel);
                    html_css::merge_style_from_props(s, &panel_style_props)
                })
                .into_any()
        }
        Intrinsic::Caption => caption_view(&children),
        Intrinsic::TextInput => {
            let initial = props_string(props, &["value", "defaultValue", "default"]).unwrap_or_default();
            let placeholder = props_string(props, &["placeholder", "hint"]).unwrap_or_default();
            let appearance = props_string(props, &["appearance", "theme"]).unwrap_or_else(|| "system".to_string());
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
        Intrinsic::Slider => {
            let start = props_f64(props, &["value", "defaultValue"], 40.0).clamp(0.0, 100.0);
            let pct = create_rw_signal(start.pct());
            Slider::new_rw(pct)
                .slider_style(|s| {
                    s.bar_color(Color::rgb8(220, 225, 235))
                        .accent_bar_color(Color::rgb8(59, 130, 246))
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
        Intrinsic::Toggle => {
            let on = create_rw_signal(false);
            toggle_button(move || on.get())
                .on_toggle(move |v| on.set(v))
                .style(|s| s.margin_vert(8.0))
                .into_any()
        }
        Intrinsic::Radiogroup => radiogroup_view(children),
        Intrinsic::Container => {
            let body: floem::AnyView = if children.len() == 1 {
                value_into_any_view(children.into_iter().next().unwrap())
            } else {
                v_stack_dyn_children(children).into_any()
            };
            let container_style_props = props.clone();
            container(body)
                .style(move |s| {
                    html_css::merge_style_from_props(s.width_full().padding(4.0), &container_style_props)
                })
                .into_any()
        }
        Intrinsic::Themebox => themebox_view(props, children),
        Intrinsic::Dropdown => dropdown_view(props, children),
        Intrinsic::Texteditor => texteditor_view(props),
        Intrinsic::Tooltip => tooltip_view(props, children),
        Intrinsic::Svgview => svg_view(props),
        Intrinsic::Imgdemo => imgdemo_view(),
        Intrinsic::Clip => clip_view(children),
        Intrinsic::Tabpanel => tabpanel_view(props, children),
        Intrinsic::List => list_intrinsic_view(children),
        Intrinsic::Richtext => richtext_demo_view(),
    }
}

fn v_stack_dyn_children(children: Vec<Value>) -> impl IntoView {
    v_stack_from_iter(children.into_iter().map(|child| value_into_any_view(child).into_view()))
        .style(|s| {
            s.width_full()
                .row_gap(20.0)
                .items_start()
                .justify_start()
        })
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

        v_stack((scroll(
            container(dyn_container(
                move || root.get(),
                move |v| value_into_any_view(v),
            ))
            .style(|s| s.width_full().min_height_full()),
        )
        .style(|s| s.flex_grow(1.0).min_height(0.0).width_full()),))
        .style(|s| s.width_full().height_full().min_height(0.0))
        .keyboard_navigable()
        .on_event(EventListener::ThemeChanged, move |e| {
            if let Event::ThemeChanged(t) = e {
                os_dark.set(Some(*t == Theme::Dark));
            }
            EventPropagation::Continue
        })
        .on_event(EventListener::KeyUp, |e| {
            if let Event::KeyUp(ev) = e {
                let is_tilde = match &ev.key.logical_key {
                    Key::Character(s) => s.as_str() == "~",
                    _ => false,
                };
                if is_tilde {
                    action::inspect();
                    return EventPropagation::Stop;
                }
            }
            EventPropagation::Continue
        })
    });
}

/// `import { floem } from 'tish:floem'` → object with `run(callback)`.
pub fn floem_object() -> Value {
    tishlang_core::tish_module! {
        "run" => |args: &[Value]| {
            if let Some(Value::Function(f)) = args.first() {
                floem_run(Rc::clone(f));
            }
            Value::Null
        },
    }
}
