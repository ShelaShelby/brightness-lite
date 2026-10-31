//! Window look: one card per monitor, toggle switch, flat button, and the
//! Theme panel (palette button at the top). All colours come from `theme`.

use crate::{
    ddc,
    state::{Cmd, Core},
    theme::{self, Accent, Appearance, Rgb, Theme},
};
use eframe::egui::{
    self, Align, Align2, Button, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText, Role, Sense, Slider,
    SliderOrientation, Stroke, StrokeKind, Vec2, WidgetInfo,
};
use std::collections::HashMap;

pub const WINDOW_W: f32 = 360.0;
/// Text of the startup switch (tray menu and window).
pub const STARTUP_LABEL: &str = if cfg!(windows) { "Start with Windows" } else { "Start at login" };
const RADIUS: u8 = 8;
const MARGIN: i8 = 14;
/// Gap between the window and the bottom of the work area, in pixels (as when it opens).
pub const EDGE: i32 = 8;

fn c(rgb: Rgb) -> Color32 {
    Color32::from_rgb(rgb.0, rgb.1, rgb.2)
}

/// The theme's tokens as egui colours.
#[derive(Clone, Copy)]
struct Pal {
    bg: Color32,
    card: Color32,
    border: Color32,
    border_hover: Color32,
    track: Color32,
    hover: Color32,
    pressed: Color32,
    text: Color32,
    muted: Color32,
    warn: Color32,
    accent: Color32,
    /// Selected segment: a light tint of the accent on the card.
    accent_soft: Color32,
}

impl Pal {
    fn of(t: Theme) -> Pal {
        let p = t.palette();
        Pal {
            bg: c(p.bg),
            card: c(p.card),
            border: c(p.border),
            border_hover: c(p.border_hover),
            track: c(p.track),
            hover: c(p.hover),
            pressed: c(p.pressed),
            text: c(p.text),
            muted: c(p.muted),
            warn: c(p.warn),
            accent: c(p.accent),
            accent_soft: c(theme::mix(p.card, p.accent, 0.18)),
        }
    }
}

/// Where the last frame drew the controls (screen points), for the tests and
/// the Windows --theme-test, which click them like a user would.
#[derive(Clone, Copy)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Hits {
    pub palette: egui::Rect,
    pub appearance: [(egui::Rect, egui::Id); 3],
    pub accent: [(egui::Rect, egui::Id); 7],
    /// The first slider, if any.
    pub slider: Option<egui::Rect>,
    pub reset: egui::Rect,
}

impl Default for Hits {
    fn default() -> Self {
        let none = (egui::Rect::NOTHING, egui::Id::NULL);
        Hits {
            palette: egui::Rect::NOTHING,
            appearance: [none; 3],
            accent: [none; 7],
            slider: None,
            reset: egui::Rect::NOTHING,
        }
    }
}

pub struct UiState {
    /// Slider values while the user is dragging (applied on release), by
    /// (monitor index, setting index). Only valid for `generation`.
    drag: HashMap<(usize, usize), f64>,
    /// The monitor list the drag map belongs to. When the list changes
    /// (plug/unplug), an index may point at another monitor, so the map is cleared.
    generation: u64,
    last_height: f32,
    theme: theme::Store,
    pal: Pal,
    /// The Theme panel is open. Never saved: it starts closed.
    expanded: bool,
    /// Light title bar asked for (None: nothing asked yet).
    light_title: Option<bool>,
    pub hits: Hits,
}

impl Default for UiState {
    fn default() -> Self {
        UiState::new(theme::Store::default())
    }
}

impl UiState {
    pub fn new(theme: theme::Store) -> Self {
        let pal = Pal::of(theme.theme);
        UiState {
            drag: HashMap::new(),
            generation: 0,
            last_height: 0.0,
            theme,
            pal,
            expanded: false,
            light_title: None,
            hits: Hits::default(),
        }
    }

    /// Forget any drag in progress (the monitor list may have changed).
    pub fn reset_drag(&mut self) {
        self.drag.clear();
    }

