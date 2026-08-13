//! Оконный трекер claude-wt для macOS.
//!
//! Окна у приложения нет — только значок в трее. Работа идёт в отдельном
//! потоке: у Tauri главный поток занят циклом событий, и такт трекера, встав в
//! него, отнял бы у меню отзывчивость.

mod ax;
mod deliver;
mod dump;

use mwm_core::config::{config_path, parse_config, Config};
use mwm_core::publish::{build_file, fingerprint, should_write};
use mwm_core::tracker::Tracker;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;

/// Что показывать человеку в трее. Английский — правило проекта: всё, что
/// видит человек, по-английски.
#[derive(Clone)]
struct Status(Arc<Mutex<String>>);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn load_config() -> Config {
    let home = std::env::var("HOME").unwrap_or_default();
    let text = std::fs::read_to_string(config_path(&home)).unwrap_or_default();
    let hostname = std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    parse_config(&text, &hostname)
}

/// Такт трекера.
///
/// Разрешение проверяется на каждом обороте, а не однажды при старте: человек
/// выдаёт его в System Settings уже после запуска, и приложение обязано
/// заработать без перезапуска.
///
/// Без разрешения файл не пишется вовсе. Пустой файл означал бы «окон нет» и
/// погасил бы чужие пометки; прежний протухнет у читателя сам, и это правда.
fn run_tracker(status: Status) {
    let cfg = load_config();
    let mut tracker = Tracker::new(2);
    let mut registry = ax::Registry::default();
    let mut cache = dump::Cache::default();
    let mut last_print: Option<String> = None;
    let mut last_write_ms = 0u64;
    let pid = std::process::id();
    loop {
        std::thread::sleep(Duration::from_millis(cfg.tick_ms));
        if !ax::trusted() {
            *status.0.lock().unwrap() = "Accessibility not granted".to_string();
            continue;
        }
        let now = now_ms();
        let seen = ax::list_windows(&mut registry, &cfg.terminals);
        // Список незнакомых заголовков — с прошлого такта, и это правильно:
        // за дампом идут до тика, а узнают о незнакомом заголовке из него.
        // Отставание в один такт стоит секунды, а спрашивать дважды за оборот
        // стоило бы второго ssh.
        let wanted = !tracker.unresolved().is_empty();
        let index = cache.get(&cfg, now, wanted).clone();
        tracker.tick(&seen, &index, now);
        let bound = tracker.bound();
        let print = fingerprint(&bound);
        if should_write(&print, last_print.as_deref(), last_write_ms, now) {
            let payload = build_file(&bound, &cfg.host, pid, now, false);
            match deliver::send(&cfg, &payload) {
                Ok(()) => {
                    last_print = Some(print);
                    last_write_ms = now;
                    *status.0.lock().unwrap() = format!("{} windows tracked", bound.len());
                }
                // Ничего не копится: следующая посылка везёт текущее состояние
                // целиком, а протухший файл читатель отбрасывает сам.
                Err(e) => *status.0.lock().unwrap() = format!("publish failed: {e}"),
            }
        }
    }
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let status = Status(Arc::new(Mutex::new("starting…".to_string())));
            let state = MenuItem::with_id(app, "status", "starting…", false, None::<&str>)?;
            let grant = MenuItem::with_id(app, "grant", "Grant Accessibility…", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&state, &grant, &quit])?;
            TrayIconBuilder::new()
                .menu(&menu)
                .icon(app.default_window_icon().cloned().unwrap())
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "grant" => ax::prompt_for_trust(),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            let worker = status.clone();
            std::thread::spawn(move || run_tracker(worker));

            // Строка состояния обновляется своим тиком: лезть в меню из потока
            // трекера нельзя — пункты меню живут на главном потоке.
            let painter = status.clone();
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(2));
                let text = painter.0.lock().unwrap().clone();
                let _ = handle.run_on_main_thread({
                    let state = state.clone();
                    move || { let _ = state.set_text(&text); }
                });
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
