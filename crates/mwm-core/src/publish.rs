//! Файл окон: что уезжает читателю и когда.

use crate::tracker::Bound;
use serde_json::json;
use std::collections::BTreeMap;

/// Как часто файл переписывается, когда расклад не менялся.
///
/// Читатель судит о свежести по полю `generated` и только по нему: mtime не
/// отличает «демон умер» от «ничего не менялось», а разница между этими
/// случаями — вся суть файла.
pub const HEARTBEAT_MS: u64 = 30_000;

/// Публикуемый вид: у какой сессии открыто окно, на какой машине и когда на
/// него смотрели.
///
/// Формат — тот же, что у Windows-трекера, и это не совпадение, а условие
/// затеи: читатель уже умеет его разбирать. Отличий три, и все объявлены.
/// `desktop` всегда `null` — программного интерфейса к Spaces у macOS нет.
/// `snapshots` и `projects` пусты — первое отложено, второе живёт на
/// Windows-стороне. `focus` говорит, умеет ли этот трекер поднимать окно; на
/// этом этапе он не умеет, и без такого признания пикер предложил бы человеку
/// молчащий Enter.
///
/// Время наружу уезжает в секундах: читатель сравнивает `generated` со своим
/// «сейчас», а оно у него в секундах.
pub fn build_file(
    bound: &BTreeMap<String, Bound>,
    host: &str,
    pid: u32,
    now_ms: u64,
    can_focus: bool,
) -> serde_json::Value {
    let mut windows = serde_json::Map::new();
    for (sid, b) in bound {
        windows.insert(
            sid.clone(),
            json!({
                "title": b.title,
                "desktop": serde_json::Value::Null,
                "lastSeen": b.last_seen_ms / 1000,
                "focusedAt": if b.focused_at_ms == 0 { 0 } else { b.focused_at_ms / 1000 },
            }),
        );
    }
    json!({
        "generated": now_ms / 1000,
        "host": host,
        "pid": pid,
        "focus": can_focus,
        "windows": windows,
        "snapshots": [],
        "projects": [],
    })
}

/// Отпечаток расклада — без времени.
///
/// Считается по тому, что читатель увидит как изменение: набор сессий, их
/// заголовки и отметка взгляда. `lastSeen` в него не входит намеренно — он
/// растёт каждый такт, и включив его, мы получили бы отпечаток, который всегда
/// разный.
pub fn fingerprint(bound: &BTreeMap<String, Bound>) -> String {
    let mut out = String::new();
    for (sid, b) in bound {
        out.push_str(sid);
        out.push('\u{1}');
        out.push_str(&b.title);
        out.push('\u{1}');
        out.push_str(&b.focused_at_ms.to_string());
        out.push('\u{2}');
    }
    out
}

/// Писать ли файл на этом такте.
pub fn should_write(
    fingerprint: &str,
    last: Option<&str>,
    last_write_ms: u64,
    now_ms: u64,
) -> bool {
    match last {
        None => true,
        Some(prev) if prev != fingerprint => true,
        _ => now_ms.saturating_sub(last_write_ms) >= HEARTBEAT_MS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::Bound;
    use std::collections::BTreeMap;

    const SID: &str = "aaaaaaaa-1111-2222-3333-444444444444";

    fn bound(title: &str, last_seen_ms: u64) -> BTreeMap<String, Bound> {
        let mut m = BTreeMap::new();
        m.insert(SID.to_string(), Bound {
            session_id: SID.to_string(),
            title: title.to_string(),
            last_seen_ms,
            focused_at_ms: 9_000,
        });
        m
    }

    #[test]
    fn file_shape_matches_what_the_reader_expects() {
        // Формат — тот же, что у Windows-трекера, и это условие всей затеи:
        // читатель уже умеет его разбирать, и переучивать его не пришлось.
        // Время в файле — в секундах: читатель сравнивает `generated` со своим
        // «сейчас», а оно у него в секундах.
        let v = build_file(&bound("ccfzf", 60_000), "mac-host", 7, 60_000, false);
        assert_eq!(v["host"], "mac-host");
        assert_eq!(v["pid"], 7);
        assert_eq!(v["generated"], 60);
        assert_eq!(v["focus"], false);
        assert_eq!(v["windows"][SID]["title"], "ccfzf");
        assert_eq!(v["windows"][SID]["lastSeen"], 60);
        assert_eq!(v["windows"][SID]["focusedAt"], 9);
        // Виртуальных столов у macOS программно нет, и подменять их нечем.
        assert!(v["windows"][SID]["desktop"].is_null());
        // Снимки и хоткеи — не этого этапа и не этой машины, но ключи должны
        // быть: читатель разбирает их терпимо, а вот отсутствие разберёт как
        // «трекер прежней версии» и промолчит.
        assert!(v["snapshots"].as_array().unwrap().is_empty());
        assert!(v["projects"].as_array().unwrap().is_empty());
    }

    #[test]
    fn fingerprint_ignores_time() {
        // `generated` меняется на каждом такте. Отпечаток, включивший его, не
        // сэкономил бы ни одной записи, а выглядело бы это работающей
        // экономией.
        assert_eq!(fingerprint(&bound("ccfzf", 1_000)), fingerprint(&bound("ccfzf", 90_000)));
    }

    #[test]
    fn fingerprint_notices_a_new_title() {
        assert_ne!(fingerprint(&bound("ccfzf", 1_000)), fingerprint(&bound("other", 1_000)));
    }

    #[test]
    fn heartbeat_writes_even_when_nothing_changed() {
        // Читатель судит о живости по `generated` и только по нему. Без
        // сердцебиения отметка залипала бы у здорового трекера, и читатель
        // погасил бы пометки об окнах, которые открыты.
        assert!(!should_write("abc", Some("abc"), 1_000, 1_000 + HEARTBEAT_MS - 1));
        assert!(should_write("abc", Some("abc"), 1_000, 1_000 + HEARTBEAT_MS));
    }

    #[test]
    fn change_writes_at_once() {
        assert!(should_write("abc", Some("xyz"), 1_000, 1_001));
        assert!(should_write("abc", None, 0, 1), "первая запись обязана состояться");
    }
}
