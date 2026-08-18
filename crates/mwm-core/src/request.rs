//! Просьбы, приезжающие по MQTT: что просят и о какой сессии.
//!
//! Разбор живёт здесь, а не рядом с подпиской, по той же причине, по какой
//! здесь живёт весь `mwm-core`: тесты этого крейта гоняются на любой машине, а
//! на маке их гонять неудобно.

/// О чём просят.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Поднять окно этой сессии.
    Focus(String),
    /// Вернуть сессию в непрочитанное.
    Unread(String),
    /// Разложить окна. Порядок сессий — тот, в котором их видит просящий;
    /// пустой список значит «все ведомые окна, порядком этой машины».
    Arrange { mode: Layout, ids: Vec<String> },
}

use crate::layout::Layout;

/// Имя команды — хвост топика после базы.
pub fn command_from_topic<'a>(topic: &'a str, base: &str) -> Option<&'a str> {
    let rest = topic.strip_prefix(base)?.strip_prefix('/')?;
    if rest.is_empty() || rest.contains('/') {
        return None;
    }
    Some(rest)
}

/// Просьба из имени команды и тела.
///
/// Тело разбирается после того, как узнана команда, а не до: у просьбы о
/// раскладке в теле не сессия, а раскладка со списком, и общий разбор `id`
/// отказал бы ей раньше, чем до неё дошла очередь.
pub fn parse_request(command: &str, payload: &str) -> Option<Request> {
    match command {
        "claude-focus" => Some(Request::Focus(id_from_payload(payload)?)),
        "claude-session-unread" => Some(Request::Unread(id_from_payload(payload)?)),
        "claude-place" => parse_arrange(payload),
        _ => None,
    }
}

/// Просьба о раскладке: `{"mode": …, "ids": [...]}`, json-строка и сырая
/// строка — теми же тремя видами, что и просьба о сессии, и по той же причине.
///
/// Список сессий необязателен: панель шлёт одно имя раскладки, а порядок у неё
/// взяться неоткуда. Пустой список не отказ, а «разложи всё, что ведёшь».
fn parse_arrange(payload: &str) -> Option<Request> {
    let text = payload.trim();
    if text.is_empty() {
        return None;
    }
    let (name, ids) = match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Object(map)) => {
            let name = map.get("mode")?.as_str()?.to_string();
            let ids = map
                .get("ids")
                .and_then(|v| v.as_array())
                .map(|list| {
                    list.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            (name, ids)
        }
        Ok(serde_json::Value::String(s)) => (s, Vec::new()),
        _ => (text.to_string(), Vec::new()),
    };
    Some(Request::Arrange { mode: Layout::from_name(&name)?, ids })
}

