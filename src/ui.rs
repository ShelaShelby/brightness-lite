//! Window look: dark theme, one card per monitor, toggle switch, flat button.

use crate::{
    ddc,
    state::{Cmd, Core},
};
use eframe::egui::{
    self, Align, Button, Color32, CornerRadius, Frame, Layout, Margin, RichText, Sense, Slider, SliderOrientation,
    Stroke, StrokeKind, Vec2,
};
use std::collections::HashMap;

pub const WINDOW_W: f32 = 360.0;
/// Text of the startup switch (tray menu and window).
pub const STARTUP_LABEL: &str = if cfg!(windows) { "Start with Windows" } else { "Start at login" };
pub const BG: Color32 = Color32::from_rgb(0x16, 0x18, 0x1c);
const PANEL: Color32 = Color32::from_rgb(0x1f, 0x22, 0x28);
const BORDER: Color32 = Color32::from_rgb(0x2c, 0x30, 0x38);
const RAIL: Color32 = Color32::from_rgb(0x34, 0x38, 0x42);
const ACCENT: Color32 = Color32::from_rgb(0x4c, 0x8d, 0xff);
const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe8, 0xec);
const MUTED: Color32 = Color32::from_rgb(0x8b, 0x91, 0x9c);
const WARN: Color32 = Color32::from_rgb(0xe8, 0xb3, 0x4a);
const RADIUS: u8 = 8;
const MARGIN: i8 = 14;

#[derive(Default)]
pub struct UiState {
    /// Slider values while the user is dragging (applied on release), by
    /// (monitor index, setting index). Only valid for `generation`.
    drag: HashMap<(usize, usize), f64>,
    /// The monitor list the drag map belongs to. When the list changes
    /// (plug/unplug), an index may point at another monitor, so the map is cleared.
    generation: u64,
    last_height: f32,
}

impl UiState {
    /// Forget any drag in progress (the monitor list may have changed).
    pub fn reset_drag(&mut self) {
        self.drag.clear();
    }

    pub fn dragging(&self) -> bool {
        !self.drag.is_empty()
    }
}

/// Only the one font the window uses: the monospace font (Hack) is never shown,
/// so it is not loaded.
fn lean_fonts() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.remove("Hack");
    for list in fonts.families.values_mut() {
        list.retain(|name| name != "Hack");
    }
    fonts
}

pub fn apply_theme(ctx: &egui::Context) {
    ctx.set_fonts(lean_fonts());
    ctx.set_theme(egui::Theme::Dark);
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG;
    v.faint_bg_color = PANEL;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(RADIUS);
    }
    v.widgets.inactive.bg_fill = RAIL;
    v.widgets.inactive.weak_bg_fill = PANEL;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x27, 0x2b, 0x33);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x3a, 0x3f, 0x4a));
    v.widgets.active.weak_bg_fill = Color32::from_rgb(0x2c, 0x30, 0x38);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 8.0);
        s.spacing.interact_size.y = 20.0;
    });
}

/// The app icon, 32x32 RGBA: a blue disc with a lighter centre, drawn in code.
pub fn icon_rgba() -> Vec<u8> {
    let n = 32usize;
    let mut rgba = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = (x as f32 + 0.5 - 16.0, y as f32 + 0.5 - 16.0);
            let d = (dx * dx + dy * dy).sqrt();
            let px = if d <= 13.0 {
                if d <= 6.0 {
                    [0xe6, 0xee, 0xff, 0xff]
                } else {
                    [0x4c, 0x8d, 0xff, 0xff]
                }
            } else {
                [0, 0, 0, 0]
            };
            rgba[(y * n + x) * 4..(y * n + x) * 4 + 4].copy_from_slice(&px);
        }
    }
    rgba
}

fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(RADIUS)
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(38.0, 20.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let r = rect.height() / 2.0;
    let fill = if *on { ACCENT } else { RAIL };
    ui.painter().rect(rect, r, fill, Stroke::new(1.0, BORDER), StrokeKind::Inside);
    let x = egui::lerp((rect.left() + r)..=(rect.right() - r), t);
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), r - 3.0, Color32::WHITE);
    resp
}

/// Small button with an up and a down arrow, drawn with the painter (no font glyphs).
/// Its id depends only on its position, so it is the same from frame to frame.
fn swap_button(ui: &mut egui::Ui, pos: usize, enabled: bool) -> egui::Response {
    let size = Vec2::new(40.0, 18.0);
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), size.y), Sense::hover());
    let rect = egui::Rect::from_center_size(row.center(), size);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let resp = ui.interact(rect, ui.make_persistent_id(("swap", pos)), sense);
    let resp = if enabled { resp.on_hover_text("Swap these two monitors") } else { resp };
    let fill = if resp.hovered() && enabled { Color32::from_rgb(0x27, 0x2b, 0x33) } else { PANEL };
    let color = if !enabled {
        RAIL
    } else if resp.hovered() {
        TEXT
    } else {
        MUTED
    };
    let p = ui.painter();
    p.rect(rect, RADIUS, fill, Stroke::new(1.0, BORDER), StrokeKind::Inside);
    let stroke = Stroke::new(1.5, color);
    let (cy, h, head) = (rect.center().y, 5.0, 3.0);
    for (dx, up) in [(-5.0, true), (5.0, false)] {
        let x = rect.center().x + dx;
        let (tip, tail) = if up { (cy - h, cy + h) } else { (cy + h, cy - h) };
        let back = if up { tip + head } else { tip - head };
        p.line_segment([egui::pos2(x, tail), egui::pos2(x, tip)], stroke);
        p.line_segment([egui::pos2(x - head, back), egui::pos2(x, tip)], stroke);
        p.line_segment([egui::pos2(x + head, back), egui::pos2(x, tip)], stroke);
    }
    resp
}

