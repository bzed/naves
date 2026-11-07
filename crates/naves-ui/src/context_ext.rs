use egui::emath::Float as _;
use egui::{Align2, Mesh, Rect, Shape, Vec2, pos2};

use crate::{DesignTokens, TopBarStyle};

#[derive(Clone, Copy, serde::Deserialize, serde::Serialize)]
struct TestMarker;

fn test_marker_id() -> egui::Id {
    egui::Id::new("__rerun_test_marker")
}

/// Extension trait for [`egui::Context`].
///
/// This trait provides Rerun-specific helpers and utilities that require access to the egui
/// context.
pub trait ContextExt {
    fn ctx(&self) -> &egui::Context;

    fn tokens(&self) -> &'static DesignTokens {
        crate::design_tokens_of(self.ctx().theme())
    }

    fn mark_as_test(&self) {
        self.ctx()
            .data_mut(|data| data.insert_persisted(test_marker_id(), TestMarker));
    }

    fn is_test(&self) -> bool {
        self.ctx()
            .data_mut(|data| data.get_persisted::<TestMarker>(test_marker_id()))
            .is_some()
    }

    // -----------------------------------------------------
    // Style-related stuff.
    // We could have this on a `StyleExt` trait, but we prefer to have it here on `Context`
    // so that it is the same style everywhere (instead of being specific to the parent `ui.style`).

    /// Text format used for regular body.
    fn text_format_body(&self) -> egui::TextFormat {
        egui::TextFormat::simple(
            egui::TextStyle::Body.resolve(&self.ctx().global_style()),
            self.ctx().global_style().visuals.text_color(),
        )
    }

    /// Text format used for labels referring to keys and buttons.
    fn text_format_key(&self) -> egui::TextFormat {
        let mut style = egui::TextFormat::simple(
            egui::TextStyle::Monospace.resolve(&self.ctx().global_style()),
            self.ctx().global_style().visuals.text_color(),
        );
        style.background = self
            .ctx()
            .global_style()
            .visuals
            .widgets
            .noninteractive
            .bg_fill;
        style
    }

    fn rerun_logo_uri(&self) -> &'static str {
        if self.ctx().global_style().visuals.dark_mode {
            "bytes://logo_dark_mode"
        } else {
            "bytes://logo_light_mode"
        }
    }

    /// Hovered UI and spatial primitives should have this outline.
    fn hover_stroke(&self) -> egui::Stroke {
        // We want something bright here.
        self.ctx().global_style().visuals.widgets.active.fg_stroke
    }

    /// Selected UI and spatial primitives should have this outline.
    fn selection_stroke(&self) -> egui::Stroke {
        self.ctx().global_style().visuals.selection.stroke

        // It is tempting to use the background selection color for outlines,
        // but in practice it is way too dark for spatial views (you can't tell what is selected).
        // Also: background colors should not be used as stroke colors.
        // let color = self.ctx().global_style().visuals.selection.bg_fill;
        // let stroke_width = self.ctx().global_style().visuals.selection.stroke.width;
        // egui::Stroke::new(stroke_width, color)
    }

    /// Text colored to indicate success.
    #[must_use]
    fn success_text(&self, text: impl Into<String>) -> egui::RichText {
        egui::RichText::new(text).color(self.tokens().success_text_color)
    }

    /// Text colored to indicate a warning.
    ///
    /// For most cases, you should use [`crate::UiExt::warning_label`] instead,
    /// which has a nice fat border around it.
    #[must_use]
    fn warning_text(&self, text: impl Into<String>) -> egui::RichText {
        let style = self.ctx().global_style();
        egui::RichText::new(text).color(style.visuals.warn_fg_color)
    }

    /// Text colored to indicate an error.
    ///
    /// For most cases, you should use [`crate::UiExt::error_label`] instead,
    /// which has a nice fat border around it.
    #[must_use]
    fn error_text(&self, text: impl Into<String>) -> egui::RichText {
        let style = self.ctx().global_style();
        egui::RichText::new(text).color(style.visuals.error_fg_color)
    }

    /// Port note: upstream takes `_frame: &eframe::Frame` here and uses it to measure the
    /// native macOS traffic-light button area (`eframe::WindowChromeMetrics` +
    /// `raw_window_handle`), so the top bar can make room for it in full-size windows.
    /// That chain is not vendored (it is the only reason this crate would need `eframe`,
    /// `raw_window_handle` and `unsafe` code); the port always returns the tokens-based
    /// height with zero indent. Consumers wanting rerun-style full-size chrome on macOS
    /// must reserve the traffic-light indent themselves.
    fn top_bar_style(&self, _style_like_web: bool) -> TopBarStyle {
        let tokens = self.tokens();
        TopBarStyle {
            height: tokens.top_bar_height() - tokens.top_bar_margin().sum().y,
            indent: 0.0,
        }
    }

    /// Paint a watermark
    fn paint_watermark(&self) {
        if let Ok(egui::load::TexturePoll::Ready { texture }) = self.ctx().try_load_texture(
            self.rerun_logo_uri(),
            egui::TextureOptions::default(),
            egui::SizeHint::Scale(1.0.ord()),
        ) {
            let rect = Align2::RIGHT_BOTTOM
                .align_size_within_rect(texture.size, self.ctx().content_rect())
                .translate(-Vec2::splat(16.0));
            let mut mesh = Mesh::with_texture(texture.id);
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            mesh.add_rect_with_uv(rect, uv, self.ctx().tokens().strong_fg_color);
            self.ctx().debug_painter().add(Shape::mesh(mesh));
        }
    }

    /// Whether to show extra information in the UI, e.g. in tooltips.
    ///
    /// This is controlled by holding the `Alt` key down.
    fn show_extras(&self) -> bool {
        self.ctx().input(|input| input.modifiers.alt)
    }
}

impl ContextExt for egui::Context {
    fn ctx(&self) -> &egui::Context {
        self
    }
}
