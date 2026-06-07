use eframe::egui::{self, Color32, Grid, RichText, Slider, TextEdit};
use std::path::PathBuf;

/// Modal that creates a blank zero-filled disk image for a chosen SCSI ID.
/// Mirrors snow's DiskImageDialog.
pub struct CreateDiskDialog {
    open: bool,
    scsi_id: u8,
    filename: String,
    size_mb: f64,
    result: Option<CreateDiskResult>,
}

pub struct CreateDiskResult {
    pub scsi_id: u8,
    pub path: PathBuf,
}

impl Default for CreateDiskDialog {
    fn default() -> Self {
        Self { open: false, scsi_id: 1, filename: String::new(), size_mb: 1024.0, result: None }
    }
}

impl CreateDiskDialog {
    pub fn open_for(&mut self, scsi_id: u8) {
        self.scsi_id = scsi_id;
        self.filename = format!("scsi{scsi_id}.raw");
        self.size_mb = 1024.0;
        self.result = None;
        self.open = true;
    }
    pub fn take_result(&mut self) -> Option<CreateDiskResult> { self.result.take() }

    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open { return; }
        let mut close = false;
        egui::Window::new(format!("Create blank HDD image for SCSI #{}", self.scsi_id))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(380.0);
                Grid::new("create_disk_grid").num_columns(2).striped(true).show(ui, |ui| {
                    ui.label("Filename");
                    ui.horizontal(|ui| {
                        ui.add(TextEdit::singleline(&mut self.filename).desired_width(220.0));
                        if ui.button("📁").clicked() {
                            if let Some(p) = rfd::FileDialog::new()
                                .add_filter("Disk image", &["raw", "img"])
                                .set_file_name(&self.filename)
                                .save_file()
                            {
                                self.filename = p.to_string_lossy().into_owned();
                            }
                        }
                    });
                    ui.end_row();
                    ui.label("Size (MB)");
                    ui.add(Slider::new(&mut self.size_mb, 8.0..=16384.0).step_by(8.0).logarithmic(true));
                    ui.end_row();
                });
                ui.add_space(4.0);
                ui.label(RichText::new("The image will be created as a zero-filled file. \
                    Reset the machine after attaching new drives.")
                    .color(Color32::GRAY).small());
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() { close = true; }
                    if ui.add(egui::Button::new("Create")
                        .fill(Color32::from_rgb(60, 110, 60))).clicked()
                    {
                        // Create file on disk now.
                        let path = PathBuf::from(&self.filename);
                        let size_bytes = (self.size_mb * 1024.0 * 1024.0) as u64;
                        match std::fs::File::create(&path)
                            .and_then(|f| f.set_len(size_bytes))
                        {
                            Ok(_) => {
                                self.result = Some(CreateDiskResult { scsi_id: self.scsi_id, path });
                                close = true;
                            }
                            Err(e) => {
                                // Show an inline error; keep dialog open.
                                log::error!("create disk image failed: {e}");
                            }
                        }
                    }
                });
            });
        if close { self.open = false; }
    }
}
