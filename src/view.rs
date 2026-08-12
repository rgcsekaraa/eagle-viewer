use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};

pub struct ViewTransform {
    pub zoom: f32,
    target_zoom: f32,
    pub pan: Vec2,
    target_pan: Vec2,
}

impl Default for ViewTransform {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            target_zoom: 1.0,
            pan: Vec2::ZERO,
            target_pan: Vec2::ZERO,
        }
    }
}

impl ViewTransform {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn set_actual_size(&mut self, viewport: Rect, image_size: [u32; 2]) {
        let image = vec2(image_size[0] as f32, image_size[1] as f32);
        let fit = (viewport.width() / image.x)
            .min(viewport.height() / image.y)
            .min(1.0);
        self.target_zoom = 1.0 / fit.max(0.000_001);
        self.zoom = self.target_zoom;
        self.pan = Vec2::ZERO;
        self.target_pan = Vec2::ZERO;
    }

    pub fn zoom_step(&mut self, factor: f32) {
        self.target_zoom = (self.target_zoom * factor).clamp(0.05, 64.0);
    }

    pub fn zoom_by(&mut self, factor: f32, pointer: Option<Pos2>, viewport: Rect) {
        let old_zoom = self.target_zoom;
        self.target_zoom = (self.target_zoom * factor).clamp(0.05, 64.0);

        if let Some(pointer) = pointer {
            let from_center = pointer - viewport.center() - self.target_pan;
            let ratio = self.target_zoom / old_zoom;
            self.target_pan -= from_center * (ratio - 1.0);
        }
        // Direct manipulation should track the mouse/trackpad with no animation lag.
        self.zoom = self.target_zoom;
        self.pan = self.target_pan;
    }

    pub fn pan_by(&mut self, delta: Vec2) {
        self.pan += delta;
        self.target_pan += delta;
    }

    pub fn tick(&mut self, dt: f32) -> bool {
        let amount = 1.0 - (-22.0 * dt.min(0.1)).exp();
        self.zoom += (self.target_zoom - self.zoom) * amount;
        self.pan += (self.target_pan - self.pan) * amount;

        let moving = (self.target_zoom - self.zoom).abs() > 0.0005
            || (self.target_pan - self.pan).length_sq() > 0.05;
        if !moving {
            self.zoom = self.target_zoom;
            self.pan = self.target_pan;
        }
        moving
    }

    pub fn image_rect(&self, viewport: Rect, image_size: [u32; 2]) -> Rect {
        let image = vec2(image_size[0] as f32, image_size[1] as f32);
        let fit = (viewport.width() / image.x)
            .min(viewport.height() / image.y)
            .min(1.0);
        let size = image * fit * self.zoom;
        Rect::from_center_size(viewport.center() + self.pan, size)
    }

    pub fn actual_scale(&self, viewport: Rect, image_size: [u32; 2]) -> f32 {
        let image = vec2(image_size[0] as f32, image_size[1] as f32);
        let fit = (viewport.width() / image.x)
            .min(viewport.height() / image.y)
            .min(1.0);
        fit * self.zoom
    }

    #[allow(dead_code)]
    pub fn center(&self, viewport: Rect) -> Pos2 {
        pos2(
            viewport.center().x + self.pan.x,
            viewport.center().y + self.pan.y,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_never_upscales_by_default() {
        let view = ViewTransform::default();
        let rect = view.image_rect(
            Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 1000.0)),
            [100, 50],
        );
        assert_eq!(rect.size(), vec2(100.0, 50.0));
    }

    #[test]
    fn pointer_zoom_keeps_the_source_point_under_the_cursor() {
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0));
        let pointer = pos2(700.0, 300.0);
        let mut view = ViewTransform::default();
        let before = view.image_rect(viewport, [2000, 1000]);
        let source = (pointer - before.min) / before.size();

        view.zoom_by(2.0, Some(pointer), viewport);
        let after = view.image_rect(viewport, [2000, 1000]);
        let projected = after.min + source * after.size();

        assert!((projected - pointer).length() < 0.01);
    }

    #[test]
    fn panning_moves_the_rendered_image_immediately() {
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0));
        let mut view = ViewTransform::default();
        let before = view.image_rect(viewport, [2000, 1000]);

        view.pan_by(vec2(45.0, -30.0));
        let after = view.image_rect(viewport, [2000, 1000]);

        assert_eq!(after.center() - before.center(), vec2(45.0, -30.0));
    }
}
