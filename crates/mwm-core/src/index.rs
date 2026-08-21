//! Индекс «заголовок окна → сессия: id и каталог» из дампа агрегатора.

use crate::title::strip_decoration;
use std::collections::BTreeMap;

/// Сессия, какой её знает дамп агрегатора.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRef {
    pub id: String,
    /// Каталог проекта. Пустая строка — дамп его не назвал.
    pub cwd: String,
}

/// Чем сессия доказывает, что окно с этим заголовком принадлежит ей.
///
/// `activity` — отметка хука (`activityAt` в дампе): хук стучит на каждый вызов
/// инструмента работающего агента, поэтому свежая отметка — довод сильнее
/// любого другого. `mtime` — свежесть транскрипта, а у сессии, чей файл ещё не
/// заведён, момент старта её процесса.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rank {
    activity: f64,
    mtime: f64,
}

impl Rank {
    /// Главнее ли эта сессия прежней. Три случая, и различать их обязательно.
    ///
    /// - у обеих есть отметка — она и решает: `mtime` двигают служебные
    ///   записи, и по нему брошенная сессия выглядит свежее работающей;
    /// - ни у одной нет — хуков на машине не стоит, спорить не о чем, остаётся
    ///   свежесть файла;
    /// - у одной есть, у другой нет — сравнивать ноль с чужой отметкой как
    ///   число нельзя. Ноль значит «хук про эту сессию не писал», а не
    ///   «активности не было с 1970 года», и запись без отметки проигрывала по
    ///   одному факту её отсутствия: только что заведённая сессия отдавала
    ///   своё окно суточной тёзке навсегда. Сравнивается то, что есть у обеих.
    ///
    /// Два последних случая складываются в одно выражение: там, где отметки
    /// нет, `max` и так отдаёт `mtime`.
    ///
    /// То же правило и по той же причине живёт у соседнего трекера —
    /// `byActivityThen` в `windows11-manager/src/claude-wt/sessions-helpers.js`.
    /// Расхождение поведением не поймать: привязка проходит, окно у сессии
    /// есть, просто не у той.
    fn outranks(self, prev: Rank) -> bool {
        if self.activity > 0.0 && prev.activity > 0.0 {
            return self.activity > prev.activity;
        }
        self.activity.max(self.mtime) > prev.activity.max(prev.mtime)
    }
}

/// «Очищенный заголовок → сессия: id и каталог» из дампа агрегатора.
///
/// Недоверие к содержимому то же, что у файла окон: запись без id или без
/// заголовка не значит ничего и выбрасывается молча. Порченый дамп стоит
/// индекса, а не запуска — без индекса привязка живёт на прежних слотах.
pub fn parse_index(json: &str) -> BTreeMap<String, SessionRef> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return BTreeMap::new();
    };
    let mut out: BTreeMap<String, (SessionRef, Rank)> = BTreeMap::new();
    for s in v.get("sessions").and_then(|s| s.as_array()).into_iter().flatten() {
        let (Some(id), Some(title)) = (
            s.get("id").and_then(|x| x.as_str()),
            s.get("title").and_then(|x| x.as_str()),
        ) else {
            continue;
        };
        let key = strip_decoration(title);
        if key.is_empty() || id.is_empty() {
            continue;
        }
        let cwd = s.get("cwd").and_then(|x| x.as_str()).unwrap_or_default().to_string();
        let activity = s.get("activityAt").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let mtime = s.get("mtime").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let rank = Rank { activity, mtime };
        // Тёзки: побеждает свежесть. Иначе только что открытая сессия
        // проигрывала бы суточной тёзке навсегда.
        match out.get(&key) {
            Some((_, prev)) if !rank.outranks(*prev) => {}
            _ => {
                out.insert(key, (SessionRef { id: id.to_string(), cwd }, rank));
            }
        }
    }
    out.into_iter().map(|(k, (r, _))| (k, r)).collect()
}