fn setting_row(ui: &mut egui::Ui, core: &mut Core, st_ui: &mut UiState, i: usize, st: ddc::Setting) {
    let idx = (i, st.index());
    let cur = core.cache.views[i].value(st);
    let mut v = st_ui.drag.get(&idx).copied().unwrap_or(cur.unwrap_or(0) as f64);
    let caption = if st == ddc::Setting::Brightness { "Brightness" } else { "Contrast" };
    let number = if cur.is_some() { format!("{}", v.round() as u32) } else { "--".to_string() };
    ui.horizontal(|ui| {
        ui.label(RichText::new(caption).color(MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(number).strong());
        });
    });
    ui.spacing_mut().slider_width = ui.available_width();
    let resp = ui.add_enabled(
        cur.is_some(),
        Slider::new(&mut v, 0.0..=100.0)
            .orientation(SliderOrientation::Horizontal)
            .show_value(false)
            .trailing_fill(true)
            .step_by(1.0),
    );
    if resp.dragged() {
        st_ui.drag.insert(idx, v);
    }
    // Apply when the user lets go (or uses the keyboard), not on every move.
    if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
        st_ui.drag.remove(&idx);
        let target = ddc::clamp(v.round() as i64);
        if core.cache.views[i].value(st) != Some(target) {
            core.write(i, st, target);
        }
    }
}

/// Draw the whole window and keep its height fitted to the content.
pub fn draw(ui: &mut egui::Ui, core: &mut Core, st_ui: &mut UiState) {
    let ctx = ui.ctx().clone();
    let outer = Frame::new().fill(BG).inner_margin(Margin::same(MARGIN)).show(ui, |ui| {
        ui.set_width(WINDOW_W - 2.0 * MARGIN as f32);
        if st_ui.generation != core.cache.generation {
            st_ui.generation = core.cache.generation;
            st_ui.reset_drag();
        }
        // The whole frame is drawn in one order. A swap click is applied after
        // it, and the next frame (asked for at once) shows the new order.
        let shown = core.display_order();
        let mut swap_at = None;
        for (pos, &i) in shown.iter().enumerate() {
            if pos > 0 {
                // Between two neighbouring cards: swap them on screen.
                if swap_button(ui, pos, core.can_swap(pos - 1)).clicked() {
                    swap_at = Some(pos - 1);
                }
            }
            // The id follows the monitor (its key), not the position, so widget state stays with its card.
            let id = core.cache.views[i].key.clone().unwrap_or_else(|| format!("#{i}"));
            ui.push_id(("monitor", id), |ui| {
                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(core.cache.views[i].label()).size(15.0).strong());
                        #[cfg(windows)]
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let m = &core.cache.views[i];
                            let target = m.display_number.zip(m.rect);
                            let identify = Button::new(RichText::new("Identify").color(MUTED).size(12.0))
                                .fill(PANEL)
                                .stroke(Stroke::new(1.0, BORDER))
                                .corner_radius(RADIUS);
                            if ui.add_enabled(target.is_some(), identify).clicked() {
                                if let Some((n, rect)) = target {
                                    crate::identify::show(n.to_string(), rect);
                                }
                            }
                        });
                    });
                    if let Some(name) = core.cache.views[i].name.clone() {
                        ui.add(egui::Label::new(RichText::new(name).color(MUTED).size(12.0)).truncate());
                    }
                    for st in ddc::Setting::ALL {
                        // A setting the monitor does not have is not shown at all.
                        if core.cache.views[i].supports(st) {
                            setting_row(ui, core, st_ui, i, st);
                        }
                    }
                })
            });
        }

        if let Some(pos) = swap_at {
            core.swap(pos);
            ctx.request_repaint();
        }

        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(STARTUP_LABEL);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut on = core.startup;
                    if toggle(ui, &mut on).changed() {
                        core.set_startup(on);
                    }
                });
            });
        });

        let lines = core.status_lines();
        if !lines.is_empty() {
            for l in lines {
                ui.label(RichText::new(l).color(WARN));
            }
        }

        let reset = Button::new("Reset to saved values")
            .fill(PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(RADIUS)
            .min_size(Vec2::new(ui.available_width(), 32.0));
        if ui.add(reset).clicked() {
            core.send(Cmd::Reset);
        }
    });

    let h = outer.response.rect.height();
    if (h - st_ui.last_height).abs() > 0.5 {
        st_ui.last_height = h;
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(WINDOW_W, h)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lean_fonts_keep_a_font_for_every_family() {
        let f = lean_fonts();
        assert!(!f.font_data.contains_key("Hack"));
        for list in f.families.values() {
            assert!(list.iter().any(|n| n == "Ubuntu-Light"));
            assert!(list.iter().all(|n| f.font_data.contains_key(n)));
        }
    }
}
