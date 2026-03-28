//! Floem window entry for Tish: installs a [`tishlang_ui::Host`] that maps committed vnodes to Floem views.
//!
//! This crate lives in the **tish-floem** repository. The main **tish** compiler/runtime does not depend on Floem.

use std::rc::Rc;

use floem::action;
use floem::event::{Event, EventListener, EventPropagation};
use floem::keyboard::Key;
use floem::prelude::*;
use floem::views::slider::Slider;
use tishlang_core::{ObjectMap, Value};
use tishlang_ui::{install_thread_local_host, Host, FRAGMENT_SENTINEL};

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

#[derive(Clone, Copy)]
enum Intrinsic {
    VStack,
    HStack,
    Button,
    Scroll,
    Spacer,
    Divider,
    Panel,
    Heading,
    Caption,
    TextInput,
    Checkbox,
    Slider,
    Toggle,
    Radiogroup,
    Container,
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
        "heading" | "h1" | "title" => Some(Intrinsic::Heading),
        "caption" | "subtitle" => Some(Intrinsic::Caption),
        "textinput" | "text-input" | "input" => Some(Intrinsic::TextInput),
        "checkbox" => Some(Intrinsic::Checkbox),
        "slider" => Some(Intrinsic::Slider),
        "toggle" | "switch" => Some(Intrinsic::Toggle),
        "radiogroup" | "radio-group" => Some(Intrinsic::Radiogroup),
        "container" | "box" => Some(Intrinsic::Container),
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

/// Styled `<text>` / typography. Use `variant` prop: `heading`, `caption`, `body` (default).
fn text_view(children: &[Value], props: &ObjectMap) -> floem::AnyView {
    let text = collect_visible_text(children);
    let variant = props_string(props, &["variant", "size"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    label(move || text.clone())
        .style(move |s| {
            let mut s = s;
            match variant.as_str() {
                "heading" | "h1" | "title" => {
                    s = s.font_size(20.0).font_bold().margin_bottom(4.0)
                }
                "caption" | "small" | "subtitle" => {
                    s = s.font_size(12.0).color(Color::GRAY)
                }
                _ => {
                    s = s.font_size(14.0).line_height(1.35)
                }
            }
            s
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

fn heading_view(children: &[Value]) -> floem::AnyView {
    let text = collect_visible_text(children);
    label(move || text.clone())
        .style(|s| s.font_size(18.0).font_bold().margin_bottom(6.0))
        .into_any()
}

fn caption_view(children: &[Value]) -> floem::AnyView {
    let text = collect_visible_text(children);
    label(move || text.clone())
        .style(|s| s.font_size(12.0).color(Color::GRAY).margin_bottom(8.0))
        .into_any()
}

fn value_into_any_view(v: Value) -> floem::AnyView {
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
                    let name = t.as_ref().to_string();
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
    match kind {
        Intrinsic::VStack => v_stack_from_iter(
            children
                .into_iter()
                .map(|c| value_into_any_view(c).into_view()),
        )
        .style(stack_style_v())
        .into_any(),
        Intrinsic::HStack => h_stack_from_iter(
            children
                .into_iter()
                .map(|c| value_into_any_view(c).into_view()),
        )
        .style(stack_style_h())
        .into_any(),
        Intrinsic::Button => {
            let cap = button_caption(&children, props);
            let handler = props_fn(props, &["onClick", "onclick", "onTap", "ontap"]);
            let b = button(
                label(move || cap.clone()).style(|s| s.font_size(14.0).font_bold()),
            )
            .style(|s| {
                s.padding_horiz(18.0)
                    .padding_vert(10.0)
                    .background(Color::rgb8(59, 130, 246))
                    .color(Color::WHITE)
                    .border_radius(8.0)
                    .border(0.0)
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
            // Inner min-height forces overflow so the scroll bar is functional.
            let min_h = props_f64(props, &["minHeight", "min_height"], 360.0);
            scroll(
                container(inner).style(move |s| {
                    s.padding(12.0)
                        .min_height(min_h)
                        .width_full()
                        .background(Color::rgb8(252, 252, 254))
                }),
            )
            .style(|s| {
                s.height(220.0)
                    .width_full()
                    .border(1.0)
                    .border_color(Color::rgb8(210, 210, 220))
                    .border_radius(8.0)
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
        Intrinsic::Divider => empty()
            .style(|s| {
                s.height(1.0)
                    .width_full()
                    .background(Color::rgb8(200, 200, 210))
                    .margin_vert(12.0)
            })
            .into_any(),
        Intrinsic::Panel => {
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
            container(body)
                .style(|s| {
                    s.width_full()
                        .padding(16.0)
                        .margin_bottom(14.0)
                        .border(1.0)
                        .border_color(Color::rgb8(220, 222, 232))
                        .border_radius(10.0)
                        .background(Color::rgb8(248, 249, 252))
                })
                .into_any()
        }
        Intrinsic::Heading => heading_view(&children),
        Intrinsic::Caption => caption_view(&children),
        Intrinsic::TextInput => {
            let initial = props_string(props, &["value", "defaultValue", "default"]).unwrap_or_default();
            let placeholder = props_string(props, &["placeholder", "hint"]).unwrap_or_default();
            let buf = create_rw_signal(initial);
            let mut input = text_input(buf).style(|s| {
                s.width_full()
                    .max_width(400.0)
                    .padding_horiz(12.0)
                    .padding_vert(8.0)
                    .border(1.0)
                    .border_color(Color::rgb8(190, 192, 204))
                    .border_radius(6.0)
                    .background(Color::WHITE)
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
            container(body)
                .style(|s| s.width_full().padding(4.0))
                .into_any()
        }
    }
}

/// Vertical list of vnode children. Uses a real `v_stack` (column), not `dyn_stack` — Floem's
/// `dyn_stack` defaults to **row** flex direction, which flattened the whole app into one horizontal strip.
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
/// **Floem inspector:** press **`~`** (Shift+`` ` `` on US keyboards) while this window is focused.
/// Floem’s own inspector window still uses Shift+F11 to refresh; we only remap the **host** shortcut here.
pub fn floem_run(update: Rc<dyn Fn(&[Value]) -> Value>) {
    let root = RwSignal::new(Value::Null);
    install_thread_local_host(Box::new(FloemHost::new(root)));
    update(&[]);
    floem::launch(move || {
        v_stack((
            container(
                label(|| "Tish + Floem — kitchen sink").style(|s| {
                    s.font_size(17.0)
                        .font_bold()
                        .color(Color::rgb8(30, 30, 40))
                }),
            )
            .style(|s| {
                s.width_full()
                    .padding_horiz(16.0)
                    .padding_vert(12.0)
                    .border_bottom(1.0)
                    .border_color(Color::rgb8(220, 222, 232))
                    .background(Color::rgb8(245, 246, 250))
            }),
            scroll(
                h_stack((
                    empty().style(|s| {
                        s.flex_grow(1.0)
                            .flex_basis(0.0)
                            .min_width(0.0)
                            .min_height(1.0)
                    }),
                    container(dyn_container(
                        move || root.get(),
                        move |v| value_into_any_view(v),
                    ))
                    .style(|s| {
                        s.width_full()
                            .max_width(920.0)
                            .padding_horiz(12.0)
                            .padding_vert(16.0)
                    }),
                    empty().style(|s| {
                        s.flex_grow(1.0)
                            .flex_basis(0.0)
                            .min_width(0.0)
                            .min_height(1.0)
                    }),
                ))
                .style(|s| s.width_full().items_start()),
            )
            .style(|s| s.flex_grow(1.0).min_height(0.0).width_full()),
        ))
        .style(|s| s.width_full().height_full().min_height(0.0))
        .keyboard_navigable()
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
