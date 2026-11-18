//! The Shepherd design document format.
//!
//! Agents write JSON files into `<project>/shepherd-design/`:
//! - `system.json` — the design system (tokens + components)
//! - `screens/*.json` — one file per mockup screen
//!
//! Parsing is deliberately permissive: numbers may be strings, unknown keys are ignored,
//! and most fields are optional, so a slightly-off agent output still renders.

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DesignSystem {
    pub name: String,
    pub description: Option<String>,
    pub tokens: Tokens,
    pub components: Vec<Component>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Tokens {
    /// Ordered so swatches render in the author's order.
    pub colors: IndexedMap<String>,
    pub typography: IndexedMap<TypeStyle>,
    pub spacing: IndexedMap<f32Value>,
    pub radii: IndexedMap<f32Value>,
    pub shadows: IndexedMap<Shadow>,
}

/// Wrapper so token scales accept `4`, `"4"` or `"4px"`.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq)]
pub struct f32Value(pub f32);

impl<'de> Deserialize<'de> for f32Value {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        Ok(f32Value(value_to_f32(&v).unwrap_or(0.0)))
    }
}

pub fn value_to_f32(v: &serde_json::Value) -> Option<f32> {
    match v {
        serde_json::Value::Number(n) => n.as_f64().map(|f| f as f32),
        serde_json::Value::String(s) => {
            let t = s.trim().trim_end_matches("px").trim_end_matches("pt").trim_end_matches("rem");
            t.parse::<f32>().ok().map(|f| if s.ends_with("rem") { f * 16.0 } else { f })
        }
        _ => None,
    }
}

/// An insertion-ordered string map (JSON object order is preserved).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IndexedMap<T>(pub Vec<(String, T)>);

