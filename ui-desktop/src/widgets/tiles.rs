//! 离线瓦片地图辅助：Web Mercator 坐标换算与 PNG 纹理加载。

use std::path::Path;

use egui::{Context, TextureHandle};

/// 由 Web Mercator 归一化 y (0..1, 北=0) 反算纬度（度）
pub fn merc_y2lat(y: f64) -> f64 {
    let n = std::f64::consts::PI - 2.0 * std::f64::consts::PI * y;
    (n / 2.0).sin().atan2((n / 2.0).cos()) * (180.0 / std::f64::consts::PI)
}

/// 由纬度（度）算 Web Mercator 归一化 y (0..1)
pub fn lat2merc_y(lat_deg: f64) -> f64 {
    let r = lat_deg.to_radians();
    (1.0 - r.tan().ln() / std::f64::consts::PI) / 2.0
}

/// 由经度（度）算 Web Mercator 瓦片 x
pub fn lon2xtile(lon_deg: f64, n: f64) -> f64 {
    (lon_deg + 180.0) / 360.0 * n
}

/// 由纬度（度）算 Web Mercator 瓦片 y
pub fn lat2ytile(lat_deg: f64, n: f64) -> f64 {
    lat2merc_y(lat_deg) * n
}

/// 加载瓦片 PNG 为 egui 纹理；失败返回 1x1 灰纹理（用加载前的 example 占位）。
pub fn load_tile_texture(ctx: &Context, path: &Path, key: &str) -> TextureHandle {
    let tex = ctx.load_texture(
        format!("tile-{key}"),
        egui::ColorImage::example(),
        egui::TextureOptions::default(),
    );
    if let Ok(buf) = std::fs::read(path) {
        if let Ok(img) = image::load_from_memory(&buf) {
            let rgba = img.to_rgba8();
            let (iw, ih) = (rgba.width() as usize, rgba.height() as usize);
            let pixels = rgba.into_raw();
            let color_img = egui::ColorImage::from_rgba_unmultiplied([iw, ih], &pixels);
            return ctx.load_texture(
                format!("tile-{key}"),
                color_img,
                egui::TextureOptions::default(),
            );
        }
    }
    tex
}
