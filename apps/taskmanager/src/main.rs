//! Desktop preview of Task Manager (phone-shaped window, made-up numbers).
fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([405.0, 880.0]),
        ..Default::default()
    };
    eframe::run_native("Task Manager", options, Box::new(|cc| Ok(Box::new(taskmgr::TaskMgr::new(cc)))))
}
