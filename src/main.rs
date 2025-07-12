#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use std::{sync::{Arc, RwLock}, thread::sleep, time::{Duration, Instant}};

use eframe::egui::{self, scroll_area::ScrollBarVisibility, FontData, FontDefinitions, FontFamily, Id};
use egui_extras::{install_image_loaders, Column, TableBuilder};
use futures_lite::future;
use minidisc::netmd::{commands::{DeviceStatus, Disc, OperatingStatus}, utils::RawTime, DEVICE_IDS_CROSSUSB};

fn main() -> eframe::Result {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1024.0, 768.0]),
        ..Default::default()
    };

    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("Noto Sans JP".to_owned(),
        std::sync::Arc::new(
            FontData::from_static(include_bytes!("./NotoSansJP-Regular.ttf"))
        )
    );
    fonts.families.get_mut(&FontFamily::Proportional).unwrap()
        .insert(1, "Noto Sans JP".to_owned());

    fonts.font_data.insert("Noto Emoji".to_owned(),
        std::sync::Arc::new(
            FontData::from_static(include_bytes!("./NotoEmoji-Regular.ttf"))
        )
    );
    fonts.families.get_mut(&FontFamily::Proportional).unwrap()
        .push("Noto Emoji".to_owned());

    eframe::run_native(
        "Rust Minidisc Application",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_pixels_per_point(1.5);
            cc.egui_ctx.set_fonts(fonts);

            install_image_loaders(&cc.egui_ctx);
            Ok(Box::<MyApp>::default())
        }),
    )
}

#[derive(Default)]
struct MyApp {
    md_state: Arc<RwLock<MinidiscCommunication>>,

    track_listing_table: TrackListingTable,
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.image("file://./MiniDisc192.png");
                ui.heading("Minidisc Manager");

                if !self.md_state.read().unwrap().connected && ui.button("Connect").clicked() {
                    let state = Arc::clone(&self.md_state);
                    std::thread::spawn(|| {
                        future::block_on(async { minidisc_thread(state).await });
                    });
                } else if let Some(state) = &self.md_state.read().unwrap().device_state {
                    let state = match state.state.unwrap_or(OperatingStatus::NoDisc) {
                        OperatingStatus::Ready => "✅",
                        OperatingStatus::Playing => "▶️",
                        OperatingStatus::Paused => "⏸️",
                        OperatingStatus::FastForward => "⏩",
                        OperatingStatus::Rewind => "⏪",
                        OperatingStatus::ReadingTOC => "🔄",
                        OperatingStatus::NoDisc => "No Disc",
                        OperatingStatus::DiscBlank => "Disc Blank",
                        OperatingStatus::ReadyForTransfer => "Ready",
                    }.to_string();

                    ui.label(state);
                }
            });

            ui.separator();

            let state = self.md_state.read().unwrap();
            if state.reading || state.device_state.is_some_and(|s| s.state.is_some_and(|s| s == OperatingStatus::ReadingTOC)) {
                ui.centered_and_justified(|ui| {
                    ui.spinner()
                });
            } else if state.device_state.is_some_and(|s| !s.disc_present) {
                ui.centered_and_justified(|ui| {
                    ui.heading("⚠ No Disc ⚠");
                });
            } else if state.disc_contents.as_ref().is_some_and(|c| c.track_count() == 0) {
                ui.centered_and_justified(|ui| {
                    ui.heading("Disk Blank");
                });
            } else if let Some(c) = &state.disc_contents {
                self.track_listing_table.table(ui, c);
            } else {
                ui.centered_and_justified(|ui| {
                    ui.heading("How did you do this");
                });
            }
        });

        ctx.request_repaint();
    }
}

#[derive(Default)]
struct TrackListingTable {
    selection: Option<usize>,
}

impl TrackListingTable {
    fn table(&mut self, ui: &mut egui::Ui, disc: &Disc) {
        let text_height = egui::TextStyle::Body
            .resolve(ui.style())
            .size
            .max(ui.spacing().interact_size.y);

        let available_height = ui.available_height();
        let mut table = TableBuilder::new(ui)
            .resizable(false)
            .scroll_bar_visibility(ScrollBarVisibility::VisibleWhenNeeded)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::auto())
            .column(
                Column::remainder()
                    .at_least(40.0)
                    .clip(true),
            )
            .column(Column::auto())
            .column(Column::auto())
            .min_scrolled_height(0.0)
            .max_scroll_height(available_height);

        table = table.sense(egui::Sense::click());

        table.header(20.0, |mut header| {
            header.col(|ui| {
                ui.strong("#");
            });
            header.col(|ui| {
                ui.strong("Title");
            });
            header.col(|ui| {
                ui.strong("Duration");
            });
            header.col(|ui| {
                ui.strong("");
            });
        })
        .body(|body| {
            body.rows(text_height, disc.track_count() as usize, |mut row| {
                let row_track = &disc.tracks()[row.index()];

                let title = if row_track.title().is_empty() {
                    &"No Title".to_string()
                } else {
                    row_track.title()
                };

                row.set_selected(self.selection.is_some_and(|s| s == row.index()));

                row.col(|ui| {
                    ui.label((row_track.index() + 1).to_string());
                });
                row.col(|ui| {
                    ui.label(title);
                });
                row.col(|ui| {
                    ui.label(pretty_duration(row_track.duration()));
                });
                row.col(|ui| {
                    ui.label(" ");
                });

                self.toggle_row_selection(row.index(), &row.response());
            });
        });
    }

    fn toggle_row_selection(&mut self, row_index: usize, row_response: &egui::Response) {
        if row_response.clicked() {
            if self.selection.is_some_and(|s| s == row_index) {
                self.selection = None
            } else {
                self.selection = Some(row_index)
            }
        }
    }
}


fn pretty_duration(duration: RawTime) -> String {
    format!("{:02}:{:02}:{:02}", duration.hours, duration.minutes, duration.seconds)
}

#[derive(Default)]
struct MinidiscCommunication {
    connected: bool,
    reading: bool,

    disc_contents: Option<Disc>,
    device_state: Option<DeviceStatus>,
}

async fn minidisc_thread(comm: Arc<RwLock<MinidiscCommunication>>) {
    let usb_dev = cross_usb::get_device(DEVICE_IDS_CROSSUSB.to_vec()).await.unwrap();
    let mut md_dev = minidisc::netmd::NetMDContext::new(usb_dev).await.unwrap();

    comm.write().unwrap().connected = true;

    println!("Connected to {:?}", md_dev.interface().device.device_name());

    comm.write().unwrap().device_state = md_dev.device_status().await.ok();

    comm.write().unwrap().reading = true;
    comm.write().unwrap().disc_contents = md_dev.list_content().await.ok();
    comm.write().unwrap().reading = false;

    println!("Disc contents got");

    let mut state_timer = Instant::now();
    loop {
        // Check for an updated device state
        if state_timer.elapsed() >= Duration::from_millis(500) {
            let state = md_dev.device_status().await.ok();

            comm.write().unwrap().device_state = state;

            let contents_present = comm.read().unwrap().disc_contents.is_some();
            let disc_present = state.is_some_and(|s| s.disc_present);

            if contents_present && !disc_present {
                comm.write().unwrap().disc_contents = None;
            } else if !contents_present && disc_present {
                comm.write().unwrap().reading = true;
                comm.write().unwrap().disc_contents = md_dev.list_content().await.ok();
                comm.write().unwrap().reading = false;
            }

            state_timer = Instant::now();
        }

        sleep(Duration::from_millis(50));
    }
}
