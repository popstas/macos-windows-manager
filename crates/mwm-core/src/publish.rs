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
/// затеи: читатель уже умеет его разбирать. Отличий пять, и все объявлены.
/// `desktop` всегда `null` — программного интерфейса к Spaces у macOS нет.
/// `snapshots` и `projects` пусты — первое отложено, второе живёт на
/// Windows-стороне. `focus` говорит, умеет ли этот трекер поднимать окно; на
/// этом этапе он не умеет, и без такого признания пикер предложил бы человеку
/// молчащий Enter. `openSession` всегда `false` — терминалы на маке открывает
/// сам пикер. `mqttBase` называет адрес, на который просить: топик живёт в
/// конфиге трекера, а публикует читатель.
///
/// `focus` и `mqttBase` объявляются по-разному, и это не случайный разнобой.
/// `can_focus` зовущий берёт из `link.is_live()` — обещание «подниму окно»
/// имеет смысл ровно пока соединение с брокером живо, и лживое `true` на
/// мёртвой связи выглядело бы для пикера сработавшим Enter, который ничего
/// не сделал. `mqtt_base`, наоборот, берётся из конфига всегда, живо
/// соединение или нет: это не обещание действия, а справочная запись «куда
/// писать, если когда-нибудь будет кому», и знание адреса не зависит от
/// того, поднята ли подписка в эту секунду. Единственный потребитель
/// адреса, не проверяющий `canFocus` на своей стороне, — «Mark unread» в
/// пикере, и это осознанно: отметка приезжает в список на любой машине.
/// Значит у трекера с настроенным, но недоступным брокером просьба «Mark
/// unread» уйдёт на объявленный адрес и потеряется так же, как терялась бы
/// на своей базе, — вреда почти нет, только не-починка.
///
/// Время наружу уезжает в секундах: читатель сравнивает `generated` со своим
/// «сейчас», а оно у него в секундах.
pub fn build_file(
    bound: &BTreeMap<String, Bound>,
    host: &str,
    pid: u32,
    now_ms: u64,
    can_focus: bool,
    mqtt_base: &str,
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
        // Берётся ли этот менеджер открывать сессии и терминалы. Не берётся, и
        // это не заготовка на будущее: на маке терминал открывает сам пикер, и
        // открывает верно. Объяви трекер обратное — пикер на маке увёл бы к
        // нему `claude-session-open`, а разбирать её здесь некому.
        "openSession": false,
        // Куда просить. Пустая строка значит «спроси свой конфиг»: так вёл себя
        // читатель до появления поля, и так он обязан вести себя с трекером
        // прежней версии.
        "mqttBase": mqtt_base,
        "windows": windows,
        "snapshots": [],
        "projects": [],
    })
}

/// Отпечаток расклада — без времени.
///
/// Считается по тому, что читатель увидит как изменение: набор сессий, их
/// заголовки, отметка взгляда и умение поднимать окно. `lastSeen` в него не
/// входит намеренно — он растёт каждый такт, и включив его, мы получили бы
/// отпечаток, который всегда разный.
///
/// `can_focus` подмешан отдельным байтом, а не как часть цикла по сессиям:
/// поле `focus` в файле — про машину целиком, не про конкретную сессию, и
/// значение не должно путаться с содержимым чьего-то заголовка. Без него
/// смена `focus` при неизменном раскладе окон не меняла бы отпечаток, и
/// `should_write` отложила бы запись файла до `HEARTBEAT_MS` — на живой
/// проверке «выключи брокер — Enter снова открывает терминал» это выглядит
/// как задержка до получаса и легко принимается за неработающую функцию.
pub fn fingerprint(bound: &BTreeMap<String, Bound>, can_focus: bool) -> String {
    let mut out = String::new();
    out.push(if can_focus { '\u{3}' } else { '\u{4}' });
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
        let v = build_file(&bound("ccfzf", 60_000), "mac-host", 7, 60_000, false, "");
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
    fn file_names_the_address_of_this_machine() {
        // Читателю неоткуда узнать, куда просить о подъёме: топик живёт в
        // конфиге трекера, а публикует читатель. Поэтому адрес называет тот,
        // кто его знает.
        let v = build_file(&bound("ccfzf", 60_000), "mac-host", 7, 60_000, true,
                           "home/room/mac/windows");
        assert_eq!(v["mqttBase"], "home/room/mac/windows");
        assert_eq!(v["focus"], true);
    }

    #[test]
    fn this_machine_does_not_open_sessions() {
        // Терминалы на маке открывает сам пикер, и это работает. Объявив
        // обратное, трекер увёл бы к себе просьбу `claude-session-open`,
        // которую здесь никто не разбирает, — и Enter замолчал бы.
        let v = build_file(&bound("ccfzf", 60_000), "mac-host", 7, 60_000, true, "");
        assert_eq!(v["openSession"], false);
    }

    #[test]
    fn an_unset_broker_leaves_the_address_empty() {
        // Пустая строка читается агрегатором как «спроси свой конфиг» — так
        // себя вёл читатель до появления поля.
        let v = build_file(&bound("ccfzf", 60_000), "mac-host", 7, 60_000, false, "");
        assert_eq!(v["mqttBase"], "");
        assert_eq!(v["focus"], false);
    }

    #[test]
    fn fingerprint_ignores_time() {
        // `generated` меняется на каждом такте. Отпечаток, включивший его, не
        // сэкономил бы ни одной записи, а выглядело бы это работающей
        // экономией.
        assert_eq!(
            fingerprint(&bound("ccfzf", 1_000), true),
            fingerprint(&bound("ccfzf", 90_000), true),
        );
    }

    #[test]
    fn fingerprint_notices_a_new_title() {
        assert_ne!(
            fingerprint(&bound("ccfzf", 1_000), true),
            fingerprint(&bound("other", 1_000), true),
        );
    }

    #[test]
    fn fingerprint_notices_a_focus_flip() {
        // Раскладка окон та же самая, меняется только умение поднимать окно —
        // ровно то, что происходит при потере и восстановлении связи с
        // брокером. Не различи отпечаток эти два случая, `should_write`
        // отложила бы запись файла до `HEARTBEAT_MS` (тридцать секунд), и на
        // живой проверке «выключи брокер — Enter снова открывает терминал»
        // это выглядело бы неработающей функцией, а не просто задержкой.
        assert_ne!(
            fingerprint(&bound("ccfzf", 1_000), true),
            fingerprint(&bound("ccfzf", 1_000), false),
        );
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
