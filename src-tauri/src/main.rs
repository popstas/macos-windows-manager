//! Оконный трекер claude-wt для macOS.
//!
//! Окна у приложения нет — только значок в трее. Работа идёт в отдельном
//! потоке: у Tauri главный поток занят циклом событий, и такт трекера, встав в
//! него, отнял бы у меню отзывчивость.

mod ax;
mod deliver;
mod dump;
mod mqtt;

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
    let link = mqtt::spawn(&cfg.mqtt);
    // Номер окна каждой сессии на прошлом такте: подъёму нужно окно, а `bound`
    // рассказывает про сессии. Держим рядом, потому что `Registry` знает
    // номера, а не сессии, и связать их может только тот, кто видел оба списка.
    let mut window_of: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut last_print: Option<String> = None;
    let mut last_write_ms = 0u64;
    let pid = std::process::id();
    loop {
        // Ожидание вместо сна: просьба исполняется сразу, а не к следующему
        // такту. Отсоединение канала (поток подписки умер) — тот же сон:
        // `recv_timeout` на закрытом канале возвращается мгновенно, и без
        // этой ветки такт превратился бы в горячий цикл.
        match link.requests.recv_timeout(Duration::from_millis(cfg.tick_ms)) {
            Ok(req) => {
                let mut pending = vec![req];
                // Разгребается вся очередь, а не одна просьба: иначе каждая
                // стоила бы полного такта с перечислением окон и, возможно,
                // походом за дампом по ssh.
                while let Ok(next) = link.requests.try_recv() {
                    pending.push(next);
                }
                for req in pending {
                    let note = serve(&req, &mut tracker, &registry, &window_of);
                    if let Some(note) = note {
                        eprintln!("mwm: {note}");
                        *status.0.lock().unwrap() = note;
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                std::thread::sleep(Duration::from_millis(cfg.tick_ms));
            }
        }
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
        // Сессия ↔ окно. Заголовок — единственное, что есть у обоих списков:
        // `Seen` знает номер окна, `Bound` — сессию, и оба знают, как окно
        // называется. У `Bound` заголовок уже очищен от значка состояния,
        // поэтому и здесь он чистится перед сравнением.
        window_of.clear();
        for (sid, b) in &bound {
            if let Some(w) = seen
                .iter()
                .find(|w| mwm_core::title::strip_decoration(&w.title) == b.title)
            {
                window_of.insert(sid.clone(), w.id);
            }
        }
        let print = fingerprint(&bound);

        // Ошибка чтения дампа и ошибка записи файла окон — про разные машины
        // и разные починки, и одна не должна прятать другую. Без этой строки
        // трекер с нечитаемым дампом выглядел бы неотличимо от трекера, у
        // которого просто нет сессий для привязки: публикация могла бы идти
        // исправно (`deliver::send` про дамп ничего не знает), а привязки —
        // не случаться никогда, молча.
        let fetch_note = cache
            .last_error
            .as_deref()
            .map(|e| format!("; index fetch failed: {e}"));

        if should_write(&print, last_print.as_deref(), last_write_ms, now) {
            let payload = build_file(&bound, &cfg.host, pid, now, link.is_live(), &cfg.mqtt.base);
            match deliver::send(&cfg, &payload) {
                Ok(()) => {
                    last_print = Some(print);
                    last_write_ms = now;
                    let base = format!("{} windows tracked", bound.len());
                    *status.0.lock().unwrap() =
                        fetch_note.as_deref().map_or_else(|| base.clone(), |n| format!("{base}{n}"));
                }
                // Ничего не копится: следующая посылка везёт текущее состояние
                // целиком, а протухший файл читатель отбрасывает сам.
                Err(e) => {
                    let base = format!("publish failed: {e}");
                    *status.0.lock().unwrap() =
                        fetch_note.as_deref().map_or_else(|| base.clone(), |n| format!("{base}{n}"));
                }
            }
        } else if let Some(note) = &fetch_note {
            // Расклад не менялся, публикации в этом такте не было — но
            // ошибка чтения дампа не должна ждать следующей записи файла,
            // до неё может быть далеко (или не наступить вовсе, пока
            // отпечаток не сдвинется).
            *status.0.lock().unwrap() = format!("{} windows tracked{note}", bound.len());
        }
    }
}

/// Исполнить просьбу. Возвращает жалобу, если исполнить не вышло.
///
/// Отказ виден человеку — строкой в трее и в stderr, — но не пикеру: у
/// публикации нет ответа, и заводить его ради одного мака значило бы разойтись
/// с Windows-веткой. Цена известна и уплачена ещё там.
///
/// Английский в жалобе — правило проекта: её видит человек.
fn serve(
    req: &mwm_core::request::Request,
    tracker: &mut Tracker,
    registry: &ax::Registry,
    window_of: &std::collections::HashMap<String, u64>,
) -> Option<String> {
    use mwm_core::request::Request;
    match req {
        Request::Focus(id) => {
            // Просьба о сессии без живого окна сюда не приходит: пикер
            // предлагает подъём только строкам с полем `window`. Но гонка
            // возможна — окно закрыли между опросом и нажатием.
            let Some(window_id) = window_of.get(id) else {
                return Some(format!("focus: no window for session {id}"));
            };
            ax::raise(registry, *window_id).err().map(|e| format!("focus: {e}"))
        }
        Request::Unread(id) => {
            tracker.mark_unread(id);
            None
        }
    }
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            // Окон у приложения нет, значит и месту в доке взяться неоткуда.
            // Без этой строки macOS считает процесс обычным приложением и
            // держит значок в доке рядом со значком в строке меню — два места
            // для одного трея. `Accessory` — это то же, что `LSUIElement` в
            // Info.plist, но у нас голый бинарь без бандла, и plist'а нет.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let status = Status(Arc::new(Mutex::new("starting…".to_string())));
            let state = MenuItem::with_id(app, "status", "starting…", false, None::<&str>)?;
            let grant = MenuItem::with_id(app, "grant", "Grant Accessibility…", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&state, &grant, &quit])?;
            // Значок трея заводится только здесь. Объявление `app.trayIcon` в
            // `tauri.conf.json` завело бы второй — Tauri создаёт его сам при
            // старте, и меню у него нет: в строке меню было видно два значка,
            // один рабочий, другой немой.
            TrayIconBuilder::new()
                .menu(&menu)
                // Шаблонный значок macOS перекрашивает под строку меню, читая
                // из картинки только прозрачность. Без него белые буквы «WM»
                // пропадали бы на светлой теме.
                .icon_as_template(true)
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
