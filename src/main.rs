#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use std::{path::PathBuf, sync::{mpsc, Arc, RwLock}, thread::sleep, time::{Duration, Instant}};

use anyhow::Result;
use eframe::egui::{self, include_image, scroll_area::ScrollBarVisibility, Align, FontData, FontDefinitions, FontFamily, ProgressBar};
use egui_extras::{install_image_loaders, Column, TableBuilder};
use futures_lite::future;
use minidisc::netmd::{commands::{DeviceStatus, Disc, OperatingStatus as OS}, interface::{Action, Direction, MDTrack}, NetMDContext, DEVICE_IDS_CROSSUSB};

fn main() -> eframe::Result {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1024.0, 768.0]),
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
            Ok(Box::<MinidiscManager>::default())
        }),
    )
}

#[derive(Default)]
struct MinidiscManager {
    md_state: Arc<RwLock<PlayerState>>,
    md_channel: Option<mpsc::Sender<PlayerCommand>>,

    track_listing_table: TrackListingTable,
}

impl eframe::App for MinidiscManager {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("status_bar").exact_height(35.).show(ctx, |ui| {
            ui.columns_const(|[col_1, col_2]| {
                col_1.horizontal_centered(|ui| {
                    ui.image(include_image!("./MiniDisc192.png"));
                    ui.heading("Minidisc Manager");
                });
                col_2.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if !self.md_state.read().unwrap().connected && ui.button("Connect").clicked() {
                        self.connect_to_device();
                    } else if let Some(state) = &self.md_state.read().unwrap().device_state {
                        let state = match state.state.unwrap_or(OS::NoDisc) {
                            OS::Ready => "✅",
                            OS::Playing => "▶️",
                            OS::Paused => "⏸️",
                            OS::FastForward => "⏩",
                            OS::Rewind => "⏪",
                            OS::ReadingTOC => "🔄",
                            OS::NoDisc => "No Disc",
                            OS::DiscBlank => "Disc Blank",
                            OS::ReadyForTransfer => "Ready",
                        }.to_string();

                        ui.label(state).on_hover_text("Status");
                    }

                    ui.separator();

                    if let Some(dc) = &self.md_state.read().unwrap().disc_contents {
                        ui.add(egui::Label::new(dc.title()).truncate());
                    }
                });
            });
        });

        egui::TopBottomPanel::bottom("controls").exact_height(40.).show(ctx, |ui| {
            ui.columns_const(|[col_1, col_2, col_3]| {
                col_1.horizontal_centered(|ui| {
                    if ui.button("Upload").clicked() {
                        if let Some(c) = self.md_channel.as_mut() {
                            let _ = c.send(PlayerCommand::Upload("./bad_apple.raw".into()));
                        }
                    }

                    if ui.button("⏯").clicked() {
                        if let Some(c) = self.md_channel.as_mut() {
                            let dev_state = self.md_state.read().unwrap().device_state;
                            if dev_state.is_some_and(|s| s.state.is_some_and(|s| s == OS::Playing)) {
                                let _ = c.send(PlayerCommand::Playback(Action::Pause));
                            } else {
                                let _ = c.send(PlayerCommand::Playback(Action::Play));
                            }
                        }
                    }

                    if ui.button("⏹").clicked() {
                        if let Some(c) = self.md_channel.as_mut() {
                            let _ = c.send(PlayerCommand::Stop);
                        }
                    }

                    if ui.button("⏮").clicked() {
                        if let Some(c) = self.md_channel.as_mut() {
                            let _ = c.send(PlayerCommand::SkipTrack(Direction::Previous));
                        }
                    }

                    if ui.button("⏭").clicked() {
                        if let Some(c) = self.md_channel.as_mut() {
                            let _ = c.send(PlayerCommand::SkipTrack(Direction::Next));
                        }
                    }
                });
                col_2.with_layout(egui::Layout::centered_and_justified(egui::Direction::TopDown), |ui| {
                    if let Some(s) = self.md_state.read().unwrap().device_state
                        && let Some(dc) = &self.md_state.read().unwrap().disc_contents
                    {
                        if (s.track as usize) < dc.tracks().len() {
                            ui.add(ProgressBar::new(
                                Duration::from(s.time).as_secs_f32() / dc.tracks()[s.track as usize].duration().as_duration().as_secs_f32()
                            ).corner_radius(2.));
                        }
                    } else {
                        ui.add(ProgressBar::new(0.0).corner_radius(2.));
                    }
                });
                col_3.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if let Some(s) = self.md_state.read().unwrap().device_state {
                        ui.label(pretty_duration(s.time.into()))
                    } else {
                        ui.label("00:00:00")
                    }
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let state = self.md_state.read().unwrap();
            if let Some(p) = state.progress {
                ui.centered_and_justified(|ui| {
                    ui.add(egui::ProgressBar::new(p).show_percentage().animate(true))
                });
            } else if state.reading || state.device_state.is_some_and(|s| s.state.is_some_and(|s| s == OS::ReadingTOC)) {
                ui.centered_and_justified(|ui| {
                    ui.spinner()
                });
            } else if state.device_state.is_some_and(|s| !s.disc_present) {
                ui.centered_and_justified(|ui| {
                    ui.heading("No Disc");
                });
            } else if state.disc_contents.as_ref().is_some_and(|c| c.track_count() == 0) {
                ui.centered_and_justified(|ui| {
                    ui.heading("Disk Blank");
                });
            } else if let Some(c) = &state.disc_contents {
                let playing_track = if let Some(s) = state.device_state {
                    if s.state.is_some_and(|s| {
                        s == OS::Playing || s == OS::Paused || s == OS::FastForward || s == OS::Rewind
                    }) {
                        Some(s.track as usize)
                    } else {
                        None
                    }
                } else {
                    None
                };

                self.track_listing_table.table(ui, c, playing_track, &mut self.md_channel);
            }
        });

        ctx.request_repaint();
    }
}