    pub fn dragging(&self) -> bool {
        !self.drag.is_empty()
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn theme(&self) -> Theme {
        self.theme.theme
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn expanded(&self) -> bool {
        self.expanded
    }

    /// Open or close the Theme panel (the palette button).
    pub fn toggle_panel(&mut self) {
        self.expanded = !self.expanded;
    }

    /// Put the current theme on `ctx` (at start-up, before the first frame).
    pub fn apply(&self, ctx: &egui::Context) {
        ctx.set_visuals(visuals(self.theme.theme));
    }

    /// The window is being shown: the panel starts closed, and theme.json is
    /// read again (the other app may have changed it). Called inside a frame.
    pub fn on_show(&mut self, ctx: &egui::Context) {
        self.expanded = false;
        if self.theme.reload() {
            self.restyle(ctx);
        }
    }

    /// The user picked a theme: use it at once and save it.
    pub fn pick(&mut self, ctx: &egui::Context, t: Theme) {
        if self.theme.set(t) {
            self.restyle(ctx);
        }
    }

    /// New colours. This frame was partly drawn with the old ones: egui throws
    /// it away and draws it again at once, so no mixed frame is ever shown.
    fn restyle(&mut self, ctx: &egui::Context) {
        self.pal = Pal::of(self.theme.theme);
        self.apply(ctx);
        ctx.request_discard("theme changed");
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

/// Fonts and spacing, once at start-up. The colours come from `UiState::apply`.
pub fn apply_theme(ctx: &egui::Context) {
    ctx.set_fonts(lean_fonts());
    // egui's own theme stays "dark" whatever Windows uses: the colours below
    // are the only ones, so switching Windows between dark and light changes nothing.
    ctx.set_theme(egui::Theme::Dark);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 8.0);
        s.spacing.interact_size.y = 20.0;
    });
}

/// egui's colours for a theme. Dark gives exactly the look from before themes.
fn visuals(t: Theme) -> egui::Visuals {
    let p = Pal::of(t);
    let mut v = if t.appearance.is_light() { egui::Visuals::light() } else { egui::Visuals::dark() };
    v.panel_fill = p.bg;
    v.window_fill = p.card;
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.card;
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(RADIUS);
    }
    v.widgets.inactive.bg_fill = p.track;
    v.widgets.inactive.weak_bg_fill = p.card;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.hovered.weak_bg_fill = p.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.border_hover);
    v.widgets.active.weak_bg_fill = p.pressed;
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v
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

fn card<R>(ui: &mut egui::Ui, p: &Pal, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(RADIUS)
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn toggle(ui: &mut egui::Ui, p: &Pal, on: &mut bool) -> egui::Response {
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(38.0, 20.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let r = rect.height() / 2.0;
    let fill = if *on { p.accent } else { p.track };
    ui.painter().rect(rect, r, fill, Stroke::new(1.0, p.border), StrokeKind::Inside);
    let x = egui::lerp((rect.left() + r)..=(rect.right() - r), t);
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), r - 3.0, Color32::WHITE);
    resp
}

/// Small button with an up and a down arrow, drawn with the painter (no font glyphs).
/// Its id depends only on its position, so it is the same from frame to frame.
fn swap_button(ui: &mut egui::Ui, p: &Pal, pos: usize, enabled: bool) -> egui::Response {
    let size = Vec2::new(40.0, 18.0);
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), size.y), Sense::hover());
    let rect = egui::Rect::from_center_size(row.center(), size);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let resp = ui.interact(rect, ui.make_persistent_id(("swap", pos)), sense);
    let resp = if enabled { resp.on_hover_text("Swap these two monitors") } else { resp };
    let fill = if resp.hovered() && enabled { p.hover } else { p.card };
    let color = if !enabled {
        p.track
    } else if resp.hovered() {
        p.text
    } else {
        p.muted
    };
    let border = p.border;
    let p = ui.painter();
    p.rect(rect, RADIUS, fill, Stroke::new(1.0, border), StrokeKind::Inside);
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
        ui.label(RichText::new(caption).color(st_ui.pal.muted));
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
    st_ui.hits.slider.get_or_insert(resp.rect);
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

/// Focus ring for keyboard use (Tab), drawn around `rect`.
fn focus_ring(ui: &egui::Ui, p: &Pal, resp: &egui::Response, rect: egui::Rect, radius: f32) {
    if resp.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), radius + 2.0, Stroke::new(1.0, p.text), StrokeKind::Outside);
    }
}

/// The palette button that opens the Theme panel, drawn with the painter (no
/// image or icon font). Accent coloured while the panel is open.
fn palette_button(ui: &mut egui::Ui, p: &Pal, open: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(32.0, 22.0), Sense::click());
    resp.widget_info(|| WidgetInfo::selected(Role::Button, true, open, "Theme"));
    let resp = resp.on_hover_text(if open { "Close theme" } else { "Theme" });
    let fill = if resp.is_pointer_button_down_on() {
        p.pressed
    } else if open || resp.hovered() {
        p.hover
    } else {
        p.card
    };
    let (stroke, color) = if open {
        (p.accent, p.accent)
    } else if resp.hovered() {
        (p.border_hover, p.text)
    } else {
        (p.border, p.muted)
    };
    let painter = ui.painter();
    painter.rect(rect, RADIUS, fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
    focus_ring(ui, p, &resp, rect, RADIUS as f32);
    // A painter's palette: a round board, three paint dots and a thumb hole.
    let m = rect.center();
    painter.circle_stroke(m, 7.0, Stroke::new(1.5, color));
    for d in [Vec2::new(-3.2, -2.2), Vec2::new(0.0, -4.0), Vec2::new(3.2, -2.2), Vec2::new(-3.4, 1.6)] {
        painter.circle_filled(m + d, 1.3, color);
    }
    painter.circle_filled(m + Vec2::new(2.6, 3.0), 1.9, color);
    resp
}

