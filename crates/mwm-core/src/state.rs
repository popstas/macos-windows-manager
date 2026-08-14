//! Слоты сессий на диске: где стояло окно, как называлось, из какого каталога.
//!
//! Файл машинный, конфиг человеческий — лежат они поэтому в разных местах.
//! Соседство приглашало бы спутать резервную копию одного с рабочим файлом
//! другого.

use crate::geometry::Bounds;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;

/// Что помнится про сессию между запусками.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SlotState {
    /// Устойчивое положение окна. `None` — сессию видели, а координат не знаем.
    pub bounds: Option<Bounds>,
    pub title: String,
    /// Каталог проекта. Нужен снимку: сессия из снимка может быть уже
    /// неизвестна агрегатору, и пикеру взять каталог будет неоткуда.
    pub cwd: String,
    pub last_seen_ms: u64,
    /// Отметка взгляда. Переживает перезапуск намеренно: без этого перезапуск
    /// трекера показал бы человеку все сессии непрочитанными разом, а отметку
    /// эту ставил взгляд, а не случай.
    pub focused_at_ms: u64,
}

/// Версия формата. Пишется, но не проверяется: читатель здесь один и тот же
/// процесс, а поле пригодится тому, кто будет разбирать файл руками.
const VERSION: u64 = 1;

fn bounds_from(v: &serde_json::Value) -> Option<Bounds> {
    let o = v.as_object()?;
    let n = |k: &str| o.get(k).and_then(|x| x.as_i64()).and_then(|x| i32::try_from(x).ok());
    Some(Bounds { x: n("x")?, y: n("y")?, width: n("width")?, height: n("height")? })
}

pub fn parse_state(json: &str) -> BTreeMap<String, SlotState> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return BTreeMap::new();
    };
    let Some(slots) = v.get("slots").and_then(|s| s.as_object()) else {
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    for (sid, rec) in slots {
        if sid.is_empty() {
            continue;
        }
        let text = |k: &str| {
            rec.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string()
        };
        let num = |k: &str| rec.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        out.insert(
            sid.clone(),
            SlotState {
                bounds: rec.get("bounds").and_then(bounds_from),
                title: text("title"),
                cwd: text("cwd"),
                last_seen_ms: num("lastSeen"),
                focused_at_ms: num("focusedAt"),
            },
        );
    }
    out
}

pub fn state_json(slots: &BTreeMap<String, SlotState>) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for (sid, s) in slots {
        out.insert(
            sid.clone(),
            json!({
                "bounds": s.bounds.map(|b| json!({
                    "x": b.x, "y": b.y, "width": b.width, "height": b.height
                })),
                "title": s.title,
                "cwd": s.cwd,
                "lastSeen": s.last_seen_ms,
                "focusedAt": s.focused_at_ms,
            }),
        );
    }
    json!({ "version": VERSION, "slots": out })
}

/// Куда отодвинуть порченый файл, не тронув отодвинутый прежде.
///
/// Первая порча обычно и есть первопричина, а разбираться приходят уже после
/// второй: затирай мы `.bak` каждым новым сбоем, второй уносил бы уцелевшую
/// копию первого. Нумерованных копий держим девять — каталог состояния не
/// должен расти без предела, и когда все заняты, девятой не жалко.
fn aside_path(path: &Path) -> std::path::PathBuf {
    let first = path.with_extension("json.bak");
    if !first.exists() {
        return first;
    }
    for n in 1..9 {
        let next = path.with_extension(format!("json.bak.{n}"));
        if !next.exists() {
            return next;
        }
    }
    path.with_extension("json.bak.9")
}

pub fn read_state(path: &Path) -> BTreeMap<String, SlotState> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    if serde_json::from_str::<serde_json::Value>(&text).is_err() {
        // Отодвигаем, а не удаляем: трекер обязан подняться, но байты могут
        // ещё пригодиться тому, кто будет разбираться.
        let bak = aside_path(path);
        if let Err(e) = std::fs::rename(path, &bak) {
            eprintln!("mwm: broken state file, and moving it aside failed: {e}");
        } else {
            eprintln!("mwm: broken state file, moved to {}", bak.display());
        }
        return BTreeMap::new();
    }
    parse_state(&text)
}

