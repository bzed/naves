// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The top-down map as a [`Viewport`].

use eframe::egui::{self, Color32};
use naves_core::imagery::TileImagery;
use std::collections::BTreeMap;

use naves_core::render::{LayerData, MapLayer, RenderRequest, render_layer, render_topdown};

use super::{ViewEvent, ViewFocus, ViewFrame, Viewport, VisibleLayer, WorldPoint};
use crate::gui::map_view::MapView;

/// Widest picture the map renders; larger windows scale it up.
const MAX_RENDER_WIDTH: f32 = 1024.0;

fn to_px(v: f32) -> u32 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 1..=8192 first"
    )]
    let px = v.round().clamp(1.0, 8192.0) as u32;
    px
}

/// The map seen from above: pan, zoom and the base map as a texture.
#[derive(Default)]
pub struct MapViewport {
    view: Option<MapView>,
    size: (f64, f64),
    pending: Option<ViewFocus>,
    texture: Option<(RenderRequest, u64, egui::TextureHandle)>,
    /// One picture per layer, so that fading or hiding one renders nothing.
    layer_textures: BTreeMap<MapLayer, (RenderRequest, u64, egui::TextureHandle)>,
    /// How many times a picture was rendered.
    pub renders: u64,
}

impl MapViewport {
    fn point(at: (f64, f64)) -> WorldPoint {
        WorldPoint { x: at.0, y: at.1 }
    }
}

impl Viewport for MapViewport {
    fn name(&self) -> &'static str {
        "Map"
    }

    fn show(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        frame: &ViewFrame<'_>,
    ) -> Vec<ViewEvent> {
        let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
        if w < 1.0 || h < 1.0 {
            return Vec::new();
        }
        self.size = (w, h);
        let map_size = frame.project.map_size_m();
        let mut view = self
            .view
            .take()
            .unwrap_or_else(|| match self.pending.take() {
                Some(focus) => MapView::from_focus(focus, w, h),
                None => MapView::fit(map_size, w, h),
            });
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        // Space + drag and the middle button pan; a plain left drag belongs to the tools
        let space = ui.input(|i| i.key_down(egui::Key::Space));
        let panning = response.dragged_by(egui::PointerButton::Middle)
            || (space && response.dragged_by(egui::PointerButton::Primary));
        if panning {
            let d = response.drag_delta();
            view.pan_px(f64::from(d.x), f64::from(d.y));
        }
        if let Some(pos) = response.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let factor = MapView::wheel_factor(f64::from(scroll), frame.invert_zoom);
                view.zoom_at(
                    f64::from(pos.x - rect.left()),
                    f64::from(pos.y - rect.top()),
                    w,
                    h,
                    factor,
                );
            }
        }
        let at = |pos: egui::Pos2| {
            Self::point(view.screen_to_map(
                f64::from(pos.x - rect.left()),
                f64::from(pos.y - rect.top()),
                w,
                h,
            ))
        };
        let modifiers = ui.input(|i| i.modifiers);
        let mut events = Vec::new();
        if let Some(pos) = response.hover_pos() {
            events.push(ViewEvent::Hover(at(pos)));
        }
        if let Some(pos) = response.interact_pointer_pos() {
            if !panning && response.drag_started() {
                events.push(ViewEvent::Press {
                    at: at(pos),
                    modifiers,
                });
            }
            if !panning && response.dragged() {
                events.push(ViewEvent::Drag {
                    at: at(pos),
                    modifiers,
                });
            }
            if !panning && response.drag_stopped() {
                events.push(ViewEvent::Release {
                    at: at(pos),
                    modifiers,
                });
            }
            if response.clicked() {
                events.push(ViewEvent::Click {
                    at: at(pos),
                    modifiers,
                });
            }
        } else if !response.hovered() {
            events.push(ViewEvent::Left);
        }
        self.draw(ui, rect, frame, &view);
        self.view = Some(view);
        events
    }

    fn focus(&self) -> Option<ViewFocus> {
        match self.view {
            Some(view) => Some(ViewFocus {
                center: Self::point(view.center),
                span_m: view.span_m(self.size.0, self.size.1),
            }),
            None => self.pending,
        }
    }

    fn set_focus(&mut self, focus: ViewFocus) {
        if self.view.is_some() {
            self.view = Some(MapView::from_focus(focus, self.size.0, self.size.1));
        } else {
            self.pending = Some(focus);
        }
    }

    fn project(&self, at: WorldPoint, rect: egui::Rect) -> Option<egui::Pos2> {
        let view = self.view?;
        let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
        let (sx, sy) = view.map_to_screen(at.x, at.y, w, h);
        #[expect(clippy::cast_possible_truncation, reason = "screen pixels fit an f32")]
        let pos = egui::pos2(rect.left() + sx as f32, rect.top() + sy as f32);
        rect.contains(pos).then_some(pos)
    }

    fn reset(&mut self) {
        self.view = None;
        self.pending = None;
        self.texture = None;
        self.layer_textures.clear();
    }

    fn invalidate(&mut self) {
        self.texture = None;
        self.layer_textures.clear();
    }

    #[cfg(test)]
    fn renders(&self) -> u64 {
        self.renders
    }

    #[cfg(test)]
    fn holds_picture(&self) -> bool {
        self.texture.is_some()
    }
}