/// Light | Dark | AMOLED. The selected segment is tinted with the accent.
fn appearance_control(
    ui: &mut egui::Ui,
    p: &Pal,
    current: Appearance,
    hits: &mut [(egui::Rect, egui::Id); 3],
) -> Option<Appearance> {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::hover());
    ui.painter().rect(rect, RADIUS, p.bg, Stroke::new(1.0, p.border), StrokeKind::Inside);
    let w = rect.width() / Appearance::ALL.len() as f32;
    let mut picked = None;
    for (i, a) in Appearance::ALL.into_iter().enumerate() {
        let seg =
            egui::Rect::from_min_size(rect.min + Vec2::new(w * i as f32, 0.0), Vec2::new(w, rect.height())).shrink(3.0);
        let resp = ui.interact(seg, ui.make_persistent_id(("appearance", i)), Sense::click());
        hits[i] = (seg, resp.id);
        let selected = a == current;
        resp.widget_info(|| WidgetInfo::selected(Role::RadioButton, true, selected, a.label()));
        let r = RADIUS as f32 - 2.0;
        if selected {
            ui.painter().rect(seg, r, p.accent_soft, Stroke::new(1.5, p.accent), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(seg, r, if resp.is_pointer_button_down_on() { p.pressed } else { p.hover });
        }
        focus_ring(ui, p, &resp, seg, r);
        let color = if selected || resp.hovered() { p.text } else { p.muted };
        ui.painter().text(seg.center(), Align2::CENTER_CENTER, a.label(), FontId::proportional(13.0), color);
        if resp.clicked() && !selected {
            picked = Some(a);
        }
    }
    picked
}

/// Seven round swatches; the selected one has a ring in its own colour.
fn accent_swatches(
    ui: &mut egui::Ui,
    p: &Pal,
    current: Accent,
    hits: &mut [(egui::Rect, egui::Id); 7],
) -> Option<Accent> {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::hover());
    let step = rect.width() / Accent::ALL.len() as f32;
    let mut picked = None;
    for (i, a) in Accent::ALL.into_iter().enumerate() {
        let m = egui::pos2(rect.left() + step * (i as f32 + 0.5), rect.center().y);
        let hit = egui::Rect::from_center_size(m, Vec2::splat(30.0));
        let resp = ui.interact(hit, ui.make_persistent_id(("accent", i)), Sense::click());
        hits[i] = (hit, resp.id);
        let selected = a == current;
        resp.widget_info(|| WidgetInfo::selected(Role::RadioButton, true, selected, a.label()));
        let resp = resp.on_hover_text(a.label());
        let color = c(a.rgb());
        let painter = ui.painter();
        painter.circle_filled(m, 10.5, color);
        if selected {
            painter.circle_stroke(m, 14.0, Stroke::new(2.0, p.accent));
        } else if resp.hovered() {
            painter.circle_stroke(m, 14.0, Stroke::new(1.5, p.border_hover));
        }
        if resp.has_focus() {
            painter.circle_stroke(m, 16.5, Stroke::new(1.0, p.text));
        }
        if resp.clicked() && !selected {
            picked = Some(a);
        }
    }
    picked
}

/// The Theme panel: one card with Appearance and Colour.
fn theme_card(ui: &mut egui::Ui, p: &Pal, current: Theme, hits: &mut Hits) -> Option<Theme> {
    card(ui, p, |ui| {
        let mut t = current;
        ui.label(RichText::new("Appearance").color(p.muted));
        if let Some(a) = appearance_control(ui, p, current.appearance, &mut hits.appearance) {
            t.appearance = a;
        }
        ui.label(RichText::new("Colour").color(p.muted));
        if let Some(a) = accent_swatches(ui, p, current.accent, &mut hits.accent) {
            t.accent = a;
        }
        (t != current).then_some(t)
    })
}