/// Атомарная запись: временный файл рядом, `fsync`, потом переименование.
///
/// `fsync` — это и есть смысл упражнения. Переименование журналируется, а
/// данные нет, и без него потеря питания оставляет рваный файл; рваный файл
/// стоит запомненной раскладки, ради которой всё и затевалось.
pub fn write_atomic(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let dir = path.parent().ok_or("state path has no parent directory")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("create state dir: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    let done = through_temp(&tmp, path, value);
    if done.is_err() {
        // Итоговый файл отказ не тронул — ради этого временный и заводится.
        // Но сам он оставаться не должен: иначе каталог состояния копил бы по
        // `.tmp` на каждый отказ, и разбираться в них было бы некому.
        let _ = std::fs::remove_file(&tmp);
    }
    done
}

fn through_temp(tmp: &Path, path: &Path, value: &serde_json::Value) -> Result<(), String> {
    use std::io::Write;
    {
        let mut f = std::fs::File::create(tmp).map_err(|e| format!("create temp state: {e}"))?;
        f.write_all(value.to_string().as_bytes())
            .map_err(|e| format!("write temp state: {e}"))?;
        f.sync_all().map_err(|e| format!("fsync temp state: {e}"))?;
    }
    std::fs::rename(tmp, path).map_err(|e| format!("rename temp state: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID: &str = "aaaaaaaa-1111-2222-3333-444444444444";

    fn slot() -> SlotState {
        SlotState {
            bounds: Some(Bounds { x: 10, y: 20, width: 800, height: 600 }),
            title: "ccfzf".to_string(),
            cwd: "~/projects/js/ccfzf-picker".to_string(),
            last_seen_ms: 5_000,
            focused_at_ms: 4_000,
        }
    }

    #[test]
    fn a_slot_survives_a_round_trip() {
        let mut slots = BTreeMap::new();
        slots.insert(SID.to_string(), slot());
        let back = parse_state(&state_json(&slots).to_string());
        assert_eq!(back, slots);
    }

    #[test]
    fn a_slot_without_bounds_survives_too() {
        // Сессию видели, а координат не узнали: окно закрылось раньше, чем
        // положение устоялось. Терять запись из-за этого нельзя — в ней ещё
        // каталог и отметка взгляда.
        let mut slots = BTreeMap::new();
        slots.insert(SID.to_string(), SlotState { bounds: None, ..slot() });
        let back = parse_state(&state_json(&slots).to_string());
        assert_eq!(back[SID].bounds, None);
        assert_eq!(back[SID].cwd, "~/projects/js/ccfzf-picker");
    }

    #[test]
    fn garbage_costs_itself_and_nothing_more() {
        // Недоверие к файлу то же, что у файла окон: порченая запись стоит
        // себя, а не всего состояния.
        assert!(parse_state("not json").is_empty());
        assert!(parse_state(r#"{"slots":"nope"}"#).is_empty());
        let back = parse_state(&format!(
            r#"{{"slots":{{"{SID}":{{"title":"ok","bounds":{{"x":1,"y":2,"width":"нет","height":4}}}},
                          "":{{"title":"безымянный"}}}}}}"#
        ));
        assert_eq!(back.len(), 1, "запись с пустым ключом не в счёт");
        assert_eq!(back[SID].bounds, None, "порченые координаты стоят координат, а не записи");
        assert_eq!(back[SID].title, "ok");
    }

    #[test]
    fn writing_is_atomic_and_leaves_no_temp_behind() {
        // Временный файл рядом, потом переименование. Останься он лежать —
        // каталог состояния копил бы мусор, а разбираться в нём было бы некому.
        let dir = std::env::temp_dir().join(format!("mwm-state-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.json");
        let mut slots = BTreeMap::new();
        slots.insert(SID.to_string(), slot());
        write_atomic(&path, &state_json(&slots)).expect("запись обязана удаться");
        assert_eq!(read_state(&path), slots);
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left.len(), 1, "рядом с файлом ничего не осталось: {left:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_broken_file_is_moved_aside_and_the_tracker_starts() {
        // Рваный файл не должен мешать старту: раскладка забыта, работа
        // продолжается. Но байты отодвигаются, а не удаляются — они могут ещё
        // пригодиться тому, кто будет разбираться.
        let dir = std::env::temp_dir().join(format!("mwm-state-broken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        std::fs::write(&path, "{ порвано").unwrap();
        assert!(read_state(&path).is_empty());
        assert!(dir.join("state.json.bak").exists(), "байты отодвинуты, а не выброшены");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_write_takes_its_temp_file_with_it() {
        // Отказ записи оставляет итоговый файл целым — это и есть смысл
        // временного файла. Но сам он оставаться не должен: каталог состояния
        // копил бы по `.tmp` на каждый отказ, и разбираться в них было бы
        // некому.
        //
        // Отказ подстроен каталогом на месте файла состояния: временный файл
        // создастся и запишется, а переименование поверх каталога не выйдет.
        let dir = std::env::temp_dir().join(format!("mwm-state-failed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.json");
        std::fs::create_dir_all(&path).unwrap();
        let mut slots = BTreeMap::new();
        slots.insert(SID.to_string(), slot());
        assert!(write_atomic(&path, &state_json(&slots)).is_err(), "переименование не удалось");
        assert!(!dir.join("state.json.tmp").exists(), "временный файл убран за собой");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_second_breakage_does_not_eat_the_first_backup() {
        // Байты отодвигаются, чтобы человек мог в них разобраться. Второй сбой
        // не должен уносить копию первого: первая порча обычно и есть
        // первопричина, а разбираться приходят уже после второй.
        let dir = std::env::temp_dir().join(format!("mwm-state-twice-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        std::fs::write(&path, "{ порвано первый раз").unwrap();
        assert!(read_state(&path).is_empty());
        std::fs::write(&path, "{ порвано второй раз").unwrap();
        assert!(read_state(&path).is_empty());
        assert_eq!(
            std::fs::read_to_string(dir.join("state.json.bak")).unwrap(),
            "{ порвано первый раз",
            "первая порча пережила вторую"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("state.json.bak.1")).unwrap(),
            "{ порвано второй раз",
            "вторая легла рядом, а не поверх"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_is_an_empty_state_not_an_error() {
        let path = std::env::temp_dir().join("mwm-state-does-not-exist-at-all.json");
        let _ = std::fs::remove_file(&path);
        assert!(read_state(&path).is_empty());
    }
}
