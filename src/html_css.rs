//! Minimal HTML/CSS-like `style` parsing for Tish → Floem mapping.
//!
//! In JSX, prefer `style="display: flex"` or `style={{ display: "flex", flexDirection: "row" }}`
//! (not `style={"..."}` unless you need an arbitrary expression).

use floem::peniko::Color;
use floem::style::Style;
use floem::text::Weight;
use floem::unit::{PxPct, PxPctAuto};
use floem::views::{container, empty, Decorators, StackExt};
use floem::AnyView;
use floem::IntoView;
use floem::taffy::style::{AlignItems, Display, FlexDirection, FlexWrap, JustifyContent};
use tishlang_core::{ObjectMap, Value};

use crate::value_into_any_view;

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
        return Some(Color::TRANSPARENT);
    }
    if let Some(hex) = v.strip_prefix('#') {
        let hex = hex.trim();
        if hex.len() == 3 {
            let (r, g, b) = expand_hex3(hex)?;
            return Some(Color::rgb8(r, g, b));
        }
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Color::rgb8(r, g, b));
        }
        if hex.len() == 8 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            return Some(Color::rgba8(r, g, b, a));
        }
    }
    match v.to_ascii_lowercase().as_str() {
        "black" => Some(Color::BLACK),
        "white" => Some(Color::WHITE),
        "red" => Some(Color::RED),
        "green" => Some(Color::GREEN),
        "blue" => Some(Color::BLUE),
        "gray" | "grey" => Some(Color::GRAY),
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
                    s = s.justify_content(Some(j));
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
                    s = s.align_items(Some(a));
                }
            }
            "gap" => {
                if let Some(px) = parse_px_pct(v) {
                    s = s.row_gap(px).column_gap(px);
                }
            }
            "rowgap" => {
                if let Some(px) = parse_px_pct(v) {
                    s = s.row_gap(px);
                }
            }
            "columngap" => {
                if let Some(px) = parse_px_pct(v) {
                    s = s.column_gap(px);
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
                    s = s.font_weight(Weight::BOLD);
                } else if vl == "normal" || vl == "400" {
                    s = s.font_weight(Weight::NORMAL);
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

fn props_bool_element(props: &ObjectMap, keys: &[&str]) -> bool {
    for k in keys {
        match props.get(*k) {
            Some(Value::Bool(b)) => return *b,
            Some(Value::Number(n)) => return *n != 0.0,
            _ => {}
        }
    }
    false
}

/// `div` / `span` / `p` with HTML-like defaults and optional `style` string or object.
pub fn html_element_view(tag: &str, props: &ObjectMap, children: Vec<Value>) -> AnyView {
    let decls = style_declarations_from_props(props);
    if display_is_none(&decls) {
        return container(empty()).style(|s| s.hide()).into_any();
    }

    let muted = props_bool_element(props, &["muted", "dim"]);
    let tag_l = tag.to_ascii_lowercase();
    let default_dir = match tag_l.as_str() {
        "span" => FlexDirection::Row,
        "p" | "div" | _ => FlexDirection::Column,
    };

    let direction = direction_from_decls(&decls, default_dir);

    let views: Vec<_> = children
        .into_iter()
        .map(|c| value_into_any_view(c).into_view())
        .collect();

    let stack = views.stack(direction);

    stack
        .style(move |s| {
            let mut s = s.display(Display::Flex);
            if tag_l != "span" {
                s = s.width_full();
            }
            if tag_l == "p" {
                s = s.margin_bottom(8.0);
            }
            s = apply_declarations(s, &decls);
            if muted {
                s = s.color(Color::GRAY);
            }
            s
        })
        .into_any()
}