/// Id сессии из тела: из объекта `{"id": …}`, из json-строки и из сырой строки.
///
/// Три вида, а не один, потому что топики у нас общие с windows11-manager, а
/// туда с панели openHASP прилетает сырая строка. Разойтись с соседом в разборе
/// одного и того же топика — это отладка сразу на двух машинах.
fn id_from_payload(payload: &str) -> Option<String> {
    let text = payload.trim();
    if text.is_empty() {
        return None;
    }
    let id = match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Object(map)) => map.get("id")?.as_str()?.trim().to_string(),
        Ok(serde_json::Value::String(s)) => s.trim().to_string(),
        // Не json вовсе — значит сырая строка, как её шлёт панель.
        _ => text.to_string(),
    };
    if id.is_empty() { None } else { Some(id) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID: &str = "aaaaaaaa-1111-2222-3333-444444444444";
    const BASE: &str = "home/room/mac/windows";

    #[test]
    fn command_is_the_tail_of_the_topic() {
        assert_eq!(command_from_topic("home/room/mac/windows/claude-focus", BASE), Some("claude-focus"));
    }

    #[test]
    fn a_foreign_base_is_not_ours() {
        // Подписка стоит на своей базе, но брокер один на установку, и путать
        // соседнюю машину со своей нельзя: просьбу разобрали бы оба менеджера.
        assert_eq!(command_from_topic("home/room/pc/windows/claude-focus", BASE), None);
    }

    #[test]
    fn a_tail_with_a_slash_is_not_a_command() {
        // Подписка идёт на `<base>/#`. Всё, что глубже одного уровня, — не
        // просьба, а чьё-то эхо. То же правило, что у commandFromTopic в
        // windows11-manager.
        assert_eq!(command_from_topic("home/room/mac/windows/claude/slot/1", BASE), None);
    }

    #[test]
    fn the_base_itself_is_not_a_command() {
        assert_eq!(command_from_topic(BASE, BASE), None);
        assert_eq!(command_from_topic("home/room/mac/windows/", BASE), None);
    }

    #[test]
    fn focus_and_unread_are_understood() {
        let body = format!("{{\"id\":\"{SID}\"}}");
        assert_eq!(parse_request("claude-focus", &body), Some(Request::Focus(SID.to_string())));
        assert_eq!(parse_request("claude-session-unread", &body), Some(Request::Unread(SID.to_string())));
    }

    #[test]
    fn a_bare_string_body_is_understood_too() {
        // С панели openHASP на Windows-сторону прилетает сырая строка, а не
        // объект. Здесь такого источника пока нет, но разойтись с соседом в
        // разборе одного и того же топика — это отладка на двух машинах сразу.
        assert_eq!(parse_request("claude-focus", SID), Some(Request::Focus(SID.to_string())));
        assert_eq!(parse_request("claude-focus", &format!("\"{SID}\"")), Some(Request::Focus(SID.to_string())));
    }

    #[test]
    fn a_layout_request_carries_its_order() {
        // Соль списка: «в порядке сортировки списка» на этой стороне не
        // восстановить — порядок знает только тот, кто список показывает.
        let body = format!("{{\"mode\":\"tile\",\"ids\":[\"{SID}\",\"b\"]}}");
        assert_eq!(
            parse_request("claude-place", &body),
            Some(Request::Arrange {
                mode: Layout::Tile,
                ids: vec![SID.to_string(), "b".to_string()],
            })
        );
    }

    #[test]
    fn a_layout_request_without_a_list_is_still_a_request() {
        // С панели прилетает одно имя раскладки: порядка у неё нет вовсе.
        // Пустой список — не отказ, а «разложи всё, что ведёшь».
        let empty = Some(Request::Arrange { mode: Layout::Tile, ids: Vec::new() });
        assert_eq!(parse_request("claude-place", "{\"mode\":\"tile\"}"), empty);
        assert_eq!(parse_request("claude-place", "tile"), empty);
        assert_eq!(parse_request("claude-place", "\"tile\""), empty);
    }

    #[test]
    fn a_list_of_things_that_are_not_ids_is_dropped_quietly() {
        // Список приезжает из чужого json. Числа и пустые строки в нём — не
        // повод отказать в раскладке целиком: сессий столько же, сколько
        // разобралось, а окна человек просил разложить сейчас.
        let body = format!("{{\"mode\":\"tile\",\"ids\":[\"{SID}\",17,\"  \"]}}");
        assert_eq!(
            parse_request("claude-place", &body),
            Some(Request::Arrange { mode: Layout::Tile, ids: vec![SID.to_string()] })
        );
    }

    #[test]
    fn a_layout_nobody_knows_is_nothing() {
        // Имя раскладки приезжает строкой, и разойтись с пикером в наборе имён
        // легко. Молчаливый отказ лучше расстановки наугад: окна человека
        // уехали бы туда, куда он не просил.
        assert_eq!(parse_request("claude-place", "{\"mode\":\"mosaic\"}"), None);
        assert_eq!(parse_request("claude-place", "{}"), None);
        assert_eq!(parse_request("claude-place", ""), None);
    }

    #[test]
    fn an_unknown_command_is_nothing() {
        // Молча: на своей базе может оказаться что угодно, и жаловаться на
        // каждое сообщение значит забить журнал.
        let body = format!("{{\"id\":\"{SID}\"}}");
        assert_eq!(parse_request("claude-snapshot-restore", &body), None);
    }

    #[test]
    fn a_body_without_an_id_is_nothing() {
        assert_eq!(parse_request("claude-focus", "{}"), None);
        assert_eq!(parse_request("claude-focus", "{\"id\":\"\"}"), None);
        assert_eq!(parse_request("claude-focus", "{\"id\":17}"), None);
        assert_eq!(parse_request("claude-focus", ""), None);
    }
}
