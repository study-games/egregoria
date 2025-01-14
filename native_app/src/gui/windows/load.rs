#![allow(unused)]
use crate::uiworld::{SaveLoadState, UiWorld};
use egregoria::Egregoria;
use egui::{Color32, DroppedFile, Widget};
use std::path::PathBuf;

#[derive(Default)]
pub struct LoadState {
    curpath: Option<PathBuf>,
    load_fail: String,
}

/// Load window
/// Allows to load a replay from disk and play it
pub fn load(window: egui::Window<'_>, ui: &egui::Context, uiw: &mut UiWorld, _: &Egregoria) {
    window.show(ui, |ui| {
        let mut lstate = uiw.write::<LoadState>();

        let has_save = ui.data_mut(|d| {
            *d.get_persisted_mut_or_insert_with(ui.make_persistent_id("has_save"), || {
                std::fs::metadata("world/world_replay.json").is_ok()
            })
        });

        if has_save {
            if ui.button("Load world/world_replay.json").clicked() {
                let replay = Egregoria::load_replay_from_disk("world");

                if let Some(replay) = replay {
                    let (goria, loader) = Egregoria::from_replay(replay);
                    uiw.write::<SaveLoadState>().please_load = Some(loader);
                    uiw.write::<SaveLoadState>().please_load_goria = Some(goria);
                } else {
                    lstate.load_fail = "Failed to load replay".to_string();
                }
            }
        } else {
            ui.label("No replay found in world/world_replay.json");
        }

        if let Some(ref mut loading) = uiw.write::<SaveLoadState>().please_load {
            let ticks_done = loading.pastt.0;
            let ticks_total = loading.replay.commands.last().map(|c| c.0 .0).unwrap_or(0);
            egui::ProgressBar::new((ticks_done as f32) / (ticks_total as f32))
                .text(format!("Loading replay: {ticks_done}/{ticks_total}"))
                .ui(ui);
            ui.horizontal(|ui| {
                if ui.button("||").clicked() {
                    loading.speed = 0;
                }
                if ui
                    .button(">")
                    .on_hover_text("Load the replay faster")
                    .clicked()
                {
                    loading.speed = 1;
                }
                if ui
                    .button(">>>")
                    .on_hover_text("Load the replay faster")
                    .clicked()
                {
                    loading.speed = 100;
                }
                if ui.button("1").clicked() {
                    loading.speed = 0;
                    loading.advance_n_ticks = 1;
                }
                if ui.button("10").clicked() {
                    loading.speed = 0;
                    loading.advance_n_ticks = 10;
                }
                if ui.button("100").clicked() {
                    loading.speed = 0;
                    loading.advance_n_ticks = 100;
                }
                if ui.button("1000").clicked() {
                    loading.speed = 0;
                    loading.advance_n_ticks = 1000;
                }
            });
        }

        if !lstate.load_fail.is_empty() {
            ui.colored_label(Color32::RED, &lstate.load_fail);
        }
    });
}
