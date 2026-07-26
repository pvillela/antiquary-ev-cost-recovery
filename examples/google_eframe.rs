use chrono::{self, Datelike};
use eframe::egui;
use egui_extras::DatePickerButton;
use jiff::civil::{Date, DateTime};
use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 500.0])
            .with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        "Excel Metric Processor (Jiff Engine)",
        native_options,
        Box::new(|_cc| Ok(Box::new(MyApp::default()))),
    )
}

struct MyApp {
    // Inputs
    excel_path: Option<PathBuf>,

    // Start Date/Time State
    start_date_text: String,
    start_time_text: String,

    // End Date/Time State
    end_date_text: String,
    end_time_text: String,

    // Outputs & State
    results: Option<[f64; 4]>,
    export_filename: String,

    // Warning Modal State
    show_overwrite_warning: bool,
    pending_save_path: Option<PathBuf>,
}

impl Default for MyApp {
    fn default() -> Self {
        // Fetch current system date and time via Jiff
        let now = jiff::Zoned::now().datetime();
        let today = now.date();

        Self {
            excel_path: None,

            start_date_text: today.to_string(), // Formats automatically as YYYY-MM-DD
            start_time_text: "00:00:00".to_string(),

            end_date_text: today.to_string(),
            end_time_text: now.time().to_string(), // Formats automatically as HH:MM:SS

            results: None,
            export_filename: "metrics_output.txt".to_string(),
            show_overwrite_warning: false,
            pending_save_path: None,
        }
    }
}

impl MyApp {
    fn run_calculation(&mut self) {
        if self.excel_path.is_some() {
            self.results = Some([12.34, 56.78, 91.01, 112.13]);
        }
    }

    // Helper to evaluate, parse, and validate inputs using Jiff's parsing engine
    fn validate_inputs(&self) -> Result<(DateTime, DateTime), String> {
        let start_dt = format!("{}T{}", self.start_date_text, self.start_time_text)
            .parse()
            .map_err(|_| "Invalid Start Time format (use HH:MM:SS)".to_string())?;

        // 2. Validate End Date & Time
        let end_dt = format!("{}T{}", self.end_date_text, self.end_time_text)
            .parse()
            .map_err(|_| "Invalid End Time format (use HH:MM:SS)".to_string())?;

        // 3. Chronological Constraint Verification
        if end_dt < start_dt {
            return Err("End Date/Time cannot be earlier than Start Date/Time".to_string());
        }

        Ok((start_dt, end_dt))
    }

