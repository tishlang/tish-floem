//! Minimal HTML/CSS-like `style` parsing for Tish → Floem mapping.
//!
//! In JSX, prefer `style="display: flex"` or `style={{ display: "flex", flexDirection: "row" }}`
//! (not `style={"..."}` unless you need an arbitrary expression).
//!
//! **Themed surfaces:** colors come from the same stack as [`ThemeProvider`] (see
//! `theme_palette_for_vnode` in `lib.rs`) — no separate “theme toggle” on the element.
//! - `<aside>` → sidebar chrome.
//! - `<section>` / `<article>` with HTML `class` / `className` containing `panel` or `card` → panel
//!   chrome. Plain `<section>` / `<article>` without those class tokens are unchanged.
//! The `style` prop is merged after these baselines.

use floem::peniko::Color;
use floem::peniko::color::palette::css;
use floem::style::Style;
use floem::text::FontWeight;
use floem::unit::{PxPct, PxPctAuto};
use floem::views::{container, empty, h_stack, label, v_stack_from_iter, Decorators, StackExt};
use floem::AnyView;
use floem::View;
use floem::IntoView;
use floem::taffy::style::{AlignItems, Display, FlexDirection, FlexWrap, JustifyContent};
use tishlang_core::{ObjectMap, Value};

use crate::{
    props_f64, props_string, register_scroll_anchor, resolve_appearance, theme_palette_for_vnode,
    value_into_any_view_impl, ThemePalette,
};

#[derive(Clone, Copy)]
enum HtmlThemedSurface {
    Panel,
    Sidebar,
}

fn class_string_from_props(props: &ObjectMap) -> Option<String> {
    props_string(props, &["class", "className"])
}

