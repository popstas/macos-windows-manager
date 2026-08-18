//! Оконный трекер claude-wt для macOS.
//!
//! Окна у приложения нет — только значок в трее. Работа идёт в отдельном
//! потоке: у Tauri главный поток занят циклом событий, и такт трекера, встав в
//! него, отнял бы у меню отзывчивость.

mod ax;
mod config_file;
mod deliver;
mod dump;
mod mqtt;

use mwm_core::config::{config_path, parse_config, Config};
use mwm_core::mwm_log;
use mwm_core::publish::{build_file, fingerprint, should_write};
use mwm_core::tracker::Tracker;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::menu::{Menu, MenuItem};
// Ради `app.manage`: ячейка настроек кладётся в состояние приложения, оттуда
// её берут команды окна.
use tauri::Manager;
use tauri::tray::TrayIconBuilder;

/// Что показывать человеку в трее. Английский — правило проекта: всё, что
/// видит человек, по-английски.
#[derive(Clone)]
struct Status(Arc<Mutex<String>>);

/// Заголовки ведомых окон — по пункту меню на окно.
///
/// Отдельная ячейка, а не разбор строки состояния: строка — одна фраза для
/// человека и меняется целиком (отказ публикации переписывает её всю), а
/// список окон обязан пережить такую подмену. Пишет её трекер, читает поток,
/// рисующий меню, — тем же порядком, что и `Status`.
#[derive(Clone)]
struct Windows(Arc<Mutex<Vec<String>>>);

/// Рабочая область главного экрана, как её видит главный поток.
///
/// Отдельная ячейка ровно затем, что спросить её может только главный поток
/// (`NSScreen` — API AppKit), а нужна она тому, который двигает окна. Обновляет
/// её рисовальщик меню — он и так заходит на главный поток каждые две секунды,
/// а Dock переезжает реже.
#[derive(Clone)]
struct Work(Arc<Mutex<Option<mwm_core::geometry::Bounds>>>);

/// Куда пункт меню кладёт просьбу.
///
/// Двигать окна из главного потока нельзя: реестр `AXUIElement` не `Send` и
/// живёт в потоке трекера. Поэтому клик по пункту не расставляет ничего сам, а
/// кладёт в тот же канал ту же просьбу, что приезжает по MQTT, — и оба входа
/// сходятся в одной ветке исполнения.
#[derive(Clone)]
struct Asking(Arc<Mutex<std::sync::mpsc::Sender<mwm_core::request::Request>>>);

/// Есть ли сейчас разрешение Accessibility.
///
/// Отвечает на этот вопрос трекер — он спрашивает систему каждый такт, — а
/// читает поток, рисующий меню. Свой второй вызов `ax::trusted()` он мог бы
/// сделать и сам, но тогда строка состояния и пункт «выдать разрешение»
/// отвечали бы на один вопрос, спрошенный в разные моменты, и расходились бы
/// между собой.
#[derive(Clone)]
struct Trusted(Arc<AtomicBool>);

/// Действующие настройки.
///
/// Ячейка, а не копия при старте: окно настроек пишет файл на ходу, и такт
/// обязан узнать об этом со следующего оборота. Читает её трекер, пишет —
/// команда сохранения.
#[derive(Clone)]
struct Shared(Arc<Mutex<Config>>);

impl Shared {
    /// Снимок настроек на один оборот такта. Копия, а не блокировка на весь
    /// оборот: держать мьютекс, пока идёт ssh за дампом, значило бы подвесить
    /// на это время окно настроек.
    fn get(&self) -> Config {
        self.0.lock().unwrap().clone()
    }
}

/// Нарисованный сейчас список окон: подписи и сами пункты меню.
///
/// Подписи хранятся рядом с пунктами, а не считываются с них обратно: сравнить
/// с новым списком нужно раньше, чем что-то трогать в меню, а пункты об этом
/// не спрашивают.
type Drawn = Arc<Mutex<(Vec<String>, Vec<MenuItem<tauri::Wry>>)>>;

/// Сколько пунктов раскладки стоит под `GRANT_POSITION`.
///
/// Список окон вставляется по номеру, и номер этот считается от места, где
/// кончились пункты с постоянным местом. Забудь про них счёт — список встал бы
/// поверх кнопок.
const LAYOUT_ITEMS: usize = 2;

/// Куда возвращать пункт «выдать разрешение», когда разрешение пропало.
///
/// Второй сверху: под строкой состояния, над `Quit`. Место у него постоянное —
/// пункт, который приходит и уходит, не должен ещё и прыгать по меню.
const GRANT_POSITION: usize = 1;

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

/// Путь к config.yaml. Общий для чтения и записи: разойдись они двумя копиями,
/// один читал бы не тот файл, что другой пишет.
fn config_file_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(config_path(&home))
}

/// Патч, обнуляющий ключ, отклоняется явно — на любой глубине.
///
/// `merge_patch` вложенные отображения сливает по ключам, а `null` считает
/// «не отображением» и подменяет блок целиком — так пропал бы `mqtt.password`,
/// которого форма настроек никогда не загружает и не присылает обратно.
/// Сегодняшняя форма такого патча не пришлёт, но если пришлёт когда-нибудь —
/// лучше внятный отказ здесь, чем молча стёртый пароль.
///
/// Проверка рекурсивная именно из-за пароля: страшен не столько `mqtt: null`
/// на верхнем уровне, сколько `{"mqtt": {"password": null}}` — тот стирает
/// ровно тот ключ, ради которого всё слияние и написано.
fn reject_null_values(patch: &serde_json::Value) -> Result<(), String> {
    if let Some(fields) = patch.as_object() {
        for (key, value) in fields {
            if value.is_null() {
                return Err(format!(
                    "patch cannot null out key {key}: null would replace the whole block and wipe what the form did not send (mqtt.password, for one)"
                ));
            }
            reject_null_values(value)?;
        }
    }
    Ok(())
}

/// Закрыть файл от всех, кроме владельца.
///
/// В конфиге лежит `mqtt.password`, а заводит файл теперь не человек своими
/// руками, а окно настроек: с обычным umask пароль оказался бы читаем всей
/// машине. Отказ не фатален — сохранить настройки важнее, чем выставить
/// режим, — но и молчать о нём нельзя.
#[cfg(unix)]
fn restrict_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        mwm_log!("cannot restrict {}: {e}", path.display());
    }
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &std::path::Path) {}

/// Слить патч в config.yaml на диске.
///
/// Отдельно от команды сохранения: чистая файловая операция без `AppHandle`,
/// её можно накрыть тестами во временном каталоге, не поднимая Tauri.
///
/// Бэкап кладётся один раз, перед первой перезаписью: комментарии человека
/// после неё не восстановить ничем, а класть `.bak` на каждое сохранение
/// значило бы затирать его же вчерашним состоянием.
fn write_config(path: &std::path::Path, patch: &serde_json::Value) -> Result<(), String> {
    reject_null_values(patch)?;

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let existing = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };

    let mut doc: serde_yaml::Value = if existing.trim().is_empty() {
        serde_yaml::Value::Null
    } else {
        serde_yaml::from_str(&existing).map_err(|e| format!("bad yaml in {}: {e}", path.display()))?
    };
    config_file::merge_patch(&mut doc, patch)?;

    // Тем же условием, что и разбор чуть выше: файл из одних пробелов и
    // переводов строки тоже «был пустым», и бэкапить в нём нечего — иначе он
    // занял бы единственный слот `.bak` навсегда.
    let backup = path.with_extension("yaml.bak");
    if !existing.trim().is_empty() && !backup.exists() {
        std::fs::write(&backup, &existing)
            .map_err(|e| format!("cannot write {}: {e}", backup.display()))?;
        restrict_permissions(&backup);
    }

    // Через временный файл и переименование, как пишется state.json: читатель
    // никогда не видит половину файла.
    let text = format!("{}{}", config_file::HEADER, config_file::render(&doc)?);
    let tmp = path.with_extension("yaml.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    // До переименования, а не после: иначе у конфига был бы промежуток, в
    // который пароль уже на месте, а права ещё общие.
    restrict_permissions(&tmp);
    if let Err(e) = std::fs::rename(&tmp, path) {
        // Rename не удался — не оставлять временный файл валяться на диске.
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("cannot rename onto {}: {e}", path.display()));
    }
    Ok(())
}