    fn try_save_file(&mut self) {
        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
            let full_path = folder.join(&self.export_filename);

            if full_path.exists() {
                self.pending_save_path = Some(full_path);
                self.show_overwrite_warning = true;
            } else {
                self.execute_write(full_path);
            }
        }
    }

    fn execute_write(&mut self, path: PathBuf) {
        if let Some(data) = self.results {
            let content = format!(
                "Result 1: {}\nResult 2: {}\nResult 3: {}\nResult 4: {}\n",
                data[0], data[1], data[2], data[3]
            );
            if std::fs::write(path, content).is_ok() {
                println!("File saved successfully.");
            }
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Makes everything 50% bigger than default
        ctx.set_zoom_factor(1.4);
        ctx.set_theme(egui::Theme::Light);
        ctx.style_mut(|style| {
            let visuals = &mut style.visuals;

            // Increase border stroke width and contrast for inactive widgets
            visuals.widgets.inactive.bg_stroke =
                egui::Stroke::new(1.0, egui::Color32::from_gray(100));
            // visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_gray(45);

            // Make hovered widgets pop more
            visuals.widgets.hovered.bg_stroke =
                egui::Stroke::new(1.5, egui::Color32::from_rgb(100, 150, 250));
            visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(100, 250, 150);

            // Sharpen extreme background elements (TextEdits, etc.)
            // visuals.extreme_bg_color = egui::Color32::from_gray(20);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Excel Processor Settings (Jiff Power)");
            ui.add_space(10.0);

            // 1. File Selection Row
            ui.horizontal(|ui| {
                if ui.button("Select Excel File...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Excel Files", &["xlsx", "xls", "xlsm"])
                        .pick_file()
                    {
                        self.excel_path = Some(path);
                    }
                }
                if let Some(ref path) = self.excel_path {
                    ui.label(format!(
                        "Selected: {}",
                        path.file_name().unwrap().to_string_lossy()
                    ));
                } else {
                    ui.label("No file chosen");
                }
            });
            ui.add_space(12.0);

            // Run validation check dynamically on every frame swap
            let validation_result = self.validate_inputs();

            // 2. Twin Interactive Date-Time Pickers
            egui::Grid::new("date_grid")
                .spacing([10.0, 10.0])
                .show(ui, |ui| {
                    // START Row
                    ui.label("Start Input:");
                    ui.horizontal(|ui| {
                        // Interface standard library `time` compatibility for DatePickerButton
                        let current_jiff_date: Date =
                            self.start_date_text.parse().unwrap_or_default();
                        let mut temp_time_date = chrono::NaiveDate::from_ymd_opt(
                            current_jiff_date.year() as i32,
                            current_jiff_date.month() as u32,
                            current_jiff_date.day() as u32,
                        )
                        .unwrap_or_default();

                        let prev_date = temp_time_date;
                        ui.add(DatePickerButton::new(&mut temp_time_date).id_salt("start_picker"));
                        if temp_time_date != prev_date {
                            if let Ok(jd) = Date::new(
                                temp_time_date.year() as i16,
                                temp_time_date.month() as i8,
                                temp_time_date.day() as i8,
                            ) {
                                self.start_date_text = jd.to_string();
                            }
                        }

                        ui.add(
                            egui::TextEdit::singleline(&mut self.start_date_text)
                                .desired_width(85.0),
                        );
                        ui.label("Time:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.start_time_text)
                                .desired_width(70.0),
                        );
                    });
                    ui.end_row();

                    // END Row
                    ui.label("End Input:");
                    ui.horizontal(|ui| {
                        let current_jiff_date: Date =
                            self.end_date_text.parse().unwrap_or_default();
                        let mut temp_time_date = chrono::NaiveDate::from_ymd_opt(
                            current_jiff_date.year() as i32,
                            current_jiff_date.month() as u32,
                            current_jiff_date.day() as u32,
                        )
                        .unwrap_or_default();

                        let prev_date = temp_time_date;
                        ui.add(DatePickerButton::new(&mut temp_time_date).id_salt("end_picker"));
                        if temp_time_date != prev_date {
                            if let Ok(jd) = Date::new(
                                temp_time_date.year() as i16,
                                temp_time_date.month() as i8,
                                temp_time_date.day() as i8,
                            ) {
                                self.end_date_text = jd.to_string();
                            }
                        }

                        ui.add(
                            egui::TextEdit::singleline(&mut self.end_date_text).desired_width(85.0),
                        );
                        ui.label("Time:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.end_time_text).desired_width(70.0),
                        );
                    });
                    ui.end_row();
                });
            ui.add_space(8.0);

            // 3. Dynamic Custom Validation Label Rendering
            match &validation_result {
                Ok(_) => {
                    ui.add_space(18.0);
                } // Invisible structural spacer when text passes validation cleanly
                Err(err_msg) => {
                    ui.colored_label(egui::Color32::RED, format!("⚠️ {}", err_msg));
                }
            }

            // 4. Execution Action Activation
            let can_calculate = self.excel_path.is_some() && validation_result.is_ok();
            if ui
                .add_enabled(can_calculate, egui::Button::new("Process & Calculate"))
                .clicked()
            {
                self.run_calculation();
            }
            ui.add_space(15.0);

            // 5. Results & Export Rendering
            if let Some(metrics) = self.results {
                ui.separator();
                ui.heading("Calculated Results");

                for (i, val) in metrics.iter().enumerate() {
                    ui.label(format!("Metric {}: {:.2}", i + 1, val));
                }

                ui.add_space(15.0);
                ui.separator();

                ui.horizontal(|ui| {
                    ui.label("Export Filename:");
                    ui.text_edit_singleline(&mut self.export_filename);
                });
                ui.add_space(5.0);

                if ui.button("Save Text File").clicked() {
                    self.try_save_file();
                }
            }
        });

        // Overwrite Warning Modal Window Overlay
        if self.show_overwrite_warning {
            egui::Window::new("⚠️ Warning: File Exists")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("A file with this name already exists in the selected directory.");
                    ui.label("Do you want to overwrite it?");
                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        if ui.button("Yes, Overwrite").clicked() {
                            if let Some(path) = self.pending_save_path.take() {
                                self.execute_write(path);
                            }
                            self.show_overwrite_warning = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.pending_save_path = None;
                            self.show_overwrite_warning = false;
                        }
                    });
                });
        }
    }
}
