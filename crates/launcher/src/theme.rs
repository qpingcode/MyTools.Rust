//! Applies the optional `theme` object of config.json to the shared Slint `Theme` global.
//! Keys match the property names in `ui/theme.slint`; colors are `#rrggbb` / `#rrggbbaa`,
//! sizes are logical pixels. Unknown keys and invalid values are reported, not fatal.
use crate::Theme;
use serde_json::Value;
use std::collections::HashMap;

pub type Overrides = HashMap<String, Value>;

static CONFIGURED: std::sync::OnceLock<Overrides> = std::sync::OnceLock::new();

/// Remembers the startup overrides so windows created later share the same look.
pub fn configure(overrides: Overrides) -> Vec<String> {
    let _ = CONFIGURED.set(overrides);
    let mut errors = Vec::new();
    for (key, value) in CONFIGURED.get().into_iter().flatten() {
        if color_setter(key).is_none() && length_setter(key).is_none() {
            errors.push(format!("theme.{key}: unknown key"));
        } else if color_setter(key).is_some() && parse_color(value).is_err() {
            errors.push(format!("theme.{key}: invalid color"));
        }
    }
    errors.sort();
    errors
}

/// Applies the configured overrides to one component's `Theme` global.
pub fn apply_configured(theme: &Theme<'_>) {
    if let Some(overrides) = CONFIGURED.get() {
        let _ = apply(theme, overrides);
    }
}

fn apply(theme: &Theme<'_>, overrides: &Overrides) -> Vec<String> {
    let mut errors = Vec::new();
    let mut keys: Vec<_> = overrides.keys().collect();
    keys.sort();
    for key in keys {
        if let Err(error) = apply_one(theme, key, &overrides[key]) {
            errors.push(format!("theme.{key}: {error}"));
        }
    }
    errors
}

fn apply_one(theme: &Theme<'_>, key: &str, value: &Value) -> Result<(), String> {
    if let Some(setter) = color_setter(key) {
        setter(theme, parse_color(value)?);
        return Ok(());
    }
    if let Some(setter) = length_setter(key) {
        let size = value
            .as_f64()
            .filter(|size| (0.0..=400.0).contains(size))
            .ok_or("expected a number between 0 and 400")?;
        setter(theme, size as f32);
        return Ok(());
    }
    Err("unknown key".into())
}

type ColorSetter = fn(&Theme<'_>, slint::Color);
type LengthSetter = fn(&Theme<'_>, f32);

fn color_setter(key: &str) -> Option<ColorSetter> {
    Some(match key {
        "background" => |t, v| t.set_background(v),
        "panel" => |t, v| t.set_panel(v),
        "preview" => |t, v| t.set_preview(v),
        "popup" => |t, v| t.set_popup(v),
        "border" => |t, v| t.set_border(v),
        "control-border" => |t, v| t.set_control_border(v),
        "control-border-disabled" => |t, v| t.set_control_border_disabled(v),
        "selected" => |t, v| t.set_selected(v),
        "hover" => |t, v| t.set_hover(v),
        "popup-selected" => |t, v| t.set_popup_selected(v),
        "popup-hover" => |t, v| t.set_popup_hover(v),
        "icon-background" => |t, v| t.set_icon_background(v),
        "foreground" => |t, v| t.set_foreground(v),
        "muted" => |t, v| t.set_muted(v),
        "placeholder" => |t, v| t.set_placeholder(v),
        "disabled" => |t, v| t.set_disabled(v),
        "accent" => |t, v| t.set_accent(v),
        "selection" => |t, v| t.set_selection(v),
        "primary" => |t, v| t.set_primary(v),
        "primary-hover" => |t, v| t.set_primary_hover(v),
        "button-hover" => |t, v| t.set_button_hover(v),
        "success" => |t, v| t.set_success(v),
        "warning" => |t, v| t.set_warning(v),
        "pending" => |t, v| t.set_pending(v),
        "danger" => |t, v| t.set_danger(v),
        _ => return None,
    })
}

fn length_setter(key: &str) -> Option<LengthSetter> {
    Some(match key {
        "font-search" => |t, v| t.set_font_search(v),
        "font-heading" => |t, v| t.set_font_heading(v),
        "font-title" => |t, v| t.set_font_title(v),
        "font-body" => |t, v| t.set_font_body(v),
        "font-small" => |t, v| t.set_font_small(v),
        "font-caption" => |t, v| t.set_font_caption(v),
        "search-height" => |t, v| t.set_search_height(v),
        "status-height" => |t, v| t.set_status_height(v),
        "row-height" => |t, v| t.set_row_height(v),
        "row-spacing" => |t, v| t.set_row_spacing(v),
        "icon-box" => |t, v| t.set_icon_box(v),
        "icon-size" => |t, v| t.set_icon_size(v),
        "action-row-height" => |t, v| t.set_action_row_height(v),
        "button-height" => |t, v| t.set_button_height(v),
        "radius" => |t, v| t.set_radius(v),
        _ => return None,
    })
}

fn parse_color(value: &Value) -> Result<slint::Color, String> {
    let text = value.as_str().ok_or("expected a color string")?;
    let hex = text.strip_prefix('#').ok_or("color must start with #")?;
    let channel = |index: usize| {
        u8::from_str_radix(hex.get(index..index + 2).unwrap_or("zz"), 16)
            .map_err(|_| format!("invalid color {text}"))
    };
    match hex.len() {
        6 => Ok(slint::Color::from_rgb_u8(
            channel(0)?,
            channel(2)?,
            channel(4)?,
        )),
        8 => Ok(slint::Color::from_argb_u8(
            channel(6)?,
            channel(0)?,
            channel(2)?,
            channel(4)?,
        )),
        _ => Err(format!("invalid color {text}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rgb_and_rgba_colors() {
        let color = parse_color(&"#102030".into()).unwrap();
        assert_eq!(
            (color.red(), color.green(), color.blue(), color.alpha()),
            (0x10, 0x20, 0x30, 0xff)
        );
        let color = parse_color(&"#10203080".into()).unwrap();
        assert_eq!(color.alpha(), 0x80);
        assert!(parse_color(&"102030".into()).is_err());
        assert!(parse_color(&"#12345".into()).is_err());
        assert!(parse_color(&"#gg0000".into()).is_err());
        assert!(parse_color(&1.into()).is_err());
    }

    #[test]
    fn every_documented_key_has_a_setter() {
        for key in ["background", "accent", "icon-background", "warning"] {
            assert!(color_setter(key).is_some(), "{key}");
        }
        for key in ["font-title", "font-search", "row-height", "radius"] {
            assert!(length_setter(key).is_some(), "{key}");
        }
        assert!(color_setter("font-title").is_none());
        assert!(length_setter("missing").is_none());
    }
}
