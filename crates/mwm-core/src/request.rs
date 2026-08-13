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
}

/// Имя команды — хвост топика после базы.
pub fn command_from_topic<'a>(topic: &'a str, base: &str) -> Option<&'a str> {
    let rest = topic.strip_prefix(base)?.strip_prefix('/')?;
    if rest.is_empty() || rest.contains('/') {
        return None;
    }
    Some(rest)
}

/// Просьба из имени команды и тела.
pub fn parse_request(command: &str, payload: &str) -> Option<Request> {
    let id = id_from_payload(payload)?;
    match command {
        "claude-focus" => Some(Request::Focus(id)),
        "claude-session-unread" => Some(Request::Unread(id)),
        _ => None,
    }
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
