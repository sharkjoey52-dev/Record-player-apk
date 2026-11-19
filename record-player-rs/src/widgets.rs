//! Custom-drawn widgets: vinyl record, tonearm, round icon buttons, touch sliders.

use eframe::egui::epaint::{Mesh, Vertex};
use eframe::egui::{
    pos2, vec2, Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Response, Sense,
    Shape, Stroke, StrokeKind, TextureHandle, Ui, Vec2,
};
use std::f32::consts::TAU;

pub const BG: Color32 = Color32::from_rgb(0x2a, 0x1a, 0x10);
pub const PANEL: Color32 = Color32::from_rgb(0x3b, 0x24, 0x16);
pub const DARK: Color32 = Color32::from_rgb(0x1b, 0x0f, 0x08);
pub const GOLD: Color32 = Color32::from_rgb(0xd9, 0xa4, 0x41);
pub const FG: Color32 = Color32::from_rgb(0xf1, 0xe6, 0xd4);
pub const DIM: Color32 = Color32::from_rgb(0x9c, 0x8b, 0x78);

// ---------------------------------------------------------------------------
// Turntable (vinyl + tonearm), drawn in the original 320x290 layout and scaled
// ---------------------------------------------------------------------------
pub fn draw_turntable(
    p: &Painter,
    area: Rect,
    angle_deg: f32,
    arm_deg: f32,
    cover: Option<&TextureHandle>,
) {
    let s = (area.width() / 320.0).min(area.height() / 290.0);
    let origin = area.center() - vec2(160.0, 145.0) * s;
    let at = |x: f32, y: f32| origin + vec2(x, y) * s;

    draw_vinyl(p, at(150.0, 150.0), 130.0 * s, angle_deg, cover);

    // Tonearm: pivot top-right, swings in over the record while playing.
    let a = arm_deg.to_radians();
    let pivot = at(300.0, 25.0);
    let end = pivot + vec2(-a.sin(), a.cos()) * 200.0 * s;
    p.circle_filled(pivot, 12.0 * s, Color32::from_rgb(0x88, 0x88, 0x88));
    p.circle_stroke(pivot, 12.0 * s, Stroke::new(1.0, Color32::from_rgb(0x55, 0x55, 0x55)));
    p.line_segment([pivot, end], Stroke::new(7.0 * s, Color32::from_rgb(0xd0, 0xd0, 0xd0)));
    p.circle_filled(end, 7.0 * s, Color32::from_rgb(0x22, 0x22, 0x22));
    p.circle_stroke(end, 7.0 * s, Stroke::new(1.0, Color32::from_rgb(0x88, 0x88, 0x88)));
}

fn arc(c: Pos2, r: f32, from_deg: f32, to_deg: f32) -> Vec<Pos2> {
    (0..=16)
        .map(|i| {
            let t = (from_deg + (to_deg - from_deg) * i as f32 / 16.0).to_radians();
            c + vec2(t.cos(), t.sin()) * r
        })
        .collect()
}

fn draw_vinyl(p: &Painter, c: Pos2, r: f32, angle_deg: f32, cover: Option<&TextureHandle>) {
    p.circle_filled(c, r, Color32::from_rgb(18, 18, 18));
    let groove = Stroke::new(1.0, Color32::from_rgb(42, 42, 42));
    let mut gr = r * 0.48;
    while gr < r * 0.96 {
        p.circle_stroke(c, gr, groove);
        gr += r * 0.031;
    }
    p.circle_stroke(c, r * 0.985, Stroke::new(1.5, Color32::from_rgb(60, 60, 60)));

    // The cover is the record label and rotates with the record.
    let lr = r * 0.423;
    match cover {
        Some(tex) => {
            let n = 64u32;
            let rot = angle_deg.to_radians();
            let mut mesh = Mesh::with_texture(tex.id());
            mesh.vertices.push(Vertex { pos: c, uv: pos2(0.5, 0.5), color: Color32::WHITE });
            for i in 0..=n {
                let th = i as f32 / n as f32 * TAU;
                let (sn, cs) = th.sin_cos();
                let (sn2, cs2) = (th + rot).sin_cos();
                mesh.vertices.push(Vertex {
                    pos: c + vec2(cs2, sn2) * lr,
                    uv: pos2(0.5 + 0.5 * cs, 0.5 + 0.5 * sn),
                    color: Color32::WHITE,
                });
            }
            for i in 0..n {
                mesh.indices.extend([0, i + 1, i + 2]);
            }
            p.add(Shape::mesh(mesh));
        }
        None => {
            p.circle_filled(c, lr, Color32::from_rgb(70, 70, 70));
        }
    }
    p.circle_stroke(c, lr, Stroke::new(2.0, Color32::BLACK));
    p.circle_filled(c, r * 0.046, Color32::from_rgb(14, 14, 14));
    p.circle_stroke(c, r * 0.046, Stroke::new(1.0, Color32::from_rgb(90, 90, 90)));

    // Fixed light reflections (they don't rotate - that is what sells the spin).
    let shine = Stroke::new(r * 0.05, Color32::from_white_alpha(14));
    p.add(Shape::line(arc(c, r * 0.78, 205.0, 250.0), shine));
    p.add(Shape::line(arc(c, r * 0.78, 25.0, 70.0), shine));
}

