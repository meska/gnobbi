//! Gnobbi, egui frontend. S0: an empty window with the app identity and the light timeline colour.
use eframe::egui::{self, Color32, ViewportBuilder};
use matrix_desktop::{LIGHT, app_id::{APP_ID, TITLE}};

fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

struct App;

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let fill = rgb(LIGHT.timeline);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(fill))
            .show(ui, |_| {});
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([LIGHT.width, LIGHT.height])
            .with_title(TITLE)
            .with_app_id(APP_ID),
        ..Default::default()
    };
    eframe::run_native(TITLE, options, Box::new(|_| Ok(Box::new(App))))
}

#[cfg(test)]
mod tests {
    #[test]
    fn background_is_the_light_timeline_token() {
        assert_eq!(super::rgb(0xe9f0eb), super::Color32::from_rgb(0xe9, 0xf0, 0xeb));
        assert_eq!(matrix_desktop::LIGHT.timeline, 0xe9f0eb);
    }
}
