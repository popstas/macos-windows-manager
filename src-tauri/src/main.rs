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
    // Слоты с прошлого запуска — до первого такта: иначе первое же окно
    // завело бы слот заново, и запомненное место было бы потеряно ровно тогда,
    // когда оно нужно.
    let state_path = std::path::PathBuf::from(&cfg.state_path);
    tracker.load_slots(mwm_core::state::read_state(&state_path));
    let snapshots_path = std::path::PathBuf::from(&cfg.snapshots_path);
    let mut snaps = load_snapshots(&snapshots_path);
    let mut pending_key = String::new();
    let mut pending_since_ms = 0u64;
    let mut registry = ax::Registry::default();
    let mut cache = dump::Cache::default();
    // Сказали ли уже про отсутствующее разрешение. Живёт до конца жизни
    // процесса и сбрасывается, когда разрешение появилось.
    let mut told_untrusted = false;
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
            // Один раз на потерю разрешения, а не каждый такт: жалоба длинная,
            // а такт секундный — в логе она вытеснила бы всё остальное за
            // минуту. Латч сбрасывается ниже, когда разрешение снова есть, так
            // что о повторной потере человек узнает снова.
            if !told_untrusted {
                let exe = std::env::current_exe().ok();
                for line in mwm_core::permissions::accessibility_missing(exe.as_deref()).lines() {
                    eprintln!("mwm: {line}");
                }
                told_untrusted = true;
            }
            continue;
        }
        told_untrusted = false;
        let now = now_ms();
        let seen = ax::list_windows(&mut registry, &cfg.terminals);
        // Список незнакомых заголовков — с прошлого такта, и это правильно:
        // за дампом идут до тика, а узнают о незнакомом заголовке из него.
        // Отставание в один такт стоит секунды, а спрашивать дважды за оборот
        // стоило бы второго ssh.
        let wanted = !tracker.unresolved().is_empty();
        let index = cache.get(&cfg, now, wanted).clone();
        tracker.tick(&seen, &index, now);
        // Расстановка — здесь и только здесь: реестр окон не `Send` и живёт в
        // этом потоке. Клампинг считается в момент расстановки, а не при
        // запоминании: экраны могли смениться, пока сессия была закрыта.
        let screens = ax::displays();
        for (window_id, want) in tracker.placements() {
            let target = mwm_core::geometry::clamp_to_displays(want, &screens);
            if let Err(e) = ax::place(&registry, window_id, target) {
                // Молчать нельзя: «поставил» и «не смог» отличаются только этим.
                eprintln!("mwm: place failed: {e}");
                *status.0.lock().unwrap() = format!("place failed: {e}");
            }
        }
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
        // Снимок раскладки. Клон карты слотов (`tracker.slots_state()`) и
        // список сессий (`sessions_of`) строятся каждый такт безусловно, а не
        // только внутри ветки `decide` ниже, — ей самой нужен состав, чтобы
        // решить, есть ли что писать. При секундном такте цена пренебрежима:
        // слотов мало, а копия живёт один проход и тут же выбрасывается.
        //
        // Состав для ключа берётся из того же `sessions_of`, что соберёт и
        // сам снимок, — а не из `tracker.open_session_ids()` отдельно.
        // `sessions_of` пропускает привязанную сессию без координат (это её
        // документированная норма), и раньше `key` считал её, а снимок — нет:
        // при такой сессии `key` навсегда расходился с `last_key`
        // (посчитанным по уже сохранённому снимку, где её тоже нет), и
        // `decide` выдавал `Append` на каждом такте — двадцать снимков за
        // двадцать секунд. Общий источник закрывает это по построению: что
        // решило `decide`, то и уедет на диск.
        let open = tracker.open_session_ids();
        let sessions = mwm_core::snapshots::sessions_of(&open, &tracker.slots_state());
        let key = mwm_core::snapshots::composition_key(
            &sessions.iter().map(|s| s.id.clone()).collect::<Vec<_>>(),
        );
        let last_key = snaps
            .first()
            .map(|s| {
                mwm_core::snapshots::composition_key(
                    &s.sessions.iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
                )
            })
            .unwrap_or_default();
        let decision = mwm_core::snapshots::decide(
            &key, &last_key, &pending_key, pending_since_ms, now, cfg.snapshots_debounce_ms,
        );
        (pending_key, pending_since_ms) =
            mwm_core::snapshots::track_composition(&key, &pending_key, pending_since_ms, now);
        if let Some(d) = decision {
            // Совпавший состав и координаты — не повод писать файл. `decide`
            // отдаёт `Update` на каждом такте, пока состав не изменился, вне
            // зависимости от того, сдвинулась ли хоть одна координата; сама
            // запись — плата, а не решение, и её стоит нести, только когда
            // содержимое действительно другое. Та же защита, что `take_dirty`
            // даёт `state.json` строкой выше.
            let unchanged = d == mwm_core::snapshots::Decision::Update
                && snaps.first().map(|s| &s.sessions) == Some(&sessions);
            if !sessions.is_empty() && !unchanged {
                snaps = match d {
                    mwm_core::snapshots::Decision::Append => mwm_core::snapshots::append(
                        std::mem::take(&mut snaps),
                        mwm_core::snapshots::snapshot_id(now / 1000),
                        sessions,
                        now / 1000,
                        cfg.snapshots_keep,
                    ),
                    mwm_core::snapshots::Decision::Update => mwm_core::snapshots::update_last(
                        std::mem::take(&mut snaps),
                        sessions,
                        now / 1000,
                    ),
                };
                save_snapshots(&snapshots_path, &snaps);
            }
        }

        let print = fingerprint(&bound, link.is_live());

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

        // Файл состояния пишется с `fsync`, и писать его на каждом такте —
        // плата за то, что не изменилось.
        if tracker.take_dirty() {
            if let Err(e) = mwm_core::state::write_atomic(
                &state_path,
                &mwm_core::state::state_json(&tracker.slots_state()),
            ) {
                eprintln!("mwm: state write failed: {e}");
            }
        }

        if should_write(&print, last_print.as_deref(), last_write_ms, now) {
            // Снимки едут в build_file, но в fingerprint не входят: тот их не
            // видит, и should_write не отличит «появился снимок» от «ничего не
            // изменилось». Решение — оставить как есть: HEARTBEAT_MS (полминуты)
            // перепишет файл не позже чем через полминуты, а снимок, чтобы
            // родиться, отстоял минуту дебаунса, — опоздание вдвое меньше того,
            // что уже потрачено на его рождение.
            let payload =
                build_file(&bound, &cfg.host, pid, now, link.is_live(), &cfg.mqtt.base, &snaps);
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

/// Снимки с диска. Формат — тот же, что уезжает в файле окон, и разбирается он
/// здесь же: заводить ради него отдельный модуль значило бы держать два места,
/// где знают одну структуру.
fn load_snapshots(path: &std::path::Path) -> Vec<mwm_core::snapshots::Snapshot> {
    use mwm_core::geometry::Bounds;
    use mwm_core::snapshots::{Snapshot, SnapshotSession};
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        eprintln!("mwm: broken snapshots file, starting empty");
        return Vec::new();
    };
    let mut out = Vec::new();
    for s in v.get("snapshots").and_then(|x| x.as_array()).into_iter().flatten() {
        let Some(id) = s.get("id").and_then(|x| x.as_str()).filter(|x| !x.is_empty()) else {
            continue;
        };
        let num = |k: &str| s.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        let mut sessions = Vec::new();
        for m in s.get("sessions").and_then(|x| x.as_array()).into_iter().flatten() {
            let Some(sid) = m.get("id").and_then(|x| x.as_str()).filter(|x| !x.is_empty()) else {
                continue;
            };
            let b = m.get("bounds").and_then(|x| x.as_object());
            let n = |k: &str| {
                b.and_then(|o| o.get(k)).and_then(|x| x.as_i64()).and_then(|x| i32::try_from(x).ok())
            };
            let (Some(x), Some(y), Some(width), Some(height)) =
                (n("x"), n("y"), n("width"), n("height"))
            else {
                continue;
            };
            let text = |k: &str| {
                m.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string()
            };
            sessions.push(SnapshotSession {
                id: sid.to_string(),
                title: text("title"),
                cwd: text("cwd"),
                bounds: Bounds { x, y, width, height },
            });
        }
        out.push(Snapshot {
            id: id.to_string(),
            created_s: num("created"),
            updated_s: num("updated"),
            sessions,
        });
    }
    out
}

fn save_snapshots(path: &std::path::Path, snaps: &[mwm_core::snapshots::Snapshot]) {
    let value = serde_json::json!({
        "version": 1,
        "snapshots": snaps.iter().map(|s| serde_json::json!({
            "id": s.id,
            "created": s.created_s,
            "updated": s.updated_s,
            "sessions": s.sessions.iter().map(|m| serde_json::json!({
                "id": m.id,
                "title": m.title,
                "cwd": m.cwd,
                "bounds": {
                    "x": m.bounds.x, "y": m.bounds.y,
                    "width": m.bounds.width, "height": m.bounds.height
                },
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    if let Err(e) = mwm_core::state::write_atomic(path, &value) {
        eprintln!("mwm: snapshots write failed: {e}");
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