/// Слить индекс удалённого дампа с индексом местного.
///
/// Тёзки между машинами законны: одно и то же имя сессии бывает и здесь, и
/// там, — и побеждает удалённая. Правило «побеждает более живая», которым
/// внутри одного дампа разводят тёзок, тут не годится: `activityAt` приезжает
/// из файлов хука, а на машине без хуков он нулевой у всех, то есть исход
/// решался бы не свежестью, а тем, стоят ли хуки. Вторая причина сильнее
/// первой: так трекер вёл себя до появления второго источника, и окно
/// ssh-сессии, чьё имя совпало с местной, привязывается ровно как раньше.
///
/// Цена названа: местная сессия-тёзка окна не получит. Случай редкий, а
/// ошибка в другую сторону — окно ssh-сессии, отданное местной, — стоила бы
/// пикеру неверного ▣ и Enter, поднимающего не ту машину.
pub fn merge_index(
    remote: BTreeMap<String, SessionRef>,
    local: BTreeMap<String, SessionRef>,
) -> BTreeMap<String, SessionRef> {
    let mut out = local;
    out.extend(remote);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaa-1111-2222-3333-444444444444";
    const B: &str = "bbbbbbbb-1111-2222-3333-444444444444";
    const C: &str = "cccccccc-1111-2222-3333-444444444444";
    const D: &str = "dddddddd-1111-2222-3333-444444444444";

    fn r(id: &str, cwd: &str) -> SessionRef {
        SessionRef { id: id.to_string(), cwd: cwd.to_string() }
    }

    #[test]
    fn titles_map_to_sessions() {
        let idx = parse_index(&format!(
            r#"{{"sessions":[{{"id":"{A}","title":"ccfzf"}},{{"id":"{B}","title":"other"}}]}}"#
        ));
        assert_eq!(idx.get("ccfzf"), Some(&r(A, "")));
        assert_eq!(idx.get("other"), Some(&r(B, "")));
    }

    #[test]
    fn title_is_stored_stripped() {
        // Сравнивать будут с заголовком окна, а он приезжает со значком
        // состояния. Чистить одну сторону — значит не сойтись с другой.
        let idx = parse_index(&format!(r#"{{"sessions":[{{"id":"{A}","title":"✳ ccfzf"}}]}}"#));
        assert_eq!(idx.get("ccfzf"), Some(&r(A, "")));
    }

    #[test]
    fn twins_go_to_the_livelier_one() {
        // Два заголовка-тёзки законны. Побеждает тот, у кого свежее активность:
        // иначе только что открытая сессия проигрывала бы суточной тёзке
        // навсегда. Порядок в документе взят в обе стороны — livelier запись
        // то первая, то вторая, — иначе тест доказывал бы только «выигрывает
        // последняя строка», что при этой конкретной раскладке данных
        // совпадает со «выигрывает более живая» чисто по счастливой
        // случайности порядка и не отличило бы правило от простой
        // перезаписи.
        let idx = parse_index(&format!(
            r#"{{"sessions":[{{"id":"{B}","title":"ccfzf","activityAt":99}},
                             {{"id":"{A}","title":"ccfzf","activityAt":10}},
                             {{"id":"{C}","title":"other","activityAt":10}},
                             {{"id":"{D}","title":"other","activityAt":99}}]}}"#
        ));
        assert_eq!(
            idx.get("ccfzf"),
            Some(&r(B, "")),
            "живее первая запись — безусловная перезапись последней строкой выбрала бы A"
        );
        assert_eq!(
            idx.get("other"),
            Some(&r(D, "")),
            "живее вторая запись — правило «никогда не перезаписывать первую» выбрало бы C"
        );
    }

    #[test]
    fn a_fresh_session_beats_a_namesake_with_a_cooled_stamp() {
        // Отметка хука бывает только у той сессии, которая успела вызвать хоть
        // один инструмент. У только что заведённой её нет вовсе, и сравнение
        // одних лишь `activityAt` отдавало окно тёзке навсегда — по одному
        // факту отсутствия отметки, а не по свежести.
        //
        // Снято живьём 2026-08-22 на паре `ExpertizeMe`: у работающей
        // `activityAt: 0` и `mtime` момента старта процесса (04:12:31), у
        // двадцатиминутной тёзки — отметка 04:04:25. Окно ушло к старой,
        // агрегатор от этого пометил её живой (`live |= set(windows)`), и в
        // пикере она встала с чужими сообщениями и чужим возрастом. Ту же
        // ошибку и тем же правилом чинил у себя windows11-manager
        // (`byActivityThen` в `sessions-helpers.js`, замер 2026-08-12).
        //
        // Порядок записей взят в обе стороны: иначе тест доказывал бы
        // «выигрывает последняя строка», а не само правило.
        let fresh = format!(r#"{{"id":"{A}","title":"ExpertizeMe","activityAt":0,"mtime":1787353951}}"#);
        let cooled =
            format!(r#"{{"id":"{B}","title":"ExpertizeMe","activityAt":1787353465,"mtime":1787353549}}"#);
        let doc = |first: &String, second: &String| format!(r#"{{"sessions":[{first},{second}]}}"#);
        assert_eq!(
            parse_index(&doc(&fresh, &cooled)).get("ExpertizeMe"),
            Some(&r(A, "")),
            "новая сессия названа первой"
        );
        assert_eq!(
            parse_index(&doc(&cooled, &fresh)).get("ExpertizeMe"),
            Some(&r(A, "")),
            "новая сессия названа второй"
        );
    }

    #[test]
    fn two_stamps_are_compared_between_themselves() {
        // Оговорка к правилу выше, и она обязательна. Отметка хука — довод
        // сильнее свежести файла: `mtime` двигают служебные записи, и по нему
        // давно брошенная сессия выглядит свежее работающей. Поэтому там, где
        // отметка есть у обеих, решает она, а `mtime` не заглядывает вовсе.
        let idx = parse_index(&format!(
            r#"{{"sessions":[{{"id":"{A}","title":"ccfzf","activityAt":99,"mtime":10}},
                             {{"id":"{B}","title":"ccfzf","activityAt":10,"mtime":9999}}]}}"#
        ));
        assert_eq!(
            idx.get("ccfzf"),
            Some(&r(A, "")),
            "свежая отметка главнее чужого mtime"
        );
    }

    #[test]
    fn garbage_costs_itself_and_nothing_more() {
        // Порченый дамп стоит индекса, а не запуска: без индекса привязка живёт
        // на прежних слотах, а без трекера не живёт ничего.
        assert!(parse_index("not json at all").is_empty());
        assert!(parse_index(r#"{"sessions":"nope"}"#).is_empty());
        let idx = parse_index(&format!(
            r#"{{"sessions":[{{"id":42}},{{"title":"x"}},{{"id":"{A}","title":"ok"}}]}}"#
        ));
        assert_eq!(idx.len(), 1);
    }

    #[test]
    fn blank_title_or_id_is_skipped_but_document_still_parses() {
        // Другой сорт порчи, чем «поля нет вовсе»: поля на месте, но после
        // очистки ничего не значат — заголовок из одного значка, id пустой
        // строкой. Запись стоит себя, а не индекса; остальной документ обязан
        // разобраться как обычно.
        let idx = parse_index(&format!(
            r#"{{"sessions":[{{"id":"{A}","title":"✳   "}},
                             {{"id":"","title":"nameless"}},
                             {{"id":"{B}","title":"ok"}}]}}"#
        ));
        assert!(
            idx.get("").is_none(),
            "пустой после очистки заголовок не должен всплыть под пустым ключом"
        );
        assert!(
            idx.get("nameless").is_none(),
            "запись с пустым id не должна попасть в индекс"
        );
        assert_eq!(idx.get("ok"), Some(&r(B, "")));
        assert_eq!(idx.len(), 1, "порченые записи не в счёт — в индексе только валидная");
    }

    #[test]
    fn the_working_directory_travels_with_the_session() {
        // Каталог нужен снимку: сессия из снимка может быть уже неизвестна
        // агрегатору — снимки затем и есть, — и пикеру взять каталог будет
        // неоткуда.
        let idx = parse_index(&format!(
            r#"{{"sessions":[{{"id":"{A}","title":"ccfzf","cwd":"~/projects/js/ccfzf-picker"}}]}}"#
        ));
        assert_eq!(idx.get("ccfzf"), Some(&r(A, "~/projects/js/ccfzf-picker")));
    }

    #[test]
    fn the_two_indexes_fill_each_other_in() {
        // Половинки складываются: местная сессия доезжает до привязки, не
        // отбирая ничего у удалённых.
        let remote = parse_index(&format!(r#"{{"sessions":[{{"id":"{A}","title":"remote one"}}]}}"#));
        let local = parse_index(&format!(r#"{{"sessions":[{{"id":"{B}","title":"local one"}}]}}"#));
        let idx = merge_index(remote, local);
        assert_eq!(idx.get("remote one"), Some(&r(A, "")));
        assert_eq!(idx.get("local one"), Some(&r(B, "")));
        assert_eq!(idx.len(), 2);
    }

    #[test]
    fn on_a_name_clash_the_remote_session_wins() {
        // Так трекер вёл себя до второго источника, и окно ssh-сессии обязано
        // привязываться как раньше. Свежесть тут не судья: `activityAt` на
        // машине без хуков нулевой у всех, и «более живая» решала бы исход
        // наличием хуков, а не свежестью.
        let remote = parse_index(&format!(
            r#"{{"sessions":[{{"id":"{A}","title":"ccfzf","activityAt":1}}]}}"#
        ));
        let local = parse_index(&format!(
            r#"{{"sessions":[{{"id":"{B}","title":"ccfzf","activityAt":99}}]}}"#
        ));
        assert_eq!(merge_index(remote, local).get("ccfzf"), Some(&r(A, "")));
    }

    #[test]
    fn an_empty_half_costs_nothing() {
        // Обе стороны бывают пустыми по-честному: местного агрегатора нет
        // вовсе, удалённый не дочитался. Слияние обязано отдать вторую целой.
        let remote = parse_index(&format!(r#"{{"sessions":[{{"id":"{A}","title":"only"}}]}}"#));
        assert_eq!(merge_index(remote.clone(), BTreeMap::new()), remote);
        assert_eq!(merge_index(BTreeMap::new(), remote.clone()), remote);
        assert!(merge_index(BTreeMap::new(), BTreeMap::new()).is_empty());
    }

    #[test]
    fn a_session_without_a_directory_is_still_a_session() {
        // Каталога может не быть вовсе — это не повод терять привязку окна.
        let idx = parse_index(&format!(r#"{{"sessions":[{{"id":"{A}","title":"ccfzf","cwd":17}}]}}"#));
        assert_eq!(idx.get("ccfzf"), Some(&r(A, "")));
    }
}