impl MapViewport {
    fn draw(&mut self, ui: &mut egui::Ui, rect: egui::Rect, frame: &ViewFrame<'_>, view: &MapView) {
        let (x0, y0, x1, y1) = view.bounds(f64::from(rect.width()), f64::from(rect.height()));
        let request = RenderRequest {
            x0,
            y0,
            x1,
            y1,
            px: to_px(rect.width().min(MAX_RENDER_WIDTH)),
            overlay: frame.base,
        };
        let stale = self
            .texture
            .as_ref()
            .is_none_or(|(r, g, _)| *r != request || *g != frame.generation);
        if stale
            && let Ok(image) = render_topdown(
                &frame.data.heightmap,
                &request,
                Some(&frame.data.surfaces),
                frame
                    .data
                    .imagery
                    .as_ref()
                    .map(|loaded| TileImagery {
                        composite: &loaded.composite,
                        project: frame.project,
                    })
                    .as_ref(),
            )
        {
            let size = [image.width() as usize, image.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
            let handle = ui
                .ctx()
                .load_texture("map", color, egui::TextureOptions::LINEAR);
            self.texture = Some((request, frame.generation, handle));
            self.renders += 1;
        }
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        if let Some((_, _, texture)) = &self.texture {
            ui.painter()
                .with_clip_rect(rect)
                .image(texture.id(), rect, uv, Color32::WHITE);
        }
        for visible in frame.layers {
            self.draw_layer(ui, rect, frame, &request, *visible);
        }
    }

    /// One layer over the base map. Its picture is made once per rectangle and generation; the opacity is
    /// only a tint when it is painted.
    fn draw_layer(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        frame: &ViewFrame<'_>,
        request: &RenderRequest,
        visible: VisibleLayer,
    ) {
        let stale = self
            .layer_textures
            .get(&visible.layer)
            .is_none_or(|(r, g, _)| r != request || *g != frame.generation);
        if stale
            && let Ok(image) = render_layer(
                visible.layer,
                &LayerData {
                    heightmap: &frame.data.heightmap,
                },
                request,
            )
        {
            let size = [image.width() as usize, image.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
            let handle = ui.ctx().load_texture(
                format!("layer-{}", visible.layer.id()),
                color,
                egui::TextureOptions::LINEAR,
            );
            self.layer_textures
                .insert(visible.layer, (*request, frame.generation, handle));
            self.renders += 1;
        }
        if let Some((_, _, texture)) = self.layer_textures.get(&visible.layer) {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().with_clip_rect(rect).image(
                texture.id(),
                rect,
                uv,
                Color32::WHITE.gamma_multiply(visible.opacity),
            );
        }
    }
}
