//! hyt-gui: read a Hytera radio (or open a saved read), view and edit its settings, save them and
//! write them back. All logic lives in `hyt-core`; this crate is the Tauri shell and the `ui/`
//! web page.
//!
//! Usage: `hyt-gui [folder]` opens a folder saved by `hyt read` at startup. Hytera's data is
//! loaded like the CLI does: `HYT_RCDB` (default: the CPS install) and `HYT_OPTIONS` (default:
//! the per-user option catalogue). `HYT_GUI_DEV=1` additionally offers replaying recorded CPS
//! sessions instead of a serial port, and `HYT_GUI_SIM=<saved folder>` a simulated radio holding
//! that folder's configuration (it restarts briefly after a write, like the BP515), for testing
//! without a radio.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use std::path::PathBuf;

use hytera_codeplug::rcdb::DEFAULT_CPS_DATA_DIR;

/// The simulated radio from `HYT_GUI_SIM`, if set and loadable.
fn simulated_radio() -> Option<(hytera_proto::sim::SharedSim, String)> {
    let folder = std::env::var_os("HYT_GUI_SIM")?;
    match hyt_core::load_read(std::path::Path::new(&folder)) {
        Ok(saved) => {
            let mut radio = hyt_core::sim_radio(&saved);
            radio.faults.restart_requests = 2;
            Some((hytera_proto::sim::SharedSim::new(radio), folder.to_string_lossy().into_owned()))
        }
        Err(e) => {
            tracing::warn!("HYT_GUI_SIM: {e:#}");
            None
        }
    }
}

fn main() {
    tracing_subscriber::fmt().with_max_level(tracing::Level::WARN).with_writer(std::io::stderr).init();
    let state = commands::AppState::new(
        std::env::var_os("HYT_RCDB").map_or_else(|| PathBuf::from(DEFAULT_CPS_DATA_DIR), PathBuf::from),
        std::env::var_os("HYT_OPTIONS").map(PathBuf::from),
        std::env::args_os().nth(1).map(PathBuf::from),
        std::env::var("HYT_GUI_DEV").is_ok_and(|v| v == "1"),
        simulated_radio(),
    );
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::list_ports,
            commands::open_folder,
            commands::get_overview,
            commands::get_table,
            commands::get_detail,
            commands::set_value,
            commands::revert_value,
            commands::discard_changes,
            commands::get_changes,
            commands::add_channel,
            commands::remove_channel,
            commands::get_channel_options,
            commands::add_zone_member,
            commands::remove_zone_member,
            commands::move_zone_member,
            commands::save_as,
            commands::suggest_folder,
            commands::read_radio,
            commands::write_radio,
        ])
        .run(tauri::generate_context!())
        .expect("error while running hyt-gui");
}
