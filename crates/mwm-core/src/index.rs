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

/// «Очищенный заголовок → сессия: id и каталог» из дампа агрегатора.
///
/// Недоверие к содержимому то же, что у файла окон: запись без id или без
/// заголовка не значит ничего и выбрасывается молча. Порченый дамп стоит
/// индекса, а не запуска — без индекса привязка живёт на прежних слотах.
pub fn parse_index(json: &str) -> BTreeMap<String, SessionRef> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return BTreeMap::new();
    };
    let mut out: BTreeMap<String, (SessionRef, f64)> = BTreeMap::new();
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
        // Тёзки: побеждает свежесть активности. Иначе только что открытая
        // сессия проигрывала бы суточной тёзке навсегда.
        match out.get(&key) {
            Some((_, prev)) if *prev >= activity => {}
            _ => {
                out.insert(key, (SessionRef { id: id.to_string(), cwd }, activity));
            }
        }
    }
    out.into_iter().map(|(k, (r, _))| (k, r)).collect()
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
    fn a_session_without_a_directory_is_still_a_session() {
        // Каталога может не быть вовсе — это не повод терять привязку окна.
        let idx = parse_index(&format!(r#"{{"sessions":[{{"id":"{A}","title":"ccfzf","cwd":17}}]}}"#));
        assert_eq!(idx.get("ccfzf"), Some(&r(A, "")));
    }
}
