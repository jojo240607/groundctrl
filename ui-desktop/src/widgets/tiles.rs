//! 离线/在线瓦片地图辅助：Web Mercator 坐标换算、PNG 纹理加载与在线下载缓存。

use std::path::{Path, PathBuf};

use egui::{Context, TextureHandle};

/// 默认在线瓦片源（OpenStreetMap 标准瓦片服务）
pub const DEFAULT_TILE_URL: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";

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

/// 默认瓦片缓存目录：<用户配置目录>/groundctrl/tiles
pub fn default_tile_cache_dir() -> Option<PathBuf> {
    let mut dir = crate::app::AppSettings::config_dir()?;
    dir.push("tiles");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// 构造某瓦片在磁盘上的路径（{root}/{z}/{x}/{y}.png）
pub fn tile_path(root: &Path, z: i32, x: i32, y: i32) -> PathBuf {
    root.join(format!("{z}"))
        .join(format!("{x}"))
        .join(format!("{y}.png"))
}

/// 构造在线瓦片 URL（默认 OSM）
pub fn tile_url(z: i32, x: i32, y: i32, base: &str) -> String {
    base.replace("{z}", &z.to_string())
        .replace("{x}", &x.to_string())
        .replace("{y}", &y.to_string())
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

/// 异步下载单个瓦片到缓存目录（磁盘）。成功返回 true。
/// 在 tokio 后台调用，不触碰 egui。若目录不可写或网络失败返回 false。
pub async fn fetch_tile(z: i32, x: i32, y: i32, cache_dir: &Path, base_url: &str) -> bool {
    let url = tile_url(z, x, y, base_url);
    let path = tile_path(cache_dir, z, x, y);
    if path.exists() {
        return true;
    }
    // 确保父目录存在
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return false;
        }
    }
    let client = reqwest::Client::builder()
        .user_agent("groundctrl-gcs/0.1")
        .build();
    let client = match client {
        Ok(c) => c,
        Err(_) => return false,
    };
    match client.get(&url).send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                return false;
            }
            match resp.bytes().await {
                Ok(bytes) => std::fs::write(&path, &bytes).is_ok(),
                Err(_) => false,
            }
        }
        Err(e) => {
            tracing::warn!("瓦片下载失败 {z}/{x}/{y}: {e}");
            false
        }
    }
}