// ---------------------------------------------------------------------------
// Round icon buttons (drawn with shapes so no special fonts are needed)
// ---------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Shuffle,
    Prev,
    Play,
    Pause,
    Next,
    Repeat,
    RepeatOne,
}

fn tri(p: &Painter, pts: [Pos2; 3], col: Color32) {
    p.add(Shape::convex_polygon(pts.to_vec(), col, Stroke::NONE));
}

pub fn icon_button(ui: &mut Ui, icon: Icon, d: f32, active: bool, primary: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(d), Sense::click());
    let p = ui.painter();
    let c = rect.center();
    let bg = if primary {
        GOLD
    } else if resp.is_pointer_button_down_on() {
        PANEL
    } else {
        DARK
    };
    p.circle_filled(c, d / 2.0, bg);
    let fg = if primary {
        DARK
    } else if active {
        GOLD
    } else {
        FG
    };
    let u = d * 0.2;
    let q = |x: f32, y: f32| c + vec2(x, y) * u;
    let bar = |x0: f32, x1: f32, h: f32| {
        Rect::from_min_max(q(x0, -h), q(x1, h))
    };
    let round = CornerRadius::same(2);

    match icon {
        Icon::Play => tri(p, [q(-0.7, -1.1), q(-0.7, 1.1), q(1.1, 0.0)], fg),
        Icon::Pause => {
            p.rect_filled(bar(-0.95, -0.3, 1.0), round, fg);
            p.rect_filled(bar(0.3, 0.95, 1.0), round, fg);
        }
        Icon::Next => {
            tri(p, [q(-1.1, -0.9), q(-1.1, 0.9), q(0.6, 0.0)], fg);
            p.rect_filled(bar(0.8, 1.15, 0.9), round, fg);
        }
        Icon::Prev => {
            tri(p, [q(1.1, -0.9), q(1.1, 0.9), q(-0.6, 0.0)], fg);
            p.rect_filled(bar(-1.15, -0.8, 0.9), round, fg);
        }
        Icon::Shuffle => {
            let st = Stroke::new((d * 0.05).max(2.0), fg);
            p.line_segment([q(-1.2, -0.8), q(0.7, 0.8)], st);
            p.line_segment([q(-1.2, 0.8), q(0.7, -0.8)], st);
            tri(p, [q(0.5, 0.35), q(0.5, 1.25), q(1.3, 0.8)], fg);
            tri(p, [q(0.5, -1.25), q(0.5, -0.35), q(1.3, -0.8)], fg);
        }
        Icon::Repeat | Icon::RepeatOne => {
            let st = Stroke::new((d * 0.05).max(2.0), fg);
            p.rect_stroke(
                Rect::from_min_max(q(-1.2, -0.75), q(1.2, 0.75)),
                CornerRadius::same((u * 0.6) as u8),
                st,
                StrokeKind::Middle,
            );
            tri(p, [q(0.1, -1.3), q(0.1, -0.2), q(1.0, -0.75)], fg);
            if icon == Icon::RepeatOne {
                p.text(c + vec2(-0.2 * u, 0.1 * u), Align2::CENTER_CENTER, "1", FontId::proportional(u * 1.1), fg);
            }
        }
    }
    resp
}

// ---------------------------------------------------------------------------
// Touch-friendly horizontal slider. value is 0..=1.
// Returns (new value, user is dragging/touching it, user just let go).
// ---------------------------------------------------------------------------
pub fn bar_slider(ui: &mut Ui, width: f32, value: f32) -> (f32, bool, bool) {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 38.0), Sense::click_and_drag());
    let inset = 12.0;
    let track = Rect::from_min_max(
        pos2(rect.left() + inset, rect.center().y - 4.0),
        pos2(rect.right() - inset, rect.center().y + 4.0),
    );
    let mut v = value.clamp(0.0, 1.0);
    let touching = resp.dragged() || resp.drag_started() || resp.is_pointer_button_down_on();
    if touching || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            v = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0);
        }
    }
    let released = resp.drag_stopped() || resp.clicked();

    let p = ui.painter();
    let r = CornerRadius::same(4);
    p.rect_filled(track, r, DARK);
    let filled = Rect::from_min_max(track.min, pos2(track.left() + track.width() * v, track.bottom()));
    p.rect_filled(filled, r, GOLD);
    let knob_r = if touching { 13.0 } else { 10.0 };
    p.circle_filled(pos2(filled.right(), track.center().y), knob_r, GOLD);
    p.circle_stroke(pos2(filled.right(), track.center().y), knob_r, Stroke::new(2.0, DARK));
    (v, touching, released)
}