impl MinidiscManager {
    fn connect_to_device(&mut self) {
        let state = Arc::new(RwLock::new(PlayerState::default()));
        let (send, recv) = mpsc::channel();

        let thread_state = Arc::clone(&state);
        std::thread::spawn(|| {
            future::block_on(async { MinidiscThread::minidisc_thread(thread_state, recv).await });
        });

        self.md_channel = Some(send);
        self.md_state = state;
    }
}

#[derive(Default)]
struct TrackListingTable {
}

impl TrackListingTable {
    fn table(&mut self, ui: &mut egui::Ui, disc: &Disc, playing: Option<usize>, channel: &mut Option<mpsc::Sender<PlayerCommand>>) {
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
                ui.strong("");
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

                if playing.is_some_and(|p| p == row.index()) {
                    row.set_selected(true);
                }

                row.col(|ui| {
                    ui.label((row_track.index() + 1).to_string());
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(title).truncate());
                });
                row.col(|ui| {
                    ui.label(row_track.encoding().to_string().to_ascii_uppercase());
                });
                row.col(|ui| {
                    ui.label(pretty_duration(row_track.duration().as_duration()));
                });
                row.col(|ui| {
                    ui.label(" ");
                });

                if let Some(ch) = channel {
                    if row.response().double_clicked() {
                        let _ = ch.send(PlayerCommand::GoToTrack(row.index()));
                    }

                    row.response().context_menu(|ui| {
                        if ui.small_button("Delete").clicked() {
                            let _ = ch.send(PlayerCommand::Delete(row.index()));
                        }
                    });
                }
            });
        });
    }
}


fn pretty_duration(duration: Duration) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        duration.as_secs() / 3600,
        (duration.as_secs() / 60) % 60,
        duration.as_secs() % 60,
    )
}

#[derive(Default)]
struct PlayerState {
    connected: bool,
    reading: bool,

    disc_contents: Option<Disc>,
    device_state: Option<DeviceStatus>,
    progress: Option<f32>,
}