fn html_themed_surface(tag: &str, props: &ObjectMap) -> Option<HtmlThemedSurface> {
    match tag.to_ascii_lowercase().as_str() {
        "aside" => Some(HtmlThemedSurface::Sidebar),
        "section" | "article" => {
            let cls = class_string_from_props(props)?;
            let hit = cls
                .split_whitespace()
                .map(|w| w.to_ascii_lowercase())
                .any(|w| w == "panel" || w == "card");
            if hit {
                Some(HtmlThemedSurface::Panel)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn themed_panel_surface(s: Style, p: &ThemePalette) -> Style {
    s.padding(16.0)
        .margin_bottom(14.0)
        .border(1.0)
        .border_color(p.border)
        .border_radius(10.0)
        .background(p.panel)
        .color(p.fg)
}

fn themed_sidebar_surface(s: Style, p: &ThemePalette) -> Style {
    s.height_full()
        .min_height(0.0)
        .flex_shrink(0.0)
        .flex_col()
        .align_items(AlignItems::Stretch)
        .padding(0.0)
        .border_right(1.0)
        .border_color(p.border)
        .background(p.sidebar)
        .color(p.fg)
}

fn norm_key(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Split `display:flex; flex-direction: row` into normalized keys and raw values.
pub fn parse_inline_style_string(css: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for part in css.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((k, v)) = part.split_once(':') {
            let k = norm_key(k);
            if !k.is_empty() {
                out.push((k, v.trim().to_string()));
            }
        }
    }
    out
}

fn style_object_pairs(map: &ObjectMap) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (k, v) in map.iter() {
        let key = norm_key(k.as_ref());
        let val = match v {
            Value::String(s) => s.as_ref().to_string(),
            Value::Number(n) => {
                if (*n - n.round()).abs() < f64::EPSILON {
                    format!("{}", *n as i64)
                } else {
                    format!("{}", n)
                }
            }
            Value::Bool(b) => {
                if *b {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            }
            _ => continue,
        };
        out.push((key, val));
    }
    out
}

/// Declarations from `style` prop: either a string or an object (React-like `{ display: "flex" }`).
pub fn style_declarations_from_props(props: &ObjectMap) -> Vec<(String, String)> {
    if let Some(Value::String(s)) = props.get("style") {
        return parse_inline_style_string(s.as_ref());
    }
    if let Some(Value::Object(o)) = props.get("style") {
        return style_object_pairs(&o.borrow());
    }
    Vec::new()
}

/// `true` when `style` uses flex growth (`flex: 1`, `flex-grow: 1`, etc.). Hosts use the same rule:
/// a flex child scroll region should set `flex: 1; min-height: 0` (and often `min-width: 0`) in `style`.
pub fn scroll_fill_from_style(props: &ObjectMap) -> bool {
    scroll_fill_from_decls(&style_declarations_from_props(props))
}

/// Merge `style` onto the Floem `Scroll` host after programmatic flex-fill defaults.
///
/// Applies every declaration [`apply_declarations`] supports except `overflow` / `overflow-x` /
/// `overflow-y` (the host implements scrolling). That way `width: 100%`, `height: 100%`, `flex`,
/// `min-width`, etc. from the authored `div` are parsed (`parse_px_pct` / `parse_px_pct_auto`)
/// and applied instead of being dropped.
///
/// **Fill-mode hosts:** [`crate::scroll_host_viewport`] merges this style, then sets `height: auto`
/// so `height: 100%` cannot collapse the viewport when the flex percentage base is indefinite.
pub fn merge_scroll_host_style_from_props(s: Style, props: &ObjectMap) -> Style {
    let decls = style_declarations_from_props(props);
    let filtered: Vec<(String, String)> = decls
        .into_iter()
        .filter(|(k, _)| !is_overflow_style_key(k.as_str()))
        .collect();
    if filtered.is_empty() {
        return s;
    }
    apply_declarations(s, &filtered)
}

fn scroll_fill_from_decls(decls: &[(String, String)]) -> bool {
    for (key, val) in decls {
        let v = val.trim();
        match key.as_str() {
            "flexgrow" => {
                if let Ok(n) = v.parse::<f32>() {
                    if n > 0.0 {
                        return true;
                    }
                }
            }
            "flex" => {
                if v.eq_ignore_ascii_case("none") {
                    continue;
                }
                let parts: Vec<&str> = v.split_whitespace().collect();
                let grow = if parts.len() == 1 {
                    parts[0].parse::<f32>().ok()
                } else {
                    parts.first().and_then(|x| x.parse::<f32>().ok())
                };
                if grow.map(|g| g > 0.0).unwrap_or(false) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn overflow_value_is_scroll(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "auto" | "scroll" | "overlay"
    )
}

/// `overflow` / `overflow-x` / `overflow-y` of `auto`, `scroll`, or `overlay` → use a scroll viewport (CSS semantics).
pub fn overflow_enables_scroll(decls: &[(String, String)]) -> bool {
    decls.iter().any(|(k, v)| {
        matches!(k.as_str(), "overflow" | "overflowx" | "overflowy") && overflow_value_is_scroll(v)
    })
}

pub fn is_overflow_style_key(k: &str) -> bool {
    matches!(k, "overflow" | "overflowx" | "overflowy")
}

/// Sizing and margin that belong on the scroll **viewport**; inner stack keeps flex, gap, padding, borders, etc.
pub fn is_scroll_host_outer_key(k: &str) -> bool {
    matches!(
        k,
        "width"
            | "height"
            | "minwidth"
            | "minheight"
            | "maxwidth"
            | "maxheight"
            | "flex"
            | "flexgrow"
            | "flexshrink"
            | "flexbasis"
            | "margin"
            | "margintop"
            | "marginright"
            | "marginbottom"
            | "marginleft"
    )
}

pub fn decls_for_scrollable_div_inner(decls: &[(String, String)]) -> Vec<(String, String)> {
    decls
        .iter()
        .cloned()
        .filter(|(k, _)| {
            !is_overflow_style_key(k.as_str()) && !is_scroll_host_outer_key(k.as_str())
        })
        .collect()
}

fn parse_px_pct_auto(v: &str) -> Option<PxPctAuto> {
    let v = v.trim();
    if v.eq_ignore_ascii_case("auto") {
        return Some(PxPctAuto::Auto);
    }
    if let Some(p) = v.strip_suffix('%').map(str::trim) {
        return p.parse::<f64>().ok().map(PxPctAuto::Pct);
    }
    if let Some(p) = v.strip_suffix("px").map(str::trim) {
        return p.parse::<f64>().ok().map(PxPctAuto::Px);
    }
    v.parse::<f64>().ok().map(PxPctAuto::Px)
}

fn parse_px_pct(v: &str) -> Option<PxPct> {
    let v = v.trim();
    if let Some(p) = v.strip_suffix('%').map(str::trim) {
        return p.parse::<f64>().ok().map(PxPct::Pct);
    }
    if let Some(p) = v.strip_suffix("px").map(str::trim) {
        return p.parse::<f64>().ok().map(PxPct::Px);
    }
    v.parse::<f64>().ok().map(PxPct::Px)
}

fn expand_hex3(h: &str) -> Option<(u8, u8, u8)> {
    if h.len() != 3 {
        return None;
    }
    let r = u8::from_str_radix(&format!("{}{}", &h[0..1], &h[0..1]), 16).ok()?;
    let g = u8::from_str_radix(&format!("{}{}", &h[1..2], &h[1..2]), 16).ok()?;
    let b = u8::from_str_radix(&format!("{}{}", &h[2..3], &h[2..3]), 16).ok()?;
    Some((r, g, b))
}

fn parse_color(v: &str) -> Option<Color> {
    let v = v.trim();
    if v.eq_ignore_ascii_case("transparent") {
        return Some(css::TRANSPARENT);
    }
    if let Some(hex) = v.strip_prefix('#') {
        let hex = hex.trim();
        if hex.len() == 3 {
            let (r, g, b) = expand_hex3(hex)?;
            return Some(Color::from_rgb8(r, g, b));
        }
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Color::from_rgb8(r, g, b));
        }
        if hex.len() == 8 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            return Some(Color::from_rgba8(r, g, b, a));
        }
    }
    match v.to_ascii_lowercase().as_str() {
        "black" => Some(css::BLACK),
        "white" => Some(css::WHITE),
        "red" => Some(css::RED),
        "green" => Some(css::GREEN),
        "blue" => Some(css::BLUE),
        "gray" | "grey" => Some(css::GRAY),
        _ => None,
    }
}

/// Apply parsed declarations to a Floem [`Style`].
pub fn apply_declarations(mut s: Style, decls: &[(String, String)]) -> Style {
    for (key, val) in decls {
        let v = val.trim();
        match key.as_str() {
            "display" => {
                let vl = v.to_ascii_lowercase();
                if vl == "none" {
                    s = s.display(Display::None);
                } else if vl == "flex" {
                    s = s.display(Display::Flex);
                } else if vl == "block" || vl == "flow" || vl == "flowroot" {
                    s = s.display(Display::Flex).flex_direction(FlexDirection::Column);
                } else if vl == "inline" || vl == "inlineblock" {
                    s = s.display(Display::Flex).flex_direction(FlexDirection::Row);
                }
            }
            "flexdirection" => {
                let vl = v.to_ascii_lowercase().replace(' ', "");
                if vl == "row" {
                    s = s.flex_direction(FlexDirection::Row);
                } else if vl == "column" {
                    s = s.flex_direction(FlexDirection::Column);
                } else if vl == "rowreverse" {
                    s = s.flex_direction(FlexDirection::RowReverse);
                } else if vl == "columnreverse" {
                    s = s.flex_direction(FlexDirection::ColumnReverse);
                }
            }
            "flexwrap" => {
                let vl = v.to_ascii_lowercase();
                if vl == "wrap" {
                    s = s.flex_wrap(FlexWrap::Wrap);
                } else if vl == "nowrap" {
                    s = s.flex_wrap(FlexWrap::NoWrap);
                }
            }
            "justifycontent" => {
                let vl = v.to_ascii_lowercase().replace(' ', "").replace('-', "");
                let jc = match vl.as_str() {
                    "flexstart" | "start" => Some(JustifyContent::FlexStart),
                    "center" => Some(JustifyContent::Center),
                    "flexend" | "end" => Some(JustifyContent::FlexEnd),
                    "spacebetween" => Some(JustifyContent::SpaceBetween),
                    "spacearound" => Some(JustifyContent::SpaceAround),
                    "spaceevenly" => Some(JustifyContent::SpaceEvenly),
                    _ => None,
                };
                if let Some(j) = jc {
                    s = s.justify_content(j);
                }
            }
            "alignitems" => {
                let vl = v.to_ascii_lowercase().replace(' ', "").replace('-', "");
                let ai = match vl.as_str() {
                    "stretch" => Some(AlignItems::Stretch),
                    "flexstart" | "start" => Some(AlignItems::FlexStart),
                    "center" => Some(AlignItems::Center),
                    "flexend" | "end" => Some(AlignItems::FlexEnd),
                    "baseline" => Some(AlignItems::Baseline),
                    _ => None,
                };
                if let Some(a) = ai {
                    s = s.align_items(a);
                }
            }
            "gap" => {
                if let Some(px) = parse_px_pct(v) {
                    s = s.row_gap(px).col_gap(px);
                }
            }
            "rowgap" => {
                if let Some(px) = parse_px_pct(v) {
                    s = s.row_gap(px);
                }
            }
            "columngap" => {
                if let Some(px) = parse_px_pct(v) {
                    s = s.col_gap(px);
                }
            }
            "padding" => {
                if let Some(px) = parse_px_pct(v) {
                    s = s.padding(px);
                }
            }
            "paddingtop" => {
                if let Some(px) = parse_px_pct(v) {
                    s = s.padding_top(px);
                }
            }
            "paddingright" => {
                if let Some(px) = parse_px_pct(v) {
                    s = s.padding_right(px);
                }
            }
            "paddingbottom" => {
                if let Some(px) = parse_px_pct(v) {
                    s = s.padding_bottom(px);
                }
            }
            "paddingleft" => {
                if let Some(px) = parse_px_pct(v) {
                    s = s.padding_left(px);
                }
            }
            "margin" => {
                if let Some(m) = parse_px_pct_auto(v) {
                    s = s.margin(m);
                }
            }
            "margintop" => {
                if let Some(m) = parse_px_pct_auto(v) {
                    s = s.margin_top(m);
                }
            }
            "marginright" => {
                if let Some(m) = parse_px_pct_auto(v) {
                    s = s.margin_right(m);
                }
            }
            "marginbottom" => {
                if let Some(m) = parse_px_pct_auto(v) {
                    s = s.margin_bottom(m);
                }
            }
            "marginleft" => {
                if let Some(m) = parse_px_pct_auto(v) {
                    s = s.margin_left(m);
                }
            }
            "width" => {
                if let Some(d) = parse_px_pct_auto(v) {
                    s = s.width(d);
                }
            }
            "height" => {
                if let Some(d) = parse_px_pct_auto(v) {
                    s = s.height(d);
                }
            }
            "minwidth" => {
                if let Some(d) = parse_px_pct_auto(v) {
                    s = s.min_width(d);
                }
            }
            "minheight" => {
                if let Some(d) = parse_px_pct_auto(v) {
                    s = s.min_height(d);
                }
            }
            "maxwidth" => {
                if let Some(d) = parse_px_pct_auto(v) {
                    s = s.max_width(d);
                }
            }
            "maxheight" => {
                if let Some(d) = parse_px_pct_auto(v) {
                    s = s.max_height(d);
                }
            }
            "flex" => {
                let vn = v.trim();
                if vn.eq_ignore_ascii_case("none") {
                    s = s.flex_grow(0.0).flex_shrink(0.0);
                } else {
                    let parts: Vec<&str> = vn.split_whitespace().collect();
                    if parts.len() == 1 {
                        if let Ok(g) = parts[0].parse::<f32>() {
                            // flex: 1  →  grow 1, shrink 1, basis 0 (scroll/flex layouts)
                            s = s.flex_grow(g).flex_shrink(1.0).flex_basis(0.0);
                        }
                    } else {
                        let g = parts
                            .first()
                            .and_then(|x| x.parse::<f32>().ok())
                            .unwrap_or(0.0);
                        let sh = parts
                            .get(1)
                            .and_then(|x| x.parse::<f32>().ok())
                            .unwrap_or(1.0);
                        s = s.flex_grow(g).flex_shrink(sh);
                        if let Some(b) = parts.get(2) {
                            if let Some(fb) = parse_px_pct_auto(b.trim()) {
                                s = s.flex_basis(fb);
                            }
                        }
                    }
                }
            }
            "flexbasis" => {
                if let Some(fb) = parse_px_pct_auto(v) {
                    s = s.flex_basis(fb);
                }
            }
            "flexgrow" => {
                if let Ok(n) = v.parse::<f32>() {
                    s = s.flex_grow(n);
                }
            }
            "flexshrink" => {
                if let Ok(n) = v.parse::<f32>() {
                    s = s.flex_shrink(n);
                }
            }
            "alignself" => {
                let vl = v.to_ascii_lowercase().replace(' ', "").replace('-', "");
                let ai = match vl.as_str() {
                    "stretch" => Some(AlignItems::Stretch),
                    "flexstart" | "start" => Some(AlignItems::FlexStart),
                    "center" => Some(AlignItems::Center),
                    "flexend" | "end" => Some(AlignItems::FlexEnd),
                    "baseline" => Some(AlignItems::Baseline),
                    _ => None,
                };
                if let Some(a) = ai {
                    s = s.align_self(a);
                }
            }
            "bordercolor" => {
                if let Some(c) = parse_color(v) {
                    s = s.border_color(c);
                }
            }
            "borderright" => {
                if let Some(tok) = v.split_whitespace().next() {
                    let t = tok.trim_end_matches("px").trim();
                    if let Ok(p) = t.parse::<f32>() {
                        s = s.border_right(p);
                    }
                }
            }
            "borderradius" => {
                if let Some(px) = parse_px_pct(v) {
                    s = s.border_radius(px);
                }
            }
            "border" => {
                let lower = v.to_ascii_lowercase();
                if lower.contains("none") {
                    s = s.border(0.0);
                } else if let Some(px) = v.split_whitespace().next().and_then(parse_px_pct) {
                    if let PxPct::Px(p) = px {
                        s = s.border(p);
                    }
                }
            }
            "background" | "backgroundcolor" => {
                if let Some(c) = parse_color(v) {
                    s = s.background(c);
                }
            }
            "color" => {
                if let Some(c) = parse_color(v) {
                    s = s.color(c);
                }
            }
            "fontsize" => {
                if let Ok(n) = v.trim_end_matches("px").trim().parse::<f32>() {
                    s = s.font_size(n);
                }
            }
            "fontweight" => {
                let vl = v.to_ascii_lowercase();
                if vl == "bold" || vl == "700" {
                    s = s.font_weight(FontWeight::BOLD);
                } else if vl == "normal" || vl == "400" {
                    s = s.font_weight(FontWeight::NORMAL);
                }
            }
            "fontfamily" => {
                if !v.is_empty() {
                    s = s.font_family(v.to_string());
                }
            }
            _ => {}
        }
    }
    s
}

/// Merge `style` prop (string or object) into an existing style.
pub fn merge_style_from_props(s: Style, props: &ObjectMap) -> Style {
    let decls = style_declarations_from_props(props);
    if decls.is_empty() {
        return s;
    }
    apply_declarations(s, &decls)
}

fn direction_from_decls(decls: &[(String, String)], fallback: FlexDirection) -> FlexDirection {
    let mut d = fallback;
    for (k, v) in decls {
        if k != "flexdirection" {
            continue;
        }
        let vl = v.to_ascii_lowercase().replace(' ', "");
        d = match vl.as_str() {
            "row" => FlexDirection::Row,
            "column" => FlexDirection::Column,
            "rowreverse" => FlexDirection::RowReverse,
            "columnreverse" => FlexDirection::ColumnReverse,
            _ => d,
        };
    }
    d
}

fn display_is_none(decls: &[(String, String)]) -> bool {
    decls.iter().any(|(k, v)| {
        k == "display" && v.trim().eq_ignore_ascii_case("none")
    })
}

/// `div` / `span` / `p` with HTML-like defaults and optional `style` string or object.
pub fn html_element_view(tag: &str, props: &ObjectMap, children: Vec<Value>) -> AnyView {
    let decls = style_declarations_from_props(props);
    if display_is_none(&decls) {
        return container(empty()).style(|s| s.hide()).into_any();
    }
    let child_inh = Some(resolve_appearance(props));

    let tag_l = tag.to_ascii_lowercase();
    let default_dir = match tag_l.as_str() {
        "span" | "label" | "nav" => FlexDirection::Row,
        "p" | "div" | "section" | "article" | "aside" | "fieldset" | "ul" | "ol" | "li" | _ => {
            FlexDirection::Column
        }
    };

    let direction = direction_from_decls(&decls, default_dir);
    let anchor_key = props_string(props, &["id"]);

    if tag_l == "ul" || tag_l == "ol" {
        let is_ordered = tag_l == "ol";
        let list_style_props = props.clone();
        let rows: Vec<_> = children
            .into_iter()
            .enumerate()
            .map(|(idx, c)| {
                let (body_vals, row_idx) = match &c {
                    Value::Object(o) => {
                        let m = o.borrow();
                        let t = m
                            .get("tag")
                            .and_then(|v| match v {
                                Value::String(s) => Some(s.as_ref().to_string()),
                                _ => None,
                            })
                            .unwrap_or_default();
                        let ch = match m.get("children") {
                            Some(Value::Array(a)) => a.borrow().clone(),
                            _ => vec![],
                        };
                        drop(m);
                        if t.eq_ignore_ascii_case("li") {
                            (ch, idx)
                        } else {
                            (vec![c], idx)
                        }
                    }
                    _ => (vec![c], idx),
                };
                let prefix = if is_ordered {
                    format!("{}. ", row_idx + 1)
                } else {
                    "• ".to_string()
                };
                let inner: AnyView = if body_vals.len() == 1 {
                    value_into_any_view_impl(
                        body_vals.into_iter().next().unwrap(),
                        child_inh.clone(),
                    )
                } else {
                    let vs: Vec<_> = body_vals
                        .into_iter()
                        .map(|x| value_into_any_view_impl(x, child_inh.clone()).into_view())
                        .collect();
                    vs.stack(FlexDirection::Column).into_any()
                };
                h_stack((
                    label(move || prefix.clone()).style(|s| s.min_width(22.0)),
                    inner.into_view(),
                ))
                .style(|s| s.items_start())
                .into_view()
            })
            .collect();
        let stack = v_stack_from_iter(rows).style(move |s| {
            let mut s = s.display(Display::Flex).flex_direction(FlexDirection::Column);
            s = apply_declarations(s, &decls);
            let sizing_from_style = decls.iter().any(|(k, _)| {
                matches!(
                    k.as_str(),
                    "width" | "flex" | "flexgrow" | "minwidth" | "maxwidth"
                )
            });
            if !sizing_from_style {
                s = s.width_full();
            }
            merge_style_from_props(s, &list_style_props)
        });
        if let Some(key) = anchor_key {
            let c = container(stack).style(|s| s.width_full());
            register_scroll_anchor(key, c.id());
            return c.into_any();
        }
        return stack.into_any();
    }

    let views: Vec<_> = children
        .into_iter()
        .map(|c| value_into_any_view_impl(c, child_inh.clone()).into_view())
        .collect();

    if tag_l == "div" && overflow_enables_scroll(&decls) {
        let inner_decl = decls_for_scrollable_div_inner(&decls);
        let fill = scroll_fill_from_style(props);
        let min_h = props_f64(props, &["minHeight", "min_height"], 120.0);
        let inner_decl_clone = inner_decl.clone();
        let stack = views.stack(direction);
        let scroll_body = stack.style(move |s| {
            let mut s = s.display(Display::Flex);
            s = apply_declarations(s, &inner_decl_clone);
            // Only suppress width_full() when the author set an explicit concrete width or flex-grow
            // on the outer scroll div. min-width/max-width are flex resets, not width specs, so we
            // must NOT count them — otherwise `min-width: 0` (a common flex trick) silently removes
            // width_full() and collapses the scroll content to a narrow natural width.
            let explicit_width = inner_decl_clone.iter().any(|(k, _)| {
                matches!(k.as_str(), "width" | "flex" | "flexgrow")
            });
            if !explicit_width || fill {
                // Fill-mode always gets full width so percentage-width children resolve correctly.
                s = s.width_full();
            }
            if fill {
                // Scrollable document: keep intrinsic content height (do not shrink to the viewport).
                s = s.flex_grow(0.0).flex_shrink(0.0);
            }
            s
        });
        let wrapped = container(scroll_body).style(move |s| {
            let mut s = s.width_full().display(Display::Flex).flex_col();
            s = if fill {
                s.align_items(AlignItems::Stretch)
            } else {
                s.items_start()
            };
            if fill {
                s = s.min_height(0.0).flex_grow(0.0).flex_shrink(0.0);
            } else {
                s = s.min_height(min_h);
            }
            s
        });
        let mut out = crate::scroll_host_viewport(props, wrapped.into_any());
        if let Some(key) = anchor_key {
            let c = container(out).style(|s| s.width_full());
            register_scroll_anchor(key, c.id());
            out = c.into_any();
        }
        return out;
    }

    let stack = views.stack(direction);

    let style_props = props.clone();
    let surface = html_themed_surface(&tag_l, props);
    let decls_styled = decls.clone();
    let tag_for_width = tag_l.clone();

    let body = stack.style(move |s| {
        let mut s = s.display(Display::Flex);
        if let Some(surf) = surface {
            let pal = theme_palette_for_vnode(&style_props);
            s = match surf {
                HtmlThemedSurface::Panel => themed_panel_surface(s, &pal),
                HtmlThemedSurface::Sidebar => themed_sidebar_surface(s, &pal),
            };
        }
        s = apply_declarations(s, &decls_styled);
        // Percent height on `<aside>` after flex merge often collapses to 0; `height_full` matches
        // ThemeProvider layouts without a second theme attribute.
        if tag_for_width == "aside"
            && decls_styled
                .iter()
                .any(|(k, v)| k == "height" && v.trim().contains('%'))
        {
            s = s.height_full();
        }
        // Do not force width: 100% when the author sets an explicit width or flex-grow (row
        // children need intrinsic width). min-width/max-width are flex resets, not width specs.
        let explicit_width = decls_styled.iter().any(|(k, _)| {
            matches!(k.as_str(), "width" | "flex" | "flexgrow")
        });
        if tag_for_width != "span" && !explicit_width {
            s = s.width_full();
        }
        s
    });
    if let Some(key) = anchor_key {
        let c = container(body).style(|s| s.width_full());
        register_scroll_anchor(key, c.id());
        c.into_any()
    } else {
        body.into_any()
    }
}