impl<T> Default for IndexedMap<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T> IndexedMap<T> {
    pub fn get(&self, key: &str) -> Option<&T> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    pub fn iter(&self) -> impl Iterator<Item = &(String, T)> {
        self.0.iter()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl<'de, T: serde::de::DeserializeOwned> Deserialize<'de> for IndexedMap<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // serde_json is built without preserve_order, so walk the raw object ourselves.
        struct V<T>(std::marker::PhantomData<T>);
        impl<'de, T: serde::de::DeserializeOwned> serde::de::Visitor<'de> for V<T> {
            type Value = IndexedMap<T>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some((k, v)) = map.next_entry::<String, serde_json::Value>()? {
                    if let Ok(t) = serde_json::from_value::<T>(v) {
                        out.push((k, t));
                    }
                }
                Ok(IndexedMap(out))
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(IndexedMap(Vec::new()))
            }
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TypeStyle {
    #[serde(deserialize_with = "de_opt_f32")]
    pub size: Option<f32>,
    #[serde(deserialize_with = "de_opt_f32")]
    pub weight: Option<f32>,
    #[serde(deserialize_with = "de_opt_f32")]
    pub line_height: Option<f32>,
    #[serde(deserialize_with = "de_opt_f32")]
    pub letter_spacing: Option<f32>,
    pub family: Option<String>,
    pub italic: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Shadow {
    #[serde(deserialize_with = "de_opt_f32")]
    pub x: Option<f32>,
    #[serde(deserialize_with = "de_opt_f32")]
    pub y: Option<f32>,
    #[serde(deserialize_with = "de_opt_f32")]
    pub blur: Option<f32>,
    #[serde(deserialize_with = "de_opt_f32")]
    pub spread: Option<f32>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Component {
    pub name: String,
    pub description: Option<String>,
    pub variants: Vec<Variant>,
    /// Shorthand for a single-variant component.
    pub node: Option<Node>,
}

impl Component {
    pub fn variants(&self) -> Vec<Variant> {
        if !self.variants.is_empty() {
            self.variants.clone()
        } else if let Some(node) = &self.node {
            vec![Variant { name: "Default".into(), node: node.clone() }]
        } else {
            vec![]
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Variant {
    pub name: String,
    pub node: Node,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    #[default]
    Desktop,
    Tablet,
    Mobile,
    Watch,
}

impl Device {
    pub fn size(self) -> (f32, f32) {
        match self {
            Device::Desktop => (1280.0, 800.0),
            Device::Tablet => (834.0, 1112.0),
            Device::Mobile => (390.0, 844.0),
            Device::Watch => (198.0, 242.0),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Screen {
    pub name: String,
    pub description: Option<String>,
    pub device: Device,
    #[serde(deserialize_with = "de_opt_f32")]
    pub width: Option<f32>,
    #[serde(deserialize_with = "de_opt_f32")]
    pub height: Option<f32>,
    pub background: Option<String>,
    /// Theme override: "light" / "dark" (purely informational).
    pub theme: Option<String>,
    pub root: Node,
    /// Filled in by the loader.
    #[serde(skip)]
    pub file: String,
}

impl Screen {
    pub fn size(&self) -> (f32, f32) {
        let (w, h) = self.device.size();
        (self.width.unwrap_or(w), self.height.unwrap_or(h))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    /// Flex container; direction from style.direction (default column).
    #[default]
    #[serde(alias = "frame", alias = "view", alias = "div", alias = "container", alias = "card")]
    Box,
    #[serde(alias = "hstack", alias = "horizontal")]
    Row,
    #[serde(alias = "vstack", alias = "vertical")]
    Column,
    /// Children overlap (absolute).
    #[serde(alias = "zstack", alias = "overlay")]
    Stack,
    #[serde(alias = "label", alias = "heading", alias = "paragraph")]
    Text,
    Icon,
    #[serde(alias = "img", alias = "avatar")]
    Image,
    Spacer,
    #[serde(alias = "separator", alias = "hr")]
    Divider,
    #[serde(alias = "textfield", alias = "text_field", alias = "textarea")]
    Input,
    Button,
    #[serde(alias = "toggle", alias = "switch")]
    Switch,
    Checkbox,
    #[serde(alias = "progress_bar")]
    Progress,
    /// Instance of a design-system component: `"ref": "Button / Primary"`, optional `"variant"`.
    #[serde(alias = "instance", alias = "component")]
    Ref,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Node {
    #[serde(rename = "type")]
    pub kind: NodeType,
    pub name: Option<String>,
    pub text: Option<String>,
    pub icon: Option<String>,
    /// For inputs: placeholder; for images: alt text.
    pub placeholder: Option<String>,
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub variant: Option<String>,
    /// For switch / checkbox.
    pub checked: Option<bool>,
    /// For progress: 0..1.
    #[serde(deserialize_with = "de_opt_f32")]
    pub value: Option<f32>,
    pub style: Style,
    pub children: Vec<Node>,
    /// Text overrides when instantiating a component `{ "label": "Save" }` replaces text of the
    /// first text node named "label".
    pub overrides: BTreeMap<String, String>,
}

/// A dimension: fixed px, "fill" (grow), "hug" (content), or a percentage "50%".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Default)]
pub enum Dim {
    #[default]
    Auto,
    Px(f32),
    Fill,
    Percent(f32),
}

impl<'de> Deserialize<'de> for Dim {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        Ok(match &v {
            serde_json::Value::String(s) => {
                let s = s.trim();
                match s {
                    "fill" | "full" | "stretch" | "100%" => Dim::Fill,
                    "hug" | "auto" | "fit" | "content" => Dim::Auto,
                    _ if s.ends_with('%') => s.trim_end_matches('%').parse().map(Dim::Percent).unwrap_or(Dim::Auto),
                    _ => value_to_f32(&v).map(Dim::Px).unwrap_or(Dim::Auto),
                }
            }
            _ => value_to_f32(&v).map(Dim::Px).unwrap_or(Dim::Auto),
        })
    }
}

/// Padding/margin shorthand: `16`, `[8, 16]`, `[t, r, b, l]`, or `"$md"` token.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct Edges {
    pub top: Length,
    pub right: Length,
    pub bottom: Length,
    pub left: Length,
}

/// A length that may reference a spacing/radius token.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Length {
    Px(f32),
    Token(String),
}

impl Default for Length {
    fn default() -> Self {
        Length::Px(0.0)
    }
}

impl Length {
    fn from_value(v: &serde_json::Value) -> Option<Length> {
        match v {
            serde_json::Value::String(s) if s.starts_with('$') => Some(Length::Token(s[1..].to_string())),
            _ => value_to_f32(v).map(Length::Px),
        }
    }
}

impl<'de> Deserialize<'de> for Length {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        Ok(Length::from_value(&v).unwrap_or_default())
    }
}

impl<'de> Deserialize<'de> for Edges {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        let l = |v: &serde_json::Value| Length::from_value(v).unwrap_or_default();
        Ok(match &v {
            serde_json::Value::Array(a) => match a.len() {
                1 => Edges { top: l(&a[0]), right: l(&a[0]), bottom: l(&a[0]), left: l(&a[0]) },
                2 => Edges { top: l(&a[0]), right: l(&a[1]), bottom: l(&a[0]), left: l(&a[1]) },
                3 => Edges { top: l(&a[0]), right: l(&a[1]), bottom: l(&a[2]), left: l(&a[1]) },
                n if n >= 4 => Edges { top: l(&a[0]), right: l(&a[1]), bottom: l(&a[2]), left: l(&a[3]) },
                _ => Edges::default(),
            },
            serde_json::Value::Object(o) => {
                let g = |k: &str| o.get(k).map(l).unwrap_or_default();
                let x = o.get("x").or(o.get("horizontal")).map(l);
                let y = o.get("y").or(o.get("vertical")).map(l);
                Edges {
                    top: o.get("top").map(l).or(y.clone()).unwrap_or_default(),
                    bottom: o.get("bottom").map(l).or(y).unwrap_or_default(),
                    left: o.get("left").map(l).or(x.clone()).unwrap_or_default(),
                    right: o.get("right").map(l).or(x).unwrap_or_else(|| g("right")),
                }
            }
            other => {
                let x = l(other);
                Edges { top: x.clone(), right: x.clone(), bottom: x.clone(), left: x }
            }
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Border {
    #[serde(deserialize_with = "de_opt_f32")]
    pub width: Option<f32>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Style {
    pub direction: Option<String>,
    pub width: Dim,
    pub height: Dim,
    #[serde(deserialize_with = "de_opt_f32")]
    pub min_width: Option<f32>,
    #[serde(deserialize_with = "de_opt_f32")]
    pub max_width: Option<f32>,
    #[serde(deserialize_with = "de_opt_f32")]
    pub min_height: Option<f32>,
    pub padding: Option<Edges>,
    pub margin: Option<Edges>,
    pub gap: Option<Length>,
    pub align: Option<String>,
    pub justify: Option<String>,
    pub wrap: Option<bool>,
    #[serde(deserialize_with = "de_opt_f32")]
    pub grow: Option<f32>,
    #[serde(alias = "background")]
    pub bg: Option<String>,
    /// Two-stop gradient `["#from", "#to"]` (vertical).
    pub gradient: Option<Vec<String>>,
    #[serde(alias = "color", alias = "foreground")]
    pub fg: Option<String>,
    pub radius: Option<Length>,
    pub border: Option<Border>,
    pub shadow: Option<String>,
    /// Typography token name (`"$body"` or `"body"`).
    pub font: Option<String>,
    #[serde(deserialize_with = "de_opt_f32")]
    pub size: Option<f32>,
    #[serde(deserialize_with = "de_opt_f32")]
    pub weight: Option<f32>,
    pub italic: Option<bool>,
    pub text_align: Option<String>,
    #[serde(deserialize_with = "de_opt_f32")]
    pub opacity: Option<f32>,
    pub overflow: Option<String>,
    /// Stack children: absolute offsets.
    #[serde(deserialize_with = "de_opt_f32")]
    pub x: Option<f32>,
    #[serde(deserialize_with = "de_opt_f32")]
    pub y: Option<f32>,
}

fn de_opt_f32<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f32>, D::Error> {
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(v.as_ref().and_then(|v| match v {
        // Named font weights.
        serde_json::Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "thin" => Some(100.0),
            "light" => Some(300.0),
            "regular" | "normal" => Some(400.0),
            "medium" => Some(500.0),
            "semibold" => Some(600.0),
            "bold" => Some(700.0),
            "extrabold" | "black" => Some(800.0),
            _ => value_to_f32(v),
        },
        _ => value_to_f32(v),
    }))
}

/// Parse a CSS-ish color: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb(…)`, `rgba(…)`,
/// `transparent`, `white`, `black`. Returns 0xRRGGBBAA.
pub fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim();
    match s.to_ascii_lowercase().as_str() {
        "transparent" | "none" => return Some(0x00000000),
        "white" => return Some(0xFFFFFFFF),
        "black" => return Some(0x000000FF),
        _ => {}
    }
    if let Some(hex) = s.strip_prefix('#') {
        let expand = |c: char| c.to_digit(16).map(|d| d * 17);
        let chars: Vec<char> = hex.chars().collect();
        return match chars.len() {
            3 | 4 => {
                let r = expand(chars[0])?;
                let g = expand(chars[1])?;
                let b = expand(chars[2])?;
                let a = if chars.len() == 4 { expand(chars[3])? } else { 255 };
                Some((r << 24) | (g << 16) | (b << 8) | a)
            }
            6 => u32::from_str_radix(hex, 16).ok().map(|v| (v << 8) | 0xFF),
            8 => u32::from_str_radix(hex, 16).ok(),
            _ => None,
        };
    }
    let lower = s.to_ascii_lowercase();
    if let Some(inner) = lower
        .strip_prefix("rgba(")
        .or_else(|| lower.strip_prefix("rgb("))
        .and_then(|r| r.strip_suffix(')'))
    {
        let parts: Vec<f32> = inner
            .split([',', ' ', '/'])
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.trim().trim_end_matches('%').parse().ok())
            .collect();
        if parts.len() >= 3 {
            let c = |v: f32| v.clamp(0.0, 255.0) as u32;
            let a = parts.get(3).map(|a| if *a <= 1.0 { a * 255.0 } else { *a }).unwrap_or(255.0);
            return Some((c(parts[0]) << 24) | (c(parts[1]) << 16) | (c(parts[2]) << 8) | c(a));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(parse_color("#fff"), Some(0xFFFFFFFF));
        assert_eq!(parse_color("#4F46E5"), Some(0x4F46E5FF));
        assert_eq!(parse_color("#00000033"), Some(0x00000033));
        assert_eq!(parse_color("rgba(255, 0, 0, 0.5)"), Some(0xFF00007F));
        assert_eq!(parse_color("nope"), None);
    }

    #[test]
    fn permissive_node() {
        let n: Node = serde_json::from_str(
            r##"{"type":"hstack","style":{"padding":[8,"16px"],"gap":"$sm","width":"fill","bg":"$primary","weight":"bold"},
                "children":[{"type":"label","text":"Hi"},{"type":"mystery"}]}"##,
        )
        .unwrap();
        assert_eq!(n.kind, NodeType::Row);
        assert_eq!(n.style.width, Dim::Fill);
        assert_eq!(n.style.weight, Some(700.0));
        assert_eq!(n.style.gap, Some(Length::Token("sm".into())));
        assert_eq!(n.children[0].kind, NodeType::Text);
        assert_eq!(n.children[1].kind, NodeType::Unknown);
        let p = n.style.padding.unwrap();
        assert_eq!(p.left, Length::Px(16.0));
    }

    #[test]
    fn ordered_tokens() {
        let t: Tokens = serde_json::from_str(r##"{"colors":{"z":"#000","a":"#fff"},"spacing":{"sm":"8px"}}"##).unwrap();
        assert_eq!(t.colors.0[0].0, "z");
        assert_eq!(t.spacing.get("sm").unwrap().0, 8.0);
    }
}
