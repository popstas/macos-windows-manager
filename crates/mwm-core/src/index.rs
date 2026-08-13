//! Индекс «заголовок окна → id сессии» из дампа агрегатора.

use crate::title::strip_decoration;
use std::collections::BTreeMap;

/// «Очищенный заголовок → id сессии» из дампа агрегатора.
///
/// Недоверие к содержимому то же, что у файла окон: запись без id или без
/// заголовка не значит ничего и выбрасывается молча. Порченый дамп стоит
/// индекса, а не запуска — без индекса привязка живёт на прежних слотах.
pub fn parse_index(json: &str) -> BTreeMap<String, String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return BTreeMap::new();
    };
    let mut out: BTreeMap<String, (String, f64)> = BTreeMap::new();
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
        let activity = s.get("activityAt").and_then(|x| x.as_f64()).unwrap_or(0.0);
        // Тёзки: побеждает свежесть активности. Иначе только что открытая
        // сессия проигрывала бы суточной тёзке навсегда.
        match out.get(&key) {
            Some((_, prev)) if *prev >= activity => {}
            _ => {
                out.insert(key, (id.to_string(), activity));
            }
        }
    }
    out.into_iter().map(|(k, (id, _))| (k, id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaa-1111-2222-3333-444444444444";
    const B: &str = "bbbbbbbb-1111-2222-3333-444444444444";

    #[test]
    fn titles_map_to_sessions() {
        let idx = parse_index(&format!(
            r#"{{"sessions":[{{"id":"{A}","title":"ccfzf"}},{{"id":"{B}","title":"other"}}]}}"#
        ));
        assert_eq!(idx.get("ccfzf"), Some(&A.to_string()));
        assert_eq!(idx.get("other"), Some(&B.to_string()));
    }

    #[test]
    fn title_is_stored_stripped() {
        // Сравнивать будут с заголовком окна, а он приезжает со значком
        // состояния. Чистить одну сторону — значит не сойтись с другой.
        let idx = parse_index(&format!(r#"{{"sessions":[{{"id":"{A}","title":"✳ ccfzf"}}]}}"#));
        assert_eq!(idx.get("ccfzf"), Some(&A.to_string()));
    }

    #[test]
    fn twins_go_to_the_livelier_one() {
        // Два заголовка-тёзки законны. Побеждает тот, у кого свежее активность:
        // иначе только что открытая сессия проигрывала бы суточной тёзке
        // навсегда.
        let idx = parse_index(&format!(
            r#"{{"sessions":[{{"id":"{A}","title":"ccfzf","activityAt":10}},
                             {{"id":"{B}","title":"ccfzf","activityAt":99}}]}}"#
        ));
        assert_eq!(idx.get("ccfzf"), Some(&B.to_string()));
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
}