/// New top edge, in pixels, for a window whose height changes from
/// `top..bottom` to `new_h`, in a work area from `wa_top` to `wa_bottom`. It
/// grows downward; one sitting at the bottom of the work area (where it opens)
/// keeps its bottom edge, so it grows upward, and none goes below the bottom
/// (`EDGE` above it) or above the top.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn fit_top(top: i32, bottom: i32, new_h: i32, wa_top: i32, wa_bottom: i32) -> i32 {
    let (top, bottom, new_h) = (i64::from(top), i64::from(bottom), i64::from(new_h.max(0)));
    let limit = i64::from(wa_bottom) - i64::from(EDGE);
    let mut t = if bottom >= limit - 1 { bottom - new_h } else { top };
    if t + new_h > limit {
        t = limit - new_h;
    }
    t.max(i64::from(wa_top)).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// Draw the whole window. Returns the content height in points when it
/// changed: the caller resizes the window (and keeps it on screen).
pub fn draw(ui: &mut egui::Ui, core: &mut Core, st_ui: &mut UiState) -> Option<f32> {
    let ctx = ui.ctx().clone();
    let p = st_ui.pal;
    // The title bar follows the appearance (Windows draws it).
    let light = st_ui.theme.theme.appearance.is_light();
    if st_ui.light_title != Some(light) {
        st_ui.light_title = Some(light);
        let t = if light { egui::SystemTheme::Light } else { egui::SystemTheme::Dark };
        ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(t));
    }
    let mut picked = None;
    let outer = Frame::new().fill(p.bg).inner_margin(Margin::same(MARGIN)).show(ui, |ui| {
        ui.set_width(WINDOW_W - 2.0 * MARGIN as f32);
        if st_ui.generation != core.cache.generation {
            st_ui.generation = core.cache.generation;
            st_ui.reset_drag();
        }
        // Top row: the palette button, at the right (under the window's X).
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            let b = palette_button(ui, &p, st_ui.expanded);
            st_ui.hits.palette = b.rect;
            if b.clicked() {
                st_ui.toggle_panel();
            }
        });
        // The whole frame is drawn in one order. A swap click is applied after
        // it, and the next frame (asked for at once) shows the new order.
        let shown = core.display_order();
        let mut swap_at = None;
        st_ui.hits.slider = None;
        for (pos, &i) in shown.iter().enumerate() {
            if pos > 0 {
                // Between two neighbouring cards: swap them on screen.
                if swap_button(ui, &p, pos, core.can_swap(pos - 1)).clicked() {
                    swap_at = Some(pos - 1);
                }
            }
            // The id follows the monitor (its key), not the position, so widget state stays with its card.
            let id = core.cache.views[i].key.clone().unwrap_or_else(|| format!("#{i}"));
            ui.push_id(("monitor", id), |ui| {
                card(ui, &p, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(core.cache.views[i].label()).size(15.0).strong());
                        #[cfg(windows)]
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let m = &core.cache.views[i];
                            let target = m.display_number.zip(m.rect);
                            let identify = Button::new(RichText::new("Identify").color(p.muted).size(12.0))
                                .fill(p.card)
                                .stroke(Stroke::new(1.0, p.border))
                                .corner_radius(RADIUS);
                            if ui.add_enabled(target.is_some(), identify).clicked() {
                                if let Some((n, rect)) = target {
                                    crate::identify::show(n.to_string(), rect);
                                }
                            }
                        });
                    });
                    if let Some(name) = core.cache.views[i].name.clone() {
                        ui.add(egui::Label::new(RichText::new(name).color(p.muted).size(12.0)).truncate());
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

        card(ui, &p, |ui| {
            ui.horizontal(|ui| {
                ui.label(STARTUP_LABEL);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut on = core.startup;
                    if toggle(ui, &p, &mut on).changed() {
                        core.set_startup(on);
                    }
                });
            });
        });

        let lines = core.status_lines();
        if !lines.is_empty() {
            for l in lines {
                ui.label(RichText::new(l).color(p.warn));
            }
        }

        // Puts the saved brightness and contrast back; the theme is not touched.
        let reset = Button::new("Reset to saved values")
            .fill(p.card)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(RADIUS)
            .min_size(Vec2::new(ui.available_width(), 32.0));
        let r = ui.add(reset);
        st_ui.hits.reset = r.rect;
        if r.clicked() {
            core.send(Cmd::Reset);
        }

        if st_ui.expanded {
            let mut hits = st_ui.hits;
            picked = theme_card(ui, &p, st_ui.theme.theme, &mut hits);
            st_ui.hits = hits;
        }
    });
    if let Some(t) = picked {
        st_ui.pick(&ctx, t);
    }

    let h = outer.response.rect.height();
    if (h - st_ui.last_height).abs() > 0.5 {
        st_ui.last_height = h;
        return Some(h);
    }
    None
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

    #[test]
    fn panel_grows_down_or_up_and_stays_in_the_work_area() {
        // Work area 0..1040. Where it opens: bottom edge 8 px above the work area's bottom.
        let (top, bottom) = (1032 - 400, 1032);
        // Opening the panel: it keeps its bottom edge and grows upward...
        assert_eq!(fit_top(top, bottom, 550, 0, 1040), 1032 - 550);
        // ...and closing it puts it back where it was.
        assert_eq!(fit_top(1032 - 550, 1032, 400, 0, 1040), top);
        // In the middle of the screen it grows downward.
        assert_eq!(fit_top(100, 500, 550, 0, 1040), 100);
        // Near the bottom (but not at it) it grows down until the edge, then moves up.
        assert_eq!(fit_top(600, 1000, 550, 0, 1040), 1032 - 550);
        // Taller than the work area: the top stays visible.
        assert_eq!(fit_top(500, 900, 5000, 0, 1040), 0);
        // Above the work area (dragged up): moved down into it.
        assert_eq!(fit_top(-300, 100, 400, 0, 1040), 0);
        // A monitor above and left of the main one, and a taskbar at the top.
        assert_eq!(fit_top(-1080 + 300, -1080 + 700, 550, -1080, -40), -1080 + 300);
        assert_eq!(fit_top(-48 - 400, -48, 550, -1080, -40), -48 - 550);
        assert_eq!(fit_top(100, 500, 550, 48, 1080), 100);
    }

    #[test]
    fn fit_top_never_overflows() {
        let v = [i32::MIN, -1, 0, 1, 1040, i32::MAX];
        for &a in &v {
            for &b in &v {
                for &h in &v {
                    let t = fit_top(a, b, h, a.min(b), a.max(b));
                    assert!(t >= a.min(b));
                }
            }
        }
    }

    #[test]
    fn every_theme_gives_its_visuals() {
        for appearance in Appearance::ALL {
            for accent in Accent::ALL {
                let t = Theme { appearance, accent };
                let p = t.palette();
                let v = visuals(t);
                assert_eq!(v.panel_fill, c(p.bg));
                assert_eq!(v.window_fill, c(p.card));
                assert_eq!(v.selection.bg_fill, c(accent.rgb()), "slider fill follows the accent");
                assert_eq!(v.override_text_color, Some(c(p.text)), "text stays neutral");
                assert_eq!(v.widgets.inactive.bg_fill, c(p.track));
                assert_eq!(v.widgets.inactive.weak_bg_fill, c(p.card));
                assert_eq!(v.widgets.hovered.weak_bg_fill, c(p.hover));
                assert_eq!(v.widgets.active.weak_bg_fill, c(p.pressed));
                assert_eq!(v.dark_mode, !appearance.is_light());
                // The accent is nowhere else in egui's colours.
                for w in [&v.widgets.inactive, &v.widgets.hovered, &v.widgets.active, &v.widgets.noninteractive] {
                    assert_ne!(w.bg_fill, c(accent.rgb()));
                    assert_ne!(w.weak_bg_fill, c(accent.rgb()));
                }
            }
        }
    }

    /// Dark + Blue is exactly the look from before themes.
    #[test]
    fn dark_blue_is_the_old_look() {
        let v = visuals(Theme::default());
        assert_eq!(v.panel_fill, Color32::from_rgb(0x16, 0x18, 0x1c));
        assert_eq!(v.window_fill, Color32::from_rgb(0x1f, 0x22, 0x28));
        assert_eq!(v.selection.bg_fill, Color32::from_rgb(0x4c, 0x8d, 0xff));
        assert_eq!(v.widgets.inactive.bg_fill, Color32::from_rgb(0x34, 0x38, 0x42));
        assert_eq!(v.widgets.hovered.weak_bg_fill, Color32::from_rgb(0x27, 0x2b, 0x33));
        assert_eq!(v.widgets.hovered.bg_stroke.color, Color32::from_rgb(0x3a, 0x3f, 0x4a));
        assert_eq!(v.override_text_color, Some(Color32::from_rgb(0xe6, 0xe8, 0xec)));
    }

    /// Picking, reloading and showing through UiState, as the window does.
    #[test]
    fn ui_state_theme_flow() {
        let d = std::env::temp_dir().join(format!("bl-ui-theme-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join(theme::FILE_NAME);
        let ctx = egui::Context::default();
        let mut a = UiState::new(theme::Store::open(Some(path.clone())));
        let mut b = UiState::new(theme::Store::open(Some(path.clone())));
        assert!(!a.expanded());
        a.toggle_panel();
        assert!(a.expanded());
        let t = Theme { appearance: Appearance::Amoled, accent: Accent::Green };
        a.pick(&ctx, t);
        assert_eq!(a.theme(), t);
        assert_eq!(theme::read(Some(&path)), Some(t));
        // B shows the new theme when its window is shown, and its panel starts closed.
        b.toggle_panel();
        assert_eq!(b.theme(), Theme::default());
        b.on_show(&ctx);
        assert_eq!(b.theme(), t);
        assert!(!b.expanded());
        assert_eq!(b.pal.bg, Color32::BLACK);
        assert_eq!(b.pal.accent, c(Accent::Green.rgb()));
        let _ = std::fs::remove_dir_all(&d);
    }

    // ---- the real window, headless: frames with mouse and keyboard input ----

    use crate::{cache::View, state::Core};
    use egui::{epaint::ColorMode, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Shape};

    struct Win {
        ctx: egui::Context,
        core: Core,
        rx: std::sync::mpsc::Receiver<Cmd>,
        st: UiState,
        time: f64,
        /// Content height in points (as the window would be sized).
        height: f32,
        dir: std::path::PathBuf,
    }

    impl Win {
        fn new(name: &str) -> Win {
            let dir = std::env::temp_dir().join(format!("bl-win-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let mon = |n, key: &str| View {
                display_number: Some(n),
                name: Some(format!("Test {n}")),
                key: Some(key.into()),
                values: [Some(50), Some(70)],
                ..View::default()
            };
            let (core, rx) = Core::for_test(vec![mon(1, "a"), mon(2, "b")]);
            let ctx = egui::Context::default();
            apply_theme(&ctx);
            let st = UiState::new(theme::Store::open(Some(dir.join(theme::FILE_NAME))));
            st.apply(&ctx);
            let mut w = Win { ctx, core, rx, st, time: 0.0, height: 0.0, dir };
            for _ in 0..3 {
                w.frame(vec![]);
            }
            w
        }

        fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
            let raw = RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(WINDOW_W, 2000.0))),
                events,
                time: Some(self.time),
                focused: true,
                ..Default::default()
            };
            self.time += 1.0 / 60.0;
            let (core, st, height) = (&mut self.core, &mut self.st, &mut self.height);
            let mut out = self.ctx.run_ui(raw, |ui| {
                if let Some(h) = draw(ui, core, st) {
                    *height = h;
                }
            });
            // No GPU here: the font texture updates are not uploaded anywhere.
            out.textures_delta.clear();
            out
        }

        fn button(&mut self, pos: Pos2, pressed: bool) {
            let e = Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
            self.frame(vec![Event::PointerMoved(pos), e]);
        }

        fn click(&mut self, pos: Pos2) {
            self.button(pos, true);
            self.button(pos, false);
            self.frame(vec![]);
        }

        fn key(&mut self, key: Key) {
            let k =
                |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
            self.frame(vec![k(true)]);
            self.frame(vec![k(false)]);
        }

        fn focused(&self) -> Option<egui::Id> {
            self.ctx.memory(|m| m.focused())
        }
    }

    impl Drop for Win {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Every opaque colour painted in a frame.
    fn colours(out: &egui::FullOutput) -> std::collections::HashSet<Color32> {
        fn walk(s: &Shape, set: &mut std::collections::HashSet<Color32>) {
            match s {
                Shape::Vec(v) => v.iter().for_each(|s| walk(s, set)),
                Shape::Rect(r) => {
                    set.insert(r.fill);
                    set.insert(r.stroke.color);
                }
                Shape::Circle(c) => {
                    set.insert(c.fill);
                    set.insert(c.stroke.color);
                }
                Shape::LineSegment { stroke, .. } => {
                    set.insert(stroke.color);
                }
                Shape::Path(p) => {
                    set.insert(p.fill);
                    if let ColorMode::Solid(c) = p.stroke.color {
                        set.insert(c);
                    }
                }
                Shape::Text(t) => {
                    if let Some(c) = t.override_text_color {
                        set.insert(c);
                    } else {
                        for sec in &t.galley.job.sections {
                            set.insert(if sec.format.color == Color32::PLACEHOLDER {
                                t.fallback_color
                            } else {
                                sec.format.color
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        let mut set = std::collections::HashSet::new();
        for c in &out.shapes {
            walk(&c.shape, &mut set);
        }
        set.retain(|c| c.a() == 255);
        set
    }

    #[test]
    fn palette_button_opens_and_closes_the_panel() {
        let mut w = Win::new("toggle");
        let closed = w.height;
        assert!(closed > 100.0);
        assert!(!w.st.expanded());
        let at = w.st.hits.palette.center();
        // It sits at the top right, under the window's X.
        assert!(at.x > WINDOW_W - 40.0 && at.y < 40.0, "{at:?}");
        w.click(at);
        assert!(w.st.expanded());
        let open = w.height;
        assert!(open > closed + 80.0, "the window grows by the panel: {closed} -> {open}");
        w.click(at);
        assert!(!w.st.expanded());
        assert_eq!(w.height, closed);
    }

    /// 60 fast clicks: the panel follows each one, the window has only two
    /// heights, and nothing is left over (no growing egui state).
    #[test]
    fn rapid_clicks() {
        let mut w = Win::new("rapid");
        let closed = w.height;
        let at = w.st.hits.palette.center();
        w.click(at);
        let open = w.height;
        w.click(at);
        let ids = w.ctx.memory(|m| m.data.len());
        for i in 0..60 {
            w.button(at, true);
            w.button(at, false);
            assert_eq!(w.st.expanded(), i % 2 == 0, "click {i}");
        }
        w.frame(vec![]);
        assert!(!w.st.expanded());
        assert_eq!(w.height, closed);
        assert_ne!(open, closed);
        assert!(w.ctx.memory(|m| m.data.len()) <= ids + 2);
    }

    /// A slider drag that ends over the palette button only sets the slider.
    #[test]
    fn drag_a_slider_and_let_go_over_the_button() {
        let mut w = Win::new("drag");
        while w.rx.try_recv().is_ok() {}
        let slider = w.st.hits.slider.expect("a slider");
        let start = slider.center();
        w.button(start, true);
        for dx in 1..=10 {
            w.frame(vec![Event::PointerMoved(start + Vec2::new(dx as f32 * 4.0, 0.0))]);
        }
        assert!(w.st.dragging());
        // Moving onto the button while dragging, and letting go there.
        let at = w.st.hits.palette.center();
        w.frame(vec![Event::PointerMoved(at)]);
        w.button(at, false);
        w.frame(vec![]);
        assert!(!w.st.expanded(), "a drag is not a click on the button");
        assert!(!w.st.dragging());
        assert!(w.rx.try_iter().any(|c| matches!(c, Cmd::Write { .. })), "the slider value was written");
        // Opening the panel while a drag is in progress elsewhere, then dragging on.
        w.button(start, true);
        w.frame(vec![Event::PointerMoved(start + Vec2::new(-20.0, 0.0))]);
        w.st.toggle_panel();
        w.frame(vec![Event::PointerMoved(start + Vec2::new(-30.0, 0.0))]);
        assert!(w.st.dragging());
        w.button(start + Vec2::new(-30.0, 0.0), false);
        assert!(!w.st.dragging());
        assert!(w.st.expanded());
    }

    /// Tab reaches the palette button first; Space and Enter press it; Tab
    /// goes on to the segments and swatches, which Enter picks.
    #[test]
    fn keyboard() {
        let mut w = Win::new("keys");
        w.key(Key::Tab);
        let palette_id = w.focused().expect("something has the focus");
        w.key(Key::Space);
        assert!(w.st.expanded());
        w.key(Key::Enter);
        assert!(!w.st.expanded());
        w.key(Key::Space);
        assert!(w.st.expanded());
        assert_eq!(w.focused(), Some(palette_id));
        let amoled = w.st.hits.appearance[2].1;
        let mut found = false;
        for _ in 0..40 {
            w.key(Key::Tab);
            if w.focused() == Some(amoled) {
                found = true;
                break;
            }
        }
        assert!(found, "Tab reaches the AMOLED segment");
        w.key(Key::Enter);
        assert_eq!(w.st.theme().appearance, Appearance::Amoled);
        let indigo = w.st.hits.accent[6].1;
        for _ in 0..10 {
            if w.focused() == Some(indigo) {
                break;
            }
            w.key(Key::Tab);
        }
        assert_eq!(w.focused(), Some(indigo));
        w.key(Key::Space);
        assert_eq!(w.st.theme().accent, Accent::Indigo);
        assert!(w.st.expanded(), "picking keeps the panel open");
    }

    /// All 21 combinations, picked with the mouse in the open panel: the
    /// right colours are on screen and none of another theme is left over.
    #[test]
    fn all_21_combinations_on_screen() {
        let mut w = Win::new("combos");
        w.click(w.st.hits.palette.center());
        let open_h = w.height;
        let dark = Theme::default().palette();
        let dark_surfaces = [dark.bg, dark.card, dark.border, dark.border_hover, dark.track, dark.hover, dark.pressed];
        for (ai, appearance) in Appearance::ALL.into_iter().enumerate() {
            for (ci, accent) in Accent::ALL.into_iter().enumerate() {
                w.click(w.st.hits.appearance[ai].0.center());
                w.click(w.st.hits.accent[ci].0.center());
                let t = Theme { appearance, accent };
                assert_eq!(w.st.theme(), t);
                assert_eq!(theme::read(w.st.theme.path.as_deref()), Some(t), "saved at once");
                assert_eq!(w.height, open_h, "switching never changes the size");
                let p = t.palette();
                // Panel open: every swatch is drawn, so check what must be there.
                let open = colours(&w.frame(vec![]));
                for must in [p.bg, p.card, p.border, p.text, p.muted, p.accent] {
                    assert!(open.contains(&c(must)), "{t:?}: {must:?} missing");
                }
                // Panel closed: no other accent, no other appearance's surfaces.
                w.st.toggle_panel();
                w.frame(vec![]);
                let closed = colours(&w.frame(vec![]));
                assert!(closed.contains(&c(p.accent)), "{t:?}: slider fill");
                for other in Accent::ALL.into_iter().filter(|&a| a != accent) {
                    assert!(!closed.contains(&c(other.rgb())), "{t:?}: {other:?} left over");
                }
                if appearance != Appearance::Dark {
                    for old in dark_surfaces {
                        assert!(!closed.contains(&c(old)), "{t:?}: dark colour {old:?} left over");
                    }
                }
                if appearance == Appearance::Light {
                    // Readable: no light text on the light surfaces.
                    assert!(!closed.contains(&c(dark.text)), "{t:?}: light text left over");
                }
                w.st.toggle_panel();
                w.frame(vec![]);
            }
        }
    }

    /// The window is shown after the other app changed theme.json: the next
    /// frame has the new colours and the panel is closed.
    #[test]
    fn shown_after_the_other_app_changed_the_theme() {
        let mut w = Win::new("show");
        w.click(w.st.hits.palette.center());
        let path = w.st.theme.path.clone().unwrap();
        let mut other = theme::Store::open(Some(path));
        let t = Theme { appearance: Appearance::Light, accent: Accent::Red };
        other.set(t);
        // Not before it is shown.
        let before = colours(&w.frame(vec![]));
        assert!(!before.contains(&c(t.palette().bg)));
        let ctx = w.ctx.clone();
        let (core, st, height) = (&mut w.core, &mut w.st, &mut w.height);
        let mut out = ctx.run_ui(RawInput::default(), |ui| {
            st.on_show(ui.ctx());
            if let Some(h) = draw(ui, core, st) {
                *height = h;
            }
        });
        out.textures_delta.clear();
        let after = colours(&out);
        assert!(after.contains(&c(t.palette().bg)) && after.contains(&c(Accent::Red.rgb())));
        assert!(!after.contains(&c(Theme::default().palette().bg)), "no frame mixes old and new colours");
        assert!(!w.st.expanded());
    }

    /// "Reset to saved values" puts the monitor values back and leaves the theme alone.
    #[test]
    fn reset_does_not_touch_the_theme() {
        let mut w = Win::new("reset");
        w.click(w.st.hits.palette.center());
        w.click(w.st.hits.appearance[0].0.center());
        w.click(w.st.hits.accent[3].0.center());
        let t = w.st.theme();
        let path = w.st.theme.path.clone().unwrap();
        let saved = std::fs::read(&path).unwrap();
        while w.rx.try_recv().is_ok() {}
        w.click(w.st.hits.reset.center());
        assert!(w.rx.try_iter().any(|c| matches!(c, Cmd::Reset)));
        assert_eq!(w.st.theme(), t);
        assert_eq!(std::fs::read(&path).unwrap(), saved);
    }

    /// Thousands of toggles and theme changes: egui keeps the same amount of
    /// state and the same textures (nothing is made again and again).
    #[test]
    fn many_changes_leave_nothing_behind() {
        let mut w = Win::new("many");
        let at = w.st.hits.palette.center();
        let warm = |w: &mut Win| {
            for i in 0..21 {
                let t = Theme { appearance: Appearance::ALL[i % 3], accent: Accent::ALL[i % 7] };
                let ctx = w.ctx.clone();
                w.st.pick(&ctx, t);
                w.frame(vec![]);
            }
        };
        w.click(at);
        warm(&mut w);
        w.click(at);
        let mem = w.ctx.memory(|m| m.data.len());
        let tex = w.ctx.tex_manager().read().num_allocated();
        for i in 0..2000 {
            w.button(at, i % 2 == 0);
        }
        for i in 0..2000 {
            let ctx = w.ctx.clone();
            w.st.pick(&ctx, Theme { appearance: Appearance::ALL[i % 3], accent: w.st.theme().accent });
            w.frame(vec![]);
        }
        for i in 0..2000 {
            let ctx = w.ctx.clone();
            w.st.pick(&ctx, Theme { appearance: w.st.theme().appearance, accent: Accent::ALL[i % 7] });
            w.frame(vec![]);
        }
        assert_eq!(w.ctx.tex_manager().read().num_allocated(), tex);
        assert!(w.ctx.memory(|m| m.data.len()) <= mem + 4, "egui state grew");
    }
}
