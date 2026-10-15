//! Window look: dark theme, one card per monitor, toggle switch, flat button.

use crate::{ddc, state::{Cmd, Core}};
use eframe::egui::{
    self, Align, Button, Color32, CornerRadius, Frame, Layout, Margin, RichText, Sense, Slider,
    SliderOrientation, Stroke, StrokeKind, Vec2,
};
use std::collections::HashMap;

pub const WINDOW_W: f32 = 360.0;
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
    /// Slider values while the user is dragging (applied on release).
    drag: HashMap<usize, f64>,
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

pub fn apply_theme(ctx: &egui::Context) {
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

fn setting_row(ui: &mut egui::Ui, core: &mut Core, st_ui: &mut UiState, i: usize, st: ddc::Setting) {
    let idx = i * 2 + st.index();
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
        for i in 0..core.cache.views.len() {
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(core.cache.views[i].label()).size(15.0).strong());
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
                for st in ddc::Setting::ALL {
                    setting_row(ui, core, st_ui, i, st);
                }
            });
        }

        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Start with Windows");
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