enum PlayerCommand {
    Disconnect,
    Playback(Action),
    Stop,
    SkipTrack(Direction),
    GoToTrack(usize),
    Upload(PathBuf),
    Delete(usize),
}

struct MinidiscThread {
    device: NetMDContext,
    state: Arc<RwLock<PlayerState>>,
    recv: mpsc::Receiver<PlayerCommand>,
}

impl MinidiscThread {
    async fn minidisc_thread(
        comm: Arc<RwLock<PlayerState>>,
        recv: mpsc::Receiver<PlayerCommand>,
    ) {
        let usb_dev = cross_usb::get_device(DEVICE_IDS_CROSSUSB.to_vec()).await.unwrap();
        let md_dev = minidisc::netmd::NetMDContext::new(usb_dev).await.unwrap();

        let mut new_self = Self {
            device: md_dev,
            state: comm,
            recv,
        };

        new_self.state.write().unwrap().connected = true;
        println!("Connected to {:?}", new_self.device.interface().device.device_name());

        match new_self.command_loop().await {
            Ok(_) => (),
            Err(e) => println!("A fatal error occurred: {e}"),
        }

        *new_self.state.write().unwrap() = PlayerState::default();
    }

    async fn get_contents(&mut self) -> Result<()> {
        self.state.write().unwrap().reading = true;
        self.state.write().unwrap().disc_contents = Some(self.device.list_content().await?);
        self.state.write().unwrap().reading = false;

        Ok(())
    }

    async fn command_loop(&mut self) -> Result<()> {
        self.state.write().unwrap().device_state = Some(self.device.device_status().await?);
        self.get_contents().await?;

        let mut state_timer = Instant::now();
        loop {
            if let Ok(r) = self.recv.try_recv() {
                match r {
                    PlayerCommand::Disconnect => break,
                    PlayerCommand::Playback(action) => {
                        self.device.interface_mut().playback_control(action).await?;
                    },
                    PlayerCommand::SkipTrack(direction) => {
                        self.device.interface_mut().track_change(direction).await?;
                    },
                    PlayerCommand::GoToTrack(track) => {
                        self.device.interface_mut().go_to_track(track as u16).await?;
                        self.device.interface_mut().playback_control(Action::Play).await?;
                    },
                    PlayerCommand::Stop => {
                        self.device.interface_mut().stop().await?;
                    }
                    PlayerCommand::Upload(path) => {
                        let track_contents: Vec<u8> = std::fs::read(path).unwrap().to_vec();
                        let track = MDTrack {
                            chunk_size: 0x400,
                            title: String::from("TestTrack"),
                            full_width_title: None,
                            format: minidisc::netmd::interface::WireFormat::LP4,
                            data: track_contents,
                        };
                        self.device.interface_mut().stop().await?;

                        let player_state_thread = Arc::clone(&self.state);
                        self.device.download(track, |out_of: usize, done: usize| {
                            player_state_thread.write().unwrap().progress = Some(done as f32/out_of as f32)
                        }).await?;
                        self.state.write().unwrap().progress = None;
                        self.get_contents().await?;
                    }
                    PlayerCommand::Delete(track) => {
                        self.state.write().unwrap().reading = true;
                        self.device.interface_mut().stop().await?;
                        self.device.interface_mut().erase_track(track as u16).await?;
                        self.get_contents().await?;
                    }
                }
            }

            // Check for an updated device state
            if state_timer.elapsed() >= Duration::from_millis(500) {
                let state = self.device.device_status().await?;

                self.state.write().unwrap().device_state = Some(state);

                let contents_present = self.state.read().unwrap().disc_contents.is_some();

                if contents_present && !state.disc_present {
                    self.state.write().unwrap().disc_contents = None;
                } else if !contents_present && state.disc_present {
                    self.get_contents().await?;
                }

                state_timer = Instant::now();
            }

            sleep(Duration::from_millis(50));
        }

        Ok(())
    }
}