/// Такт трекера.
///
/// Разрешение проверяется на каждом обороте, а не однажды при старте: человек
/// выдаёт его в System Settings уже после запуска, и приложение обязано
/// заработать без перезапуска.
///
/// Без разрешения файл не пишется вовсе. Пустой файл означал бы «окон нет» и
/// погасил бы чужие пометки; прежний протухнет у читателя сам, и это правда.
fn run_tracker(
    status: Status,
    trusted: Trusted,
    shared: Shared,
    windows: Windows,
    requests: std::sync::mpsc::Receiver<mwm_core::request::Request>,
    to_requests: std::sync::mpsc::Sender<mwm_core::request::Request>,
    work: Work,
) {
    // Копия при старте — для того, что на лету не меняется: путей состояния и
    // потока подписки. Всё, что читается каждый оборот, берётся из ячейки
    // внутри цикла.
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
    let link = mqtt::spawn(&cfg.mqtt, to_requests);
    // Номер окна каждой сессии на прошлом такте: подъёму нужно окно, а `bound`
    // рассказывает про сессии. Держим рядом, потому что `Registry` знает
    // номера, а не сессии, и связать их может только тот, кто видел оба списка.
    let mut window_of: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut last_print: Option<String> = None;
    // Прошлая жалоба на непривязанные окна — чтобы не повторять её каждый такт.
    // Хранится сама строка, а не отметка времени: повод сказать снова — другая
    // картина, а не прошедший срок.
    let mut last_diag: Option<String> = None;
    let mut last_write_ms = 0u64;
    // Каталог файлов трекеров у себя. Считается один раз: дом за жизнь
    // процесса не переезжает, а `remoteDir` из конфига сюда не годится — это
    // путь на **чужой** машине, и разворачивать его тильду здесь некому.
    let local_windows_dir = mwm_core::config::local_windows_dir(&std::env::var("HOME").unwrap_or_default());
    let pid = std::process::id();
    loop {
        // Настройки берутся один раз за оборот, а не по месту: иначе половина
        // такта работала бы по старым флагам, половина по новым, и объяснить
        // человеку увиденное было бы нечем.
        //
        // Затеняет внешний `cfg` намеренно: всё, что ниже, обязано смотреть на
        // свежие настройки, и «забыл переименовать» здесь означало бы тихо
        // работающий по старому кусок такта.
        let cfg = shared.get();
        let features = cfg.features.clone();
        // Ожидание вместо сна: просьба исполняется сразу, а не к следующему
        // такту. Отсоединение канала (поток подписки умер) — тот же сон:
        // `recv_timeout` на закрытом канале возвращается мгновенно, и без
        // этой ветки такт превратился бы в горячий цикл.
        match requests.recv_timeout(Duration::from_millis(cfg.tick_ms)) {
            Ok(req) => {
                let mut pending = vec![req];
                // Разгребается вся очередь, а не одна просьба: иначе каждая
                // стоила бы полного такта с перечислением окон и, возможно,
                // походом за дампом по ssh.
                while let Ok(next) = requests.try_recv() {
                    pending.push(next);
                }
                for req in pending {
                    // Очередь вычитывается всегда, а исполняется по флагу:
                    // невычитанные просьбы копились бы в канале и хлынули бы
                    // все разом в тот момент, когда человек вернёт тумблер.
                    if !features.requests {
                        continue;
                    }
                    let note = serve(&req, &mut tracker, &registry, &window_of, &work);
                    if let Some(note) = note {
                        mwm_log!("{note}");
                        *status.0.lock().unwrap() = note;
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                std::thread::sleep(Duration::from_millis(cfg.tick_ms));
            }
        }
        // Ответ системы объявляется всем, кому он нужен, а не только этой
        // ветке: по нему поток меню решает, показывать ли пункт «выдать
        // разрешение». Спрашивать второй раз из того потока — значит позволить
        // двум ответам разойтись.
        let is_trusted = ax::trusted();
        trusted.0.store(is_trusted, Ordering::Relaxed);
        if !is_trusted {
            *status.0.lock().unwrap() = "Accessibility not granted".to_string();
            // Список окон уходит из меню вместе с разрешением: без него такт
            // до окон не доходит вовсе, и оставшиеся пункты называли бы окна,
            // про которые трекер уже ничего не знает.
            windows.0.lock().unwrap().clear();
            // Один раз на потерю разрешения, а не каждый такт: жалоба длинная,
            // а такт секундный — в логе она вытеснила бы всё остальное за
            // минуту. Латч сбрасывается ниже, когда разрешение снова есть, так
            // что о повторной потере человек узнает снова.
            if !told_untrusted {
                let exe = std::env::current_exe().ok();
                for line in mwm_core::permissions::accessibility_missing(exe.as_deref()).lines() {
                    mwm_log!("{line}");
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
        // Жалоба на расстановку копится до конца такта, а не пишется в трей
        // сразу: итог такта ниже переписывает ту же ячейку целиком и затирал
        // бы её в том же обороте. В stderr она оставалась, но трей —
        // единственный канал, где человек видит отказ, не читая логов.
        let mut place_note = String::new();
        // Выключается только сама расстановка. Слоты продолжают вестись, и
        // `state.json` продолжает писаться: иначе выключенный тумблер стирал бы
        // человеку запомненные места, и вернувший его обратно не вернул бы их.
        if features.placement {
            for (window_id, want) in tracker.placements() {
                let target = mwm_core::geometry::clamp_to_displays(want, &screens);
                if let Err(e) = ax::place(&registry, window_id, target) {
                    // Молчать нельзя: «поставил» и «не смог» отличаются только этим.
                    mwm_log!("place failed: {e}");
                    place_note = format!("place failed: {e}");
                }
            }
        }
        let bound = tracker.bound();
        // Подписи для меню собираются каждый такт, а не в ветке публикации:
        // окна ведутся и тогда, когда посылка не удалась, и меню обязано
        // показывать то, что ведёт трекер, а не то, что уехало читателю.
        //
        // Порядок — по подписи. `bound` разложен по идентификатору сессии, то
        // есть по uuid: человеку это случайный порядок, вдобавок он менялся бы
        // от такта к такту, стоит одному окну закрыться.
        {
            let mut labels: Vec<String> = bound
                .values()
                .map(|b| mwm_core::status::window_item_label(&b.title, &b.app))
                .collect();
            labels.sort();
            *windows.0.lock().unwrap() = labels;
        }
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
        // Окна, которые видно, но которые никому не достались. Правило
        // совпадения то же, что у `window_of` строкой выше, и это не совпадение:
        // «окно не нашло своей сессии» обязано значить ровно обратное тому, что
        // значит «сессия нашла своё окно», иначе жалоба говорила бы о другом
        // событии, чем то, на которое жалуются.
        //
        // Окно, назвавшееся именем приложения, потерянным не считается: про
        // него платформа ничего не сказала (см. `Seen.nameless`), и такой такт
        // трекер пропускает целиком. Считай их здесь — и каждое гашение экрана
        // выглядело бы в логе поломкой, то есть жалоба приходила бы ровно на
        // работающую починку.
        let unbound: Vec<String> = seen
            .iter()
            .filter(|w| {
                let key = mwm_core::title::strip_decoration(&w.title);
                !w.nameless && !bound.values().any(|b| b.title == key)
            })
            .map(|w| w.title.clone())
            .collect();
        match mwm_core::diag::binding_note(
            seen.len(),
            bound.len(),
            seen.iter().filter(|w| w.nameless).count(),
            &unbound,
            &tracker.unresolved(),
        ) {
            // Печатается на смену картины, а не каждый такт: такт секундный, и
            // повторение одной и той же жалобы вытеснило бы из лога всё
            // остальное за минуту. Смена — это другой состав непривязанных
            // окон или другие заголовки у них, то есть ровно то, ради чего
            // строку и читают.
            //
            // Время здесь больше не ставится вручную: его несёт теперь каждая
            // строка лога (`log::stamped`), и довод, стоявший на этом месте,
            // оказался верен для всех — launchd отметок не ставит, а жалобу
            // сводят с событием на другой машине (сон, разрыв, перезапуск).
            Some(note) if last_diag.as_deref() != Some(note.as_str()) => {
                mwm_log!("{note}");
                last_diag = Some(note);
            }
            Some(_) => {}
            // Картина сошлась — прошлая жалоба забывается. Иначе следующая
            // такая же потерялась бы как «уже говорил», а это была бы уже
            // вторая поломка, а не та же самая.
            None => last_diag = None,
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
        //
        // Выключенные снимки не считаются вовсе — ни ключ состава, ни дебаунс:
        // считать дебаунс некому и не для чего, а вернув тумблер, человек
        // получит первый снимок ровно тем же путём, что и при первом запуске.
        if features.snapshots {
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
        }

        // Одно значение на оба применения. Разойдись они — отпечаток не заметил
        // бы смены флага, и файл окон дожил бы со старым `focus` до
        // сердцебиения.
        let focus = link.is_live() && features.requests;

        let print = fingerprint(&bound, focus);

        // Ошибка чтения дампа и ошибка записи файла окон — про разные машины
        // и разные починки, и одна не должна прятать другую. Без этой строки
        // трекер с нечитаемым дампом выглядел бы неотличимо от трекера, у
        // которого просто нет сессий для привязки: публикация могла бы идти
        // исправно (`deliver::send` про дамп ничего не знает), а привязки —
        // не случаться никогда, молча.
        let fetch_note = cache
            .last_error
            .as_deref()
            .map_or_else(String::new, |e| format!("index fetch failed: {e}"));

        // Отдельно от удалённой: чинят их в разных местах — ту на машине
        // агрегатора, эту у себя, — и одна, спрятавшая другую, отправила бы
        // человека не туда. Отсутствующий местный дамп сюда не попадает
        // вовсе: это честно выключенная половина, а не отказ.
        let local_fetch_note = cache
            .local_error
            .as_deref()
            .map_or_else(String::new, |e| format!("local index unreadable: {e}"));

        // Файл состояния пишется с `fsync`, и писать его на каждом такте —
        // плата за то, что не изменилось.
        if tracker.take_dirty() {
            if let Err(e) = mwm_core::state::write_atomic(
                &state_path,
                &mwm_core::state::state_json(&tracker.slots_state()),
            ) {
                mwm_log!("state write failed: {e}");
            }
        }

        if should_write(&print, last_print.as_deref(), last_write_ms, now) {
            // Снимки едут в build_file, но в fingerprint не входят: тот их не
            // видит, и should_write не отличит «появился снимок» от «ничего не
            // изменилось». Решение — оставить как есть: HEARTBEAT_MS (полминуты)
            // перепишет файл не позже чем через полминуты, а снимок, чтобы
            // родиться, отстоял минуту дебаунса, — опоздание вдвое меньше того,
            // что уже потрачено на его рождение.
            let payload = build_file(
                &bound,
                &cfg.host,
                pid,
                now,
                focus,
                &cfg.mqtt.base,
                // Выключенные снимки не только не пишутся на диск, но и не
                // публикуются: иначе пикер показывал бы в `^S` раскладки,
                // которые эта машина больше не ведёт.
                if features.snapshots { &snaps } else { &[] },
            );
            // Второй адрес — свой же каталог: там файл читает агрегатор этой
            // машины, и только оттуда окно доедет до сессии, которая работает
            // здесь. Отказ этой записи не отменяет удачи первой и наоборот:
            // адреса разные, читатели разные, и общий отказ на двоих скрыл бы
            // от человека, какая половина цела.
            let local_note = if features.local_source {
                match deliver::write_local(&local_windows_dir, &cfg.host, &payload) {
                    Ok(()) => String::new(),
                    Err(e) => format!("local publish failed: {e}"),
                }
            } else {
                String::new()
            };
            let notes = [
                place_note.as_str(),
                fetch_note.as_str(),
                local_fetch_note.as_str(),
                local_note.as_str(),
            ];
            match deliver::send(&cfg, &payload) {
                Ok(()) => {
                    last_print = Some(print);
                    last_write_ms = now;
                    let base = format!("{} windows tracked", bound.len());
                    *status.0.lock().unwrap() = mwm_core::status::status_line(&base, &notes);
                }
                // Ничего не копится: следующая посылка везёт текущее состояние
                // целиком, а протухший файл читатель отбрасывает сам.
                Err(e) => {
                    let base = format!("publish failed: {e}");
                    *status.0.lock().unwrap() = mwm_core::status::status_line(&base, &notes);
                }
            }
        } else if !place_note.is_empty()
            || !fetch_note.is_empty()
            || !local_fetch_note.is_empty()
        {
            // Расклад не менялся, публикации в этом такте не было — но отказ
            // расстановки и ошибка чтения дампа не должны ждать следующей
            // записи файла, до неё может быть далеко (или не наступить вовсе,
            // пока отпечаток не сдвинется).
            let base = format!("{} windows tracked", bound.len());
            *status.0.lock().unwrap() = mwm_core::status::status_line(
                &base,
                &[place_note.as_str(), fetch_note.as_str(), local_fetch_note.as_str()],
            );
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
        mwm_log!("broken snapshots file, starting empty");
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
        mwm_log!("snapshots write failed: {e}");
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
    work: &Work,
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
        Request::Arrange { mode, ids } => {
            arrange(*mode, ids, tracker, registry, window_of, work)
        }
    }
}

/// Разложить окна по главному экрану.
///
/// Порядок: тот, что приехал в просьбе, а иначе — тот, которым подписаны
/// пункты меню. «В порядке сортировки списка» иначе не сделать: порядок знает
/// только тот, кто список показывает, и своего у этой стороны нет.
fn arrange(
    mode: mwm_core::layout::Layout,
    ids: &[String],
    tracker: &Tracker,
    registry: &ax::Registry,
    window_of: &std::collections::HashMap<String, u64>,
    work: &Work,
) -> Option<String> {
    // Настоящая рабочая область приезжает с главного потока и знает про Dock с
    // любой стороны. Запасная — экран без полосы меню — считается здесь только
    // до первого захода рисовальщика: Dock в ней не вычтен, и окна слева уйдут
    // под него, но это одна раскладка в первые две секунды после запуска,
    // а отказ был бы виден всегда.
    let known = *work.0.lock().unwrap();
    let area = match known {
        Some(area) => area,
        None => match ax::main_display() {
            Some(screen) => mwm_core::layout::work_area(screen.bounds),
            None => return Some("place: no main display".to_string()),
        },
    };
    let order: Vec<u64> = if ids.is_empty() {
        // Та же подпись и та же сортировка, что рисуют меню, — потому порядок
        // раскладки и порядок пунктов совпадают по построению, а не по
        // договорённости, которую первая же правка нарушит.
        let mut named: Vec<(String, u64)> = tracker
            .bound()
            .values()
            .filter_map(|b| {
                let window_id = window_of.get(&b.session_id)?;
                Some((mwm_core::status::window_item_label(&b.title, &b.app), *window_id))
            })
            .collect();
        named.sort();
        named.into_iter().map(|(_, window_id)| window_id).collect()
    } else {
        // Сессия без окна пропускается молча: список приехал с той стороны, и
        // окно могли закрыть между опросом и нажатием. Отказать всей раскладке
        // из-за одной закрытой сессии — значит не разложить ничего.
        ids.iter().filter_map(|id| window_of.get(id).copied()).collect()
    };
    if order.is_empty() {
        return Some("place: no windows to place".to_string());
    }
    let mut failed = 0usize;
    let mut last = String::new();
    for (window_id, want) in order
        .iter()
        .zip(mwm_core::layout::arrange(mode, area, order.len()))
    {
        if let Err(e) = ax::place(registry, *window_id, want) {
            mwm_log!("place failed: {e}");
            failed += 1;
            last = e;
        }
    }
    // Подъём — после всей раскладки, а не после каждого окна: `place` двигает
    // и ужимает, но порядка по глубине не трогает вовсе, и окно, лежавшее под
    // чужим, осталось бы под ним на новом месте. Человек нажал бы кнопку и не
    // увидел ничего.
    //
    // Отказ подъёма не отменяет раскладки — окна уже стоят по местам, — но и
    // не молчит: он идёт в ту же строку состояния, что и отказ расстановки,
    // только позже неё. Отказавшая расстановка важнее: она объясняет и то,
    // почему не видно окна.
    let raise_note = ax::bring_to_front(registry, &order).err();
    if let Some(e) = raise_note.as_deref() {
        mwm_log!("{e}");
    }
    // Жалоба называет число: одно неподатливое окно из шести — не то же самое,
    // что раскладка, не сработавшая вовсе.
    (failed > 0)
        .then(|| format!("place: {failed} of {} windows failed: {last}", order.len()))
        .or(raise_note)
}

/// Настройки, как их покажет окно.
///
/// Две картины, а не одна. `file` — то, что лежит в config.yaml: по нему форма
/// заполняет поля и по нему же считает, что человек тронул. `effective` — то,
/// что подхватил трекер, с умолчаниями: оно показывается подсказкой. Второй
/// копии умолчаний в JS нет намеренно — она разошлась бы с `parse_config` на
/// первой же правке.
#[tauri::command]
fn load_settings(shared: tauri::State<'_, Shared>) -> Result<serde_json::Value, String> {
    let path = config_file_path();
    let file: serde_yaml::Value = match std::fs::read_to_string(&path) {
        Ok(text) if !text.trim().is_empty() => serde_yaml::from_str(&text)
            .map_err(|e| format!("bad yaml in {}: {e}", path.display()))?,
        // Файла нет или он пуст — не ошибка: окно настроек и заводится затем,
        // чтобы его создать.
        Ok(_) => serde_yaml::Value::Null,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_yaml::Value::Null,
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let file = serde_json::to_value(&file).map_err(|e| format!("cannot convert yaml: {e}"))?;
    Ok(serde_json::json!({
        "file": file,
        "effective": mwm_core::config::to_json(&shared.get()),
    }))
}

/// Сохранить присланное формой и объявить это трекеру.
///
/// Ячейка обновляется перечитыванием файла, а не патчем поверх прежнего
/// конфига: разбирает YAML `parse_config`, и только он знает, во что
/// превращаются мусор и умолчания. Собери мы новый `Config` из патча — окно
/// показывало бы одно, а трекер работал бы по другому.
#[tauri::command]
fn save_settings(shared: tauri::State<'_, Shared>, patch: serde_json::Value) -> Result<(), String> {
    write_config(&config_file_path(), &patch)?;
    *shared.0.lock().unwrap() = load_config();
    Ok(())
}

/// Отдать вкладке Log то, что накопилось в буфере.
///
/// Не `Result`: читать тут нечего — буфер живёт в памяти этого же процесса, и
/// отказать он может разве что паникой соседнего потока, которую `log::lines`
/// и переживает молча. Отказ, который нельзя показать иначе как в том же
/// логе, показывать было бы негде.
#[tauri::command]
fn read_log() -> Vec<String> {
    mwm_core::log::lines()
}

/// Открыть окно настроек.
///
/// Создаётся лениво: объявленное в `tauri.conf.json` окно поднималось бы на
/// старте, и второй webview висел бы в памяти у каждого, кто в настройки не
/// заходит.
///
/// `async` здесь не украшение. Синхронную команду Tauri исполняет прямо в
/// потоке цикла событий, а создание webview этот же цикл и ждёт: `build()`
/// возвращает Ok, окно появляется, а страница в нём не загружается никогда —
/// белый прямоугольник. Заплачено этим в соседнем ccfzf-picker.
#[tauri::command]
async fn open_settings(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("settings") {
        // Свёрнутое окно `show` не разворачивает, а закрытое крестиком — это
        // не уничтоженное: ветка выхода в `main` держит процесс живым, окно
        // остаётся в списке скрытым.
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    } else {
        tauri::WebviewWindowBuilder::new(
            &app,
            "settings",
            tauri::WebviewUrl::App("settings.html".into()),
        )
        .title("macos-windows-manager Settings")
        // Шире прежних 560 ровно на боковой список вкладок и ниже прежних
        // 620: длиннее пяти полей вкладки в форме нет, и высота под все
        // четырнадцать разом теперь ни к чему.
        .inner_size(720.0, 560.0)
        .center()
        .resizable(true)
        .build()
        .map_err(|e| format!("cannot open settings window: {e}"))?;
    }
    // Обе ветки проходят через активацию: `set_focus` и `build` двигают окно
    // внутри приложения, а впереди остаётся то, из которого человек полез в
    // меню трея, — терминал. Ветки объединены ради этой строки: разойдись они,
    // одна из двух дорог рано или поздно её потеряла бы.
    //
    // Отказ активации не отменяет открытия: окно уже создано и показано, и
    // отвечать на это ошибкой значило бы пугать человека тем, что он и так
    // видит. Но и молчать нельзя — в лог.
    if let Err(e) = ax::activate_self() {
        mwm_log!("{e}");
    }
    Ok(())
}

/// Попросить о раскладке из меню трея.
///
/// Отказ виден только в логе: пункт меню ответить человеку нечем, а раскладка
/// либо случится через такт, либо не случится вовсе — и тогда о причине скажет
/// строка состояния, куда трекер положит свою жалобу.
fn ask(app: &tauri::AppHandle, mode: mwm_core::layout::Layout) {
    use tauri::Manager;
    // Пустой список сессий: порядок собирает трекер — он же и знает, каким
    // порядком подписаны пункты меню.
    let req = mwm_core::request::Request::Arrange { mode, ids: Vec::new() };
    let asking = app.state::<Asking>();
    let sent = asking.0.lock().map_err(|e| e.to_string()).and_then(|tx| tx.send(req).map_err(|e| e.to_string()));
    if let Err(e) = sent {
        mwm_log!("cannot ask for a layout: {e}");
    }
}

/// Поставить глобальный хоткей раскладки плиткой.
///
/// Возвращает то же, что показывает меню: встал ли хоткей и на какой
/// комбинации. Пара, а не одно значение, потому что комбинация уезжает в
/// правую колонку пункта в обоих случаях, а вот подпись при отказе другая —
/// иначе мёртвая клавиша выглядела бы сломанным конфигом.
///
/// Непонятая строка откатывается на умолчание здесь, а не в `parse_config`:
/// понимает комбинацию плагин, а он живёт в приложении, и `mwm-core` про
/// клавиатуру не знает вовсе.
///
/// Обработчик шлёт ту же просьбу, что и пункт меню, — тем же каналом и в тот
/// же поток трекера: две дороги к одной раскладке разошлись бы на первой же
/// правке.
#[cfg(desktop)]
fn register_tile_hotkey(app: &tauri::AppHandle, wanted: &str) -> (bool, String) {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
    let (shortcut, accelerator) = match wanted.parse::<Shortcut>() {
        Ok(sc) => (sc, wanted.to_string()),
        Err(e) => {
            mwm_log!("cannot parse tileHotkey {wanted}: {e}, using default");
            let fallback = mwm_core::config::DEFAULT_TILE_HOTKEY;
            match fallback.parse::<Shortcut>() {
                Ok(sc) => (sc, fallback.to_string()),
                // Умолчание не разобралось — это ошибка в самом умолчании, и
                // молчать о ней нельзя, но и падать незачем: трей с рабочим
                // пунктом меню полезнее, чем не поднявшееся приложение.
                Err(e) => {
                    mwm_log!("default hotkey {fallback} does not parse: {e}");
                    return (false, fallback.to_string());
                }
            }
        }
    };
    let registered = match app.global_shortcut().on_shortcut(shortcut, |app, _sc, event| {
        // Только нажатие: без проверки одна и та же комбинация просила бы о
        // раскладке дважды — второй раз на отпускании.
        if event.state() == ShortcutState::Pressed {
            ask(app, mwm_core::layout::Layout::Tile);
        }
    }) {
        Ok(()) => true,
        Err(e) => {
            // Отказ не фатален и обязан быть виден: сочетание мог занять кто
            // угодно, и человек об этом узнает только из подписи пункта.
            mwm_log!("cannot register tile hotkey {accelerator}: {e}");
            false
        }
    };
    (registered, accelerator)
}

/// Время сборки этого бинаря, если оно в него вшито.
///
/// `None` у релизной сборки: её называет версия, а штамп там лишний. Ноль в
/// штампе значит именно это — см. `build.rs`.
///
/// Живёт здесь, а не рядом с `version_item_label` в `mwm-core`: `env!` читает
/// переменную того крейта, в котором написан, а вшивает её `build.rs`
/// приложения.
fn build_time() -> Option<chrono::NaiveDateTime> {
    use chrono::TimeZone;
    let secs: i64 = env!("MWM_BUILD_UNIX").parse().ok()?;
    if secs == 0 {
        return None;
    }
    Some(chrono::Local.timestamp_opt(secs, 0).single()?.naive_local())
}

fn main() {
    tauri::Builder::default()
        // Плагин хоткеев — единственный способ услышать клавишу, когда впереди
        // чужое приложение: у трея окон нет вовсе, и слушать некому.
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            load_settings,
            save_settings,
            open_settings,
            read_log
        ])
        .setup(|app| {
            // Окон у приложения нет, значит и месту в доке взяться неоткуда.
            // Без этой строки macOS считает процесс обычным приложением и
            // держит значок в доке рядом со значком в строке меню — два места
            // для одного трея. `Accessory` — это то же, что `LSUIElement` в
            // Info.plist, но у нас голый бинарь без бандла, и plist'а нет.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // Канал просьб заводится до меню и до хоткея: тот же конец нужен и
            // пунктам меню, и обработчику клавиши, а живут они на главном
            // потоке. Хоткей встаёт раньше, чем собрано меню, — и нажми человек
            // клавишу в этот промежуток, просьбе было бы некуда лечь. Отсюда же
            // и то, что канал не закроется, останься подписка ненастроенной, —
            // конец лежит в `Asking` до конца жизни приложения, и `recv_timeout`
            // у трекера не начнёт возвращать `Disconnected` в горячем цикле.
            //
            // `Mutex` не ради гонки за отправку — `Sender` и так `Send`, — а
            // ради `Sync`: обработчик меню обязан быть им, а `Sender` сам по
            // себе не `Sync`.
            let (to_requests, requests) = std::sync::mpsc::channel();
            let asking = Asking(Arc::new(Mutex::new(to_requests.clone())));
            app.manage(asking);

            let status = Status(Arc::new(Mutex::new("starting…".to_string())));
            let state = MenuItem::with_id(app, "status", "starting…", false, None::<&str>)?;
            let grant = MenuItem::with_id(
                app,
                "grant",
                "\u{26bf}\u{200a}\u{200a} Grant Accessibility…",
                true,
                None::<&str>,
            )?;
            let settings =
                MenuItem::with_id(app, "settings", "\u{2699}\u{200a}\u{200a}\u{200a}\u{200a}\u{200a} Settings", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "\u{23fb} Quit", true, None::<&str>)?;
            // Неактивный пункт: он не действие, а подпись. Стоит последним, под
            // «Quit», — читают его редко, а два пункта выше нажимают, и
            // сдвигать их ради подписи нельзя.
            let version = MenuItem::with_id(
                app,
                "version",
                mwm_core::status::version_item_label(
                    env!("CARGO_PKG_VERSION"),
                    build_time(),
                    chrono::Local::now().date_naive(),
                ),
                false,
                None::<&str>,
            )?;
            // Состояние разрешения выясняется до сборки меню, а не после:
            // пункт «выдать разрешение» при выданном разрешении не нужен вовсе,
            // и, соберись меню всегда с ним, при каждом запуске он мелькал бы и
            // пропадал через такт рисовальщика.
            let trusted_now = ax::trusted();
            let trusted = Trusted(Arc::new(AtomicBool::new(trusted_now)));
            // Конфиг читается до сборки меню: от него зависит комбинация в
            // правой колонке пункта плитки, а от того, встала ли она, — его
            // подпись. Прочитанное уезжает дальше в ячейку трекера: второе
            // чтение разошлось бы с первым, правь человек файл в этот
            // промежуток.
            let config = load_config();
            let (hotkey_registered, hotkey_accelerator) =
                register_tile_hotkey(app.handle(), &config.tile_hotkey);
            // Раскладка без разрешения не сработает — двигать окна нечем, — и
            // пункт заводится сразу неактивным, чтобы человек не нажимал в
            // пустоту. Гаснет и загорается он там же, где приходит и уходит
            // пункт «выдать разрешение».
            // Значки — знаки шрифта, а не картинки. `IconMenuItem` потребовал бы
            // растра на каждую тему и каждый масштаб экрана, а знак берёт цвет и
            // размер у меню сам. Выбраны те, у которых нет цветного начертания:
            // символ с `Emoji_Presentation` встал бы в меню цветной картинкой
            // втрое выше строки.
            //
            // За значком стоят волосяные пробелы (`U+200A`, 0,84 pt) — ими
            // ширина значка догоняется до самого широкого, `⏻` в 11,95 pt,
            // чтобы подписи начинались с одного места. Числа не выдуманы: они
            // измерены `CTLineGetTypographicBounds` в шрифте меню
            // (`.AppleSystemUIFont` 13 pt) на самой машине. Знаки эти приезжают
            // из разных шрифтов — `▦` из Apple SD Gothic Neo, `❐` из Zapf
            // Dingbats, `⚙` из Menlo, `⚿` из STIX Two Math, и только `⏻` из
            // самого шрифта меню, — оттого ширины и разошлись: в системном
            // шрифте знаков для плитки и каскада нет вовсе.
            //
            // Остаток расхождения — не больше 0,4 pt, треть волосяного пробела.
            //
            // Комбинация уходит в нативный слот акселератора, а не в подпись:
            // NSMenuItem рисует её правой колонкой сам, знаками клавиш (⌃⌥⌘C),
            // и приписать её к подписи значило бы получить эту же строку словами
            // посреди пункта.
            let tile = MenuItem::with_id(
                app,
                "tile",
                format!("\u{25a6}\u{200a} {}", mwm_core::status::tile_item_label(hotkey_registered)),
                trusted_now,
                Some(hotkey_accelerator.as_str()),
            )?;
            let cascade = MenuItem::with_id(
                app,
                "cascade",
                "\u{2750}\u{200a}\u{200a} Cascade windows",
                trusted_now,
                None::<&str>,
            )?;
            // `Settings…` встаёт над `Quit`, и `GRANT_POSITION` от этого не
            // меняется: приходящий и уходящий пункт по-прежнему второй сверху.
            // Пункты раскладки встают под `GRANT_POSITION`, а список окон —
            // под ними: у кнопки место постоянное, а список растёт и тает от
            // числа окон, и кнопка уезжала бы у человека из-под курсора.
            let menu = if trusted_now {
                Menu::with_items(app, &[&state, &tile, &cascade, &settings, &quit, &version])?
            } else {
                Menu::with_items(app, &[&state, &grant, &tile, &cascade, &settings, &quit, &version])?
            };
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
                    "settings" => {
                        // Команда `async`, а обработчик — нет; отправляем её в
                        // пул той же причины ради, что расписана у
                        // `open_settings`.
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = open_settings(app).await {
                                mwm_log!("{e}");
                            }
                        });
                    }
                    "tile" => ask(app, mwm_core::layout::Layout::Tile),
                    "cascade" => ask(app, mwm_core::layout::Layout::Cascade),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // Ячейка заводится до потока трекера: тот читает её на первом же
            // обороте, и заводить её после значило бы гоняться с ним за первый
            // такт.
            let shared = Shared(Arc::new(Mutex::new(config)));
            app.manage(shared.clone());

            let windows = Windows(Arc::new(Mutex::new(Vec::new())));

            // Рабочая область: пустая до первого захода рисовальщика на главный
            // поток. Пустоту трекер переживает — считает по запасной, — а вот
            // спросить `NSScreen` из своего потока не может вовсе.
            let work = Work(Arc::new(Mutex::new(None)));

            let worker = status.clone();
            let worker_trusted = trusted.clone();
            let worker_windows = windows.clone();
            let worker_work = work.clone();
            std::thread::spawn(move || {
                run_tracker(
                    worker,
                    worker_trusted,
                    shared,
                    worker_windows,
                    requests,
                    to_requests,
                    worker_work,
                )
            });

            // Строка состояния обновляется своим тиком: лезть в меню из потока
            // трекера нельзя — пункты меню живут на главном потоке.
            let painter = status.clone();
            let handle = app.handle().clone();
            // Второй дескриптор — тому же приложению: `handle` уходит в
            // `run_on_main_thread` каждый такт, а заводить пункты меню внутри
            // замыкания нужно через него же.
            let handle_for_items = handle.clone();
            let menu_for_painter = menu.clone();
            // Стоит ли пункт в меню сейчас. Меню трогается только на смене
            // ответа: без этой памяти каждые две секунды уходил бы `remove` или
            // `insert` впустую, а повторный `remove` уже убранного пункта ещё и
            // отвечает ошибкой.
            let mut grant_shown = !trusted_now;
            // Что за пункты со списком окон стоят в меню сейчас и как они
            // подписаны. Ячейка, а не переменная цикла: заводить и убирать
            // пункты можно только на главном потоке, а замыкание, отправленное
            // туда, ничего не возвращает — держать нарисованное между тактами
            // больше негде.
            let drawn: Drawn = Arc::new(Mutex::new((Vec::new(), Vec::new())));
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(2));
                let text = painter.0.lock().unwrap().clone();
                let labels = windows.0.lock().unwrap().clone();
                // Разрешение выдают и отзывают на ходу, перезапуска macOS для
                // этого не требует: `AXIsProcessTrusted` начинает отвечать
                // иначе, трекер это видит на следующем такте — и пункт уходит
                // или возвращается сам.
                let want_grant = !trusted.0.load(Ordering::Relaxed);
                let toggle = (want_grant != grant_shown).then_some(want_grant);
                grant_shown = want_grant;
                let _ = handle.run_on_main_thread({
                    let state = state.clone();
                    let grant = grant.clone();
                    let menu = menu_for_painter.clone();
                    let drawn = drawn.clone();
                    let app = handle_for_items.clone();
                    let tile = tile.clone();
                    let cascade = cascade.clone();
                    let work = work.clone();
                    move || {
                        // Первым делом, до всякой работы с меню: раскладка без
                        // рабочей области считает по запасной, где Dock не
                        // вычтен, и лишний такт такого счёта — это окна под
                        // Dock у того, кто нажал сразу после запуска.
                        *work.0.lock().unwrap() = ax::main_work_area();
                        let _ = state.set_text(&text);
                        match toggle {
                            // Пункт «выдать разрешение» приходит ровно тогда,
                            // когда раскладка перестаёт работать: двигать окна
                            // без Accessibility нечем. Одна ветка на оба
                            // события — разойдись они, пункт остался бы
                            // нажимаемым и молчащим.
                            Some(true) => {
                                let _ = menu.insert(&grant, GRANT_POSITION);
                                let _ = tile.set_enabled(false);
                                let _ = cascade.set_enabled(false);
                            }
                            Some(false) => {
                                let _ = menu.remove(&grant);
                                let _ = tile.set_enabled(true);
                                let _ = cascade.set_enabled(true);
                            }
                            None => {}
                        }
                        // Меню трогается только на смене списка — по той же
                        // причине, что и пункт «выдать разрешение»: такт
                        // рисовальщика двухсекундный, а состав окон меняется
                        // раз в час, и перебирать пункты впустую незачем.
                        let mut drawn = drawn.lock().unwrap();
                        if drawn.0 == labels {
                            return;
                        }
                        for item in drawn.1.drain(..) {
                            let _ = menu.remove(&item);
                        }
                        // Под пунктом «выдать разрешение», когда тот стоит:
                        // место у него постоянное (`GRANT_POSITION`), и список
                        // обязан вставать под ним, а не поверх — иначе пункт,
                        // который человек ищет глазами на одном месте, уезжал
                        // бы вниз от каждого нового окна.
                        let base = GRANT_POSITION + usize::from(want_grant) + LAYOUT_ITEMS;
                        for (i, label) in labels.iter().enumerate() {
                            // Пункты неактивные: они подписи, а не действия, —
                            // то же, что у строки состояния и версии.
                            //
                            // Отступ из двух пробелов — единственный способ
                            // показать подчинение строке состояния: вложенных
                            // пунктов без подменю в меню трея нет, а без
                            // отступа список читается как продолжение
                            // действий ниже.
                            let item = MenuItem::with_id(
                                &app,
                                format!("window-{i}"),
                                format!("  {label}"),
                                false,
                                None::<&str>,
                            );
                            if let Ok(item) = item {
                                if menu.insert(&item, base + i).is_ok() {
                                    drawn.1.push(item);
                                }
                            }
                        }
                        drawn.0 = labels;
                    }
                });
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            // Закрытие последнего окна гасит приложение по умолчанию
            // (tauri-runtime-wry 2.10.0, src/lib.rs:4177): у трея окон не было
            // вовсе, и заметить это стало возможно только с появлением первого.
            // Крестик на форме настроек не имеет права снять трекер.
            //
            // Только `code: None`. `app.exit(0)` из пункта `Quit` приезжает тем
            // же событием с `code: Some(0)`, и глухая ветка сделала бы трей
            // неубиваемым.
            if let tauri::RunEvent::ExitRequested { code: None, api, .. } = event {
                api.prevent_exit();
            }
        });
}

/// Тесты-сторожа: они читают исходник, а не зовут код.
///
/// Такт трекера не разложить на чистые функции без переписывания всего файла, а
/// проверить эти связки надо: они рвутся молча, и видны только на живом маке.
/// Приём взят у соседнего ccfzf-picker, где точно так же сторожится пункт меню
/// трея.
#[cfg(test)]
mod tests {
    /// Исходник без хвоста с самими тестами.
    ///
    /// Строки, которые ищут сторожа, написаны и в них самих, — сравнивая с
    /// целым файлом, они находили бы себя и проходили всегда. Резать по
    /// `#[cfg(test)]` надёжно ровно потому, что этот атрибут в файле один: он и
    /// открывает хвост.
    fn tracker_source() -> &'static str {
        include_str!("main.rs").split("#[cfg(test)]").next().unwrap()
    }

    /// Свой каталог во временной директории на каждый тест: `write_config`
    /// трогает настоящую файловую систему, и тесты не должны видеть файлы друг
    /// друга при параллельном запуске.
    fn temp_config_path(tag: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir()
            .join(format!("mwm-test-{}-{tag}-{n}", std::process::id()))
            .join("config.yaml")
    }

    #[test]
    fn write_config_creates_missing_directory_and_file() {
        let path = temp_config_path("missing-dir");
        assert!(!path.parent().unwrap().exists());
        super::write_config(&path, &serde_json::json!({"sshHost": "host"})).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("host"));
    }

    #[test]
    fn write_config_keeps_untouched_keys() {
        // Блок `state:` форма не показывает вовсе, и перезапись файла целиком
        // стёрла бы человеку настроенные пути молча.
        let path = temp_config_path("untouched");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "sshHost: old\nstate:\n  path: /tmp/s.json\n").unwrap();
        super::write_config(&path, &serde_json::json!({"sshHost": "new"})).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("new"), "{text}");
        assert!(!text.contains("old"), "{text}");
        assert!(text.contains("/tmp/s.json"), "чужой ключ пережил запись: {text}");
    }

    #[test]
    fn write_config_backs_up_once() {
        // Второе сохранение затёрло бы единственную копию исходного файла его
        // же, уже применённым, состоянием — и комментарии человека пропали бы
        // окончательно.
        let path = temp_config_path("backup-once");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "sshHost: original\n").unwrap();
        super::write_config(&path, &serde_json::json!({"sshHost": "first"})).unwrap();
        super::write_config(&path, &serde_json::json!({"sshHost": "second"})).unwrap();
        let backup = std::fs::read_to_string(path.with_extension("yaml.bak")).unwrap();
        assert!(backup.contains("original"), "{backup}");
    }

    #[test]
    fn write_config_does_not_back_up_whitespace_only_file() {
        // Иначе файл из пробелов занял бы единственный слот `.bak` навсегда.
        let path = temp_config_path("whitespace-only");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "   \n\n").unwrap();
        super::write_config(&path, &serde_json::json!({"sshHost": "host"})).unwrap();
        assert!(!path.with_extension("yaml.bak").exists());
    }

    #[test]
    fn write_config_rejects_null_at_any_depth() {
        // `{"mqtt": {"password": null}}` стирает ровно тот ключ, ради которого
        // написано слияние по ключам.
        let path = temp_config_path("null-nested");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "mqtt:\n  host: broker\n  password: secret\n").unwrap();
        let err = super::write_config(&path, &serde_json::json!({"mqtt": {"password": null}}))
            .unwrap_err();
        assert!(err.contains("password"), "{err}");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("secret"), "пароль пережил отказ: {text}");
    }

    #[cfg(unix)]
    #[test]
    fn write_config_keeps_files_private() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_config_path("permissions");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Права нарочно шире нужных: сохранение обязано их сузить, а не
        // унаследовать.
        std::fs::write(&path, "mqtt:\n  password: secret\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        super::write_config(&path, &serde_json::json!({"sshHost": "host"})).unwrap();
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600, "config.yaml");
        assert_eq!(mode(&path.with_extension("yaml.bak")), 0o600, "config.yaml.bak");
    }

    #[test]
    fn the_build_script_watches_what_lives_outside_the_package() {
        // Поймать это поведением нельзя: про `build.rs` cargo решает раньше,
        // чем начнёт выполняться хоть что-то наше. Отсюда сторож по тексту.
        let script = include_str!("../build.rs");
        for dir in ["../crates", "../frontend"] {
            assert!(
                script.contains(&format!("cargo:rerun-if-changed={dir}")),
                "штамп сборки застынет на прошлой выкатке при правке одного {dir}"
            );
        }
        // Путь фронтенда тот же, что в конфиге: разойдись они, скрипт следил
        // бы за каталогом, из которого статику никто не берёт.
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["build"]["frontendDist"].as_str().unwrap(), "../frontend");
    }

    #[test]
    fn the_form_covers_every_field_it_promises() {
        // Поле, забытое в форме, выглядит не поломкой, а отсутствующей
        // настройкой: человек ищет его глазами и не находит. Дешёвый сторож
        // ровно на этот случай.
        let page = include_str!("../../frontend/settings.html");
        for key in [
            "placement", "snapshots", "requests", "localSource",
            "sshHost", "remoteDir", "windowHost",
            "terminals", "tickMs", "dumpCacheMs", "tileHotkey",
            "host", "port", "user", "password", "base",
        ] {
            assert!(page.contains(key), "поле {key} пропало из формы");
        }
        assert!(
            page.contains("after restart"),
            "группа MQTT обязана честно говорить, что действует после перезапуска"
        );
        // Вкладка Log — единственный способ увидеть лог у приложения,
        // запущенного из `.app`: stderr оттуда уходит в никуда. Пропади она из
        // формы — команда `read_log` осталась бы жива и никем не зовущаяся, а
        // поломка выглядела бы как «трекер молчит».
        assert!(page.contains("id: 'log'"), "вкладка Log пропала из формы");
        // Вызов целиком, а не имя команды подстрокой: `read_logs` содержало бы
        // `read_log` и оставило бы сторожа зелёным при мёртвом вызове.
        assert!(
            page.contains("invoke('read_log')"),
            "вкладка Log перестала спрашивать буфер"
        );
        // Тело вкладки попадает в разметку через `group.body`: у остальных
        // групп его нет, и потеряйся эта подстановка — Log открывался бы
        // пустым, а поля соседних вкладок остались бы на месте.
        assert!(page.contains("group.body"), "телу вкладки Log нечем попасть в разметку");
        assert!(
            page.contains("clearInterval(logTimer)"),
            "опрос лога обязан гаснуть — иначе он переживёт перерисовку формы"
        );
        // Группы разъехались по вкладкам, и вкладка заводится по `id` группы.
        // Забудь его — `renderTabs` нарисует пункт с `data-tab="undefined"`, и
        // с экрана пропадёт не одно поле, а вся группа разом. Сторож выше
        // такого не поймает: ключи-то в тексте остались.
        for line in page.lines().filter(|line| line.contains("title: '")) {
            assert!(
                line.contains("id: '"),
                "группа без id останется без вкладки и не покажется: {line}"
            );
        }
    }

    #[test]
    fn only_the_windowless_exit_is_prevented() {
        // Закрытие последнего окна и `app.exit(0)` приезжают одним событием, и
        // отличаются только кодом: `None` у первого, `Some(0)` у второго
        // (tauri-runtime-wry 2.10.0, src/lib.rs:4177 и 4217). Глухая ветка на
        // все коды сделала бы трей неубиваемым, а отсутствие ветки — крестик на
        // окне настроек гасил бы трекер.
        let src = tracker_source();
        assert!(
            src.contains("RunEvent::ExitRequested { code: None, api, .. }"),
            "ветка выхода обязана быть только для code: None"
        );
        assert!(src.contains("api.prevent_exit()"), "и обязана звать prevent_exit");
    }

    #[test]
    fn opening_the_settings_window_brings_the_application_forward() {
        // У `Accessory`-приложения окно, показанное без активации, остаётся за
        // терминалом, из которого человек полез в меню трея: `set_focus`
        // двигает окно внутри приложения, а какое приложение впереди, решает
        // AppKit. Проверяется текстом — вызов уходит в AppKit, и на машине
        // разработки этой ветки нет вовсе.
        let src = tracker_source();
        let open = src.split("async fn open_settings").nth(1).unwrap_or("");
        assert!(
            open.contains("ax::activate_self()"),
            "открытие настроек обязано выводить приложение вперёд"
        );
    }

    #[test]
    fn the_window_list_is_collected_every_tick_not_only_on_publish() {
        // Окна ведутся и тогда, когда посылка не удалась. Собирай подписи в
        // ветке публикации — и меню при отказе доставки показывало бы состав
        // окон той минуты, когда доставка ещё удавалась.
        let src = tracker_source();
        let fill = src.find("window_item_label").expect("подписи собираются в такте");
        let publish = src.find("if should_write(").expect("публикация на месте");
        assert!(fill < publish, "список собирается до ветки публикации, а не внутри неё");
    }

    #[test]
    fn losing_the_permission_empties_the_window_list() {
        // Без разрешения такт до окон не доходит вовсе, и оставшиеся пункты
        // называли бы окна, про которые трекер уже ничего не знает.
        let src = tracker_source();
        let branch = src.split("if !is_trusted {").nth(1).unwrap_or("");
        let head = branch.split("continue;").next().unwrap_or("");
        assert!(
            head.contains("windows.0.lock().unwrap().clear()"),
            "потеря разрешения обязана убирать список окон из меню"
        );
    }

    #[test]
    fn the_window_list_stands_under_the_grant_item() {
        // Место пункта «выдать разрешение» постоянное, и список обязан
        // вставать под ним: иначе пункт, который человек ищет глазами на одном
        // месте, уезжал бы вниз от каждого нового окна.
        let src = tracker_source();
        assert!(
            src.contains("let base = GRANT_POSITION + usize::from(want_grant)"),
            "список окон вставляется под пунктом grant, а не поверх него"
        );
    }

    #[test]
    fn both_ways_to_ask_for_a_layout_end_in_one_branch() {
        // Соль связки: пункт трея ничего не двигает сам — реестр окон не `Send`
        // и живёт в потоке трекера. Он кладёт в канал ту же просьбу, что
        // приезжает по MQTT. Разойдись эти дороги — раскладок стало бы две, и
        // чинить пришлось бы обе.
        let src = tracker_source();
        assert!(
            src.contains("fn ask(app: &tauri::AppHandle, mode: mwm_core::layout::Layout)"),
            "пункт меню просит, а не расставляет"
        );
        assert!(
            src.contains("mwm_core::request::Request::Arrange { mode, ids: Vec::new() }"),
            "просьба из меню — та же, что приезжает по MQTT"
        );
        assert_eq!(
            src.matches("Request::Arrange { mode, ids } =>").count(),
            1,
            "исполняет раскладку одна ветка"
        );
    }

    #[test]
    fn the_window_list_stands_under_the_layout_items() {
        // У кнопки место постоянное, а список растёт и тает от числа окон.
        // Забудь счёт пунктов раскладки — список встал бы поверх кнопки, и та
        // уезжала бы у человека из-под курсора от каждого нового окна.
        let src = tracker_source();
        assert!(
            src.contains("let base = GRANT_POSITION + usize::from(want_grant) + LAYOUT_ITEMS;"),
            "список окон вставляется под пунктами раскладки"
        );
        assert_eq!(
            src.matches("&tile, &cascade,").count(),
            2,
            "оба пункта стоят в обеих сборках меню — с разрешением и без"
        );
        assert_eq!(
            super::LAYOUT_ITEMS, 2,
            "счёт пунктов раскладки совпадает с их числом в меню"
        );
    }

    #[test]
    fn losing_the_permission_greys_out_the_layout_item() {
        // Без Accessibility двигать окна нечем, и нажатие ушло бы в пустоту:
        // жалоба легла бы в строку состояния, а человек ждал бы окон.
        let src = tracker_source();
        let branch = src.split("Some(true) => {").nth(1).unwrap_or("");
        let head = branch.split("Some(false)").next().unwrap_or("");
        assert!(
            head.contains("tile.set_enabled(false)") && head.contains("cascade.set_enabled(false)"),
            "оба пункта гаснут там же, где приходит пункт «выдать разрешение»"
        );
    }

    #[test]
    fn a_layout_without_windows_complains_instead_of_placing() {
        // Пустой порядок — не «разложил нечего», а «сессии есть, окон нет»:
        // молчание здесь неотличимо от сработавшей раскладки.
        let src = tracker_source();
        assert!(
            src.contains("return Some(\"place: no windows to place\".to_string());"),
            "пустой список окон отвечает жалобой"
        );
    }

    #[test]
    fn the_tray_has_a_settings_item() {
        let src = tracker_source();
        assert!(src.contains("\"settings\" =>"), "пункт settings обязан быть в обработчике меню");
    }

    #[test]
    fn the_action_items_carry_an_icon() {
        // Значок стоит перед подписью, а не вместо неё: меню читают глазами по
        // словам, а знак только помогает найти строку быстрее.
        let src = tracker_source();
        // Пробелы после значка — подгонка ширины под самый широкий знак; их
        // число разное и проверяется соседним тестом.
        let hair = "\\u{200a}";
        // Плитка проверяется не тем же способом: её подпись собирается
        // `format!` — она зависит от того, встал ли хоткей, — и литерала с
        // подписью в исходнике нет вовсе. Сторожится то же самое: значок с
        // подгонкой перед подстановкой, а подпись — там, где живёт.
        let tile_icon = "\\u{25a6}";
        let want = format!("\"{tile_icon}{hair} {{}}\"");
        assert!(src.contains(&want), "у пункта плитки нет значка с подгонкой: {want}");
        assert_eq!(mwm_core::status::tile_item_label(true), "Tile windows");
        for (icon, pad, label) in [
            ("\\u{2750}", 2, "Cascade windows"),
            ("\\u{2699}", 5, "Settings"),
            ("\\u{26bf}", 2, "Grant Accessibility…"),
            ("\\u{23fb}", 0, "Quit"),
        ] {
            let want = format!("\"{icon}{} {label}\"", hair.repeat(pad));
            assert!(src.contains(&want), "у пункта {label} нет значка с подгонкой: {want}");
        }
    }

    #[test]
    fn the_icons_line_the_labels_up_to_within_half_a_point() {
        // Ширины измерены на маке в шрифте меню (`.AppleSystemUIFont` 13 pt).
        // Тест сторожит арифметику подгонки: смени кто-нибудь число пробелов
        // на глаз — подписи разъедутся, а заметить это можно только на маке.
        const HAIR: f64 = 0.84;
        const WIDEST: f64 = 11.95; // ⏻, оно же самое широкое
        for (name, width, pad) in [
            ("tile", 11.24, 1),
            ("cascade", 9.91, 2),
            ("settings", 7.83, 5),
            ("grant", 10.62, 2),
            ("quit", 11.95, 0),
        ] {
            let off = width + HAIR * f64::from(pad) - WIDEST;
            assert!(off.abs() <= 0.5, "{name}: подпись съезжает на {off:.2} pt");
        }
    }

    #[test]
    fn the_tick_rereads_the_shared_config() {
        // Без этого сохранённый тумблер молчал бы до перезапуска, а молча не
        // подействовавший тумблер хуже отсутствующего. Проверяется текстом:
        // такт не разложить на чистые функции, не переписав файл целиком.
        let src = tracker_source();
        assert!(
            src.contains("let cfg = shared.get();"),
            "такт обязан брать конфиг из ячейки, а не из копии, прочитанной на старте"
        );
        assert!(
            src.contains("*shared.0.lock().unwrap() = load_config();"),
            "сохранение обязано класть в ячейку перечитанный с диска конфиг"
        );
    }

    #[test]
    fn focus_is_gated_by_the_requests_flag() {
        // Объявить умение поднимать окно, не собираясь его поднимать, значит
        // подарить человеку молчащий Enter в пикере — а это хуже открытого
        // терминала.
        let src = tracker_source();
        assert!(
            src.contains("let focus = link.is_live() && features.requests;"),
            "focus должен считаться от живого соединения И включённого флага"
        );
        // Отпечаток считается от того же значения: иначе выключенный тумблер
        // доехал бы до файла окон только со следующим сердцебиением, до
        // полуминуты спустя.
        assert!(src.contains("fingerprint(&bound, focus)"), "отпечаток берёт то же focus");
        assert!(src.contains("focus,\n"), "build_file получает то же focus");
    }

    #[test]
    fn the_window_file_goes_to_both_addresses() {
        // Половинки местного источника ходят парой: индекс без публикации
        // привязал бы сессию, чьё окно никто не прочитает, а публикация без
        // индекса написала бы файл, в котором местных сессий нет. Обе живут в
        // такте, то есть поведением их не поймать — только текстом.
        let src = tracker_source();
        assert!(
            src.contains("deliver::write_local(&local_windows_dir, &cfg.host, &payload)"),
            "тот же payload обязан лечь и рядом с собой, а не только уехать по ssh"
        );
        assert!(
            src.contains("if features.local_source {"),
            "местная публикация обязана слушаться своего флага"
        );
        // Отказы двух дорог не сливаются в один: чинят их в разных местах, и
        // общая жалоба отправила бы человека не туда.
        assert!(
            src.contains("local publish failed: {e}") && src.contains("local index unreadable: {e}"),
            "у местной записи и местного чтения свои жалобы"
        );
    }
}
