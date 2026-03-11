use std::fs;

use eframe::egui;

pub fn setup_cjk_fonts(ctx: &egui::Context) {
    let candidates = [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\simhei.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
    ];

    let Some(font_bytes) = candidates.iter().find_map(|path| fs::read(path).ok()) else {
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "cjk".to_string(),
        egui::FontData::from_owned(font_bytes).into(),
    );

    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "cjk".to_string());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, "cjk".to_string());

    ctx.set_fonts(fonts);
}

pub fn apply_adaptive_theme(ctx: &egui::Context, dark_mode: bool) {
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size.y = 30.0;

    if dark_mode {
        style.visuals.panel_fill = egui::Color32::from_rgb(20, 25, 33);
        style.visuals.extreme_bg_color = egui::Color32::from_rgb(13, 17, 24);
        style.visuals.window_fill = egui::Color32::from_rgb(20, 25, 33);
        style.visuals.selection.bg_fill = egui::Color32::from_rgb(50, 108, 170);
        style.visuals.selection.stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
    } else {
        style.visuals.panel_fill = egui::Color32::from_rgb(252, 253, 255);
        style.visuals.extreme_bg_color = egui::Color32::from_rgb(238, 243, 250);
        style.visuals.window_fill = egui::Color32::from_rgb(252, 253, 255);
        style.visuals.selection.bg_fill = egui::Color32::from_rgb(120, 176, 243);
        style.visuals.selection.stroke = egui::Stroke::new(1.0, egui::Color32::BLACK);
    }

    ctx.set_style(style);
}
