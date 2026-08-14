//! Запись config.yaml из окна настроек.
//!
//! Порт одноимённого модуля из соседнего ccfzf-picker: то же приложение на
//! Tauri 2 с тем же конфигом, и решения здесь не выведены заново, а взяты
//! вместе с причинами, за которые там уже заплачено выкатками.

/// Шапка переписанного конфига.
///
/// Записывается затем, что перезапись через serde_yaml теряет комментарии — а
/// в config.yaml они и есть документация. Человек, открывший файл после
/// первого сохранения, должен сразу понимать, куда делись его пометки и где
/// лежит прежний файл. Сама шапка — комментарий, разбор её выбрасывает, и на
/// следующем сохранении она не удваивается.
pub const HEADER: &str = "\
# This file is managed by the macos-windows-manager settings window: saving
# rewrites it whole, and comments in it are not preserved. The previous file is
# next to it, as config.yaml.bak. All keys are documented in the repository's
# config.example.yml.
";

/// Влить патч в документ.
///
/// Отображения сливаются по ключам, всё остальное заменяется целиком. Разница
/// не косметическая: `mqtt.password` окно настроек не показывает и обратно не
/// присылает, и замена блока целиком стирала бы пароль на каждом сохранении.
/// Списки, наоборот, заменяются: слить два списка по ключам нечем, а
/// «дописать» — не то, чего хочет человек, убравший строку из формы.
pub fn merge_patch(doc: &mut serde_yaml::Value, patch: &serde_json::Value) -> Result<(), String> {
    let Some(fields) = patch.as_object() else {
        return Err("settings did not arrive as an object".into());
    };
    if doc.is_null() {
        *doc = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
    }
    let Some(map) = doc.as_mapping_mut() else {
        return Err("config.yaml is not a mapping, there is nothing to edit in it".into());
    };
    for (key, value) in fields {
        let key = serde_yaml::Value::String(key.clone());
        // Решение принимается до `get_mut`: заимствование от него живёт до
        // конца ветки, и `insert` в соседней уже не собрался бы.
        let nested = value.as_object().is_some()
            && map.get(&key).map(|v| v.is_mapping()).unwrap_or(false);
        if nested {
            // `unwrap` безопасен: `nested` истинно только когда ключ есть.
            merge_patch(map.get_mut(&key).unwrap(), value)?;
        } else {
            let incoming: serde_yaml::Value =
                serde_yaml::to_value(value).map_err(|e| format!("cannot convert value: {e}"))?;
            map.insert(key, incoming);
        }
    }
    Ok(())
}

/// Документ в текст, без шапки: её ставит вызывающий.
pub fn render(doc: &serde_yaml::Value) -> Result<String, String> {
    serde_yaml::to_string(doc).map_err(|e| format!("cannot render yaml: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(yaml: &str) -> serde_yaml::Value {
        serde_yaml::from_str(yaml).unwrap()
    }

    /// Нетронутые ключи переживают запись.
    ///
    /// Ради этого патч и слияние: окно настроек не знает про блок `state:`
    /// вовсе, а перезапись целиком стёрла бы человеку настроенные пути молча.
    #[test]
    fn untouched_keys_survive() {
        let mut d = doc("sshHost: old\nstate:\n  path: /tmp/s.json\n  keep: 3\n");
        merge_patch(&mut d, &serde_json::json!({"sshHost": "new"})).unwrap();
        let out = render(&d).unwrap();
        assert!(out.contains("new"), "новое значение записано: {out}");
        assert!(!out.contains("old"), "старое значение заменено: {out}");
        assert!(out.contains("/tmp/s.json"), "чужой ключ на месте: {out}");
    }

    /// Вложенное отображение сливается по ключам, а не заменяется целиком.
    ///
    /// Из-за пароля: окно настроек не показывает его и не присылает обратно —
    /// замена блока целиком стирала бы пароль на каждом сохранении.
    #[test]
    fn nested_maps_merge_key_by_key() {
        let mut d = doc("mqtt:\n  host: broker\n  base: home/room/mac/windows\n  password: secret\n");
        merge_patch(&mut d, &serde_json::json!({"mqtt": {"host": "other"}})).unwrap();
        let out = render(&d).unwrap();
        assert!(out.contains("other"));
        assert!(out.contains("secret"), "пароль не тронут: {out}");
        assert!(out.contains("home/room/mac/windows"), "префикс топиков не тронут: {out}");
    }

    /// Списки заменяются целиком, а не сливаются поэлементно.
    ///
    /// Образец — `terminals`, единственный список в форме (поле типа `lines`).
    /// Человек правит там строки целиком, и «дописать» вместо «заменить»
    /// означало бы, что убранный bundle id возвращается сам.
    #[test]
    fn lists_are_replaced_whole() {
        let mut d = doc("tickMs: 1000\nterminals:\n  - net.kovidgoyal.kitty\n  - com.googlecode.iterm2\n");
        merge_patch(&mut d, &serde_json::json!({
            "terminals": ["com.mitchellh.ghostty"]
        })).unwrap();
        let out = render(&d).unwrap();
        assert!(out.contains("ghostty"));
        assert!(!out.contains("iterm2"), "убранный терминал не воскресает: {out}");
        assert!(out.contains("tickMs"), "соседний ключ на месте: {out}");
    }

    /// Пустой документ — не отказ: конфига могло не быть вовсе.
    #[test]
    fn empty_document_becomes_a_mapping() {
        let mut d = serde_yaml::Value::Null;
        merge_patch(&mut d, &serde_json::json!({"sshHost": "host"})).unwrap();
        assert!(render(&d).unwrap().contains("host"));
    }

    /// Патч не той формы — отказ, а не молчаливая порча файла.
    #[test]
    fn non_object_patch_is_refused() {
        let mut d = doc("sshHost: old\n");
        assert!(merge_patch(&mut d, &serde_json::json!("строка")).is_err());
        assert!(merge_patch(&mut d, &serde_json::json!([1, 2])).is_err());
    }

    /// Шапка — комментарий, и обратно она не читается: значит, на следующем
    /// сохранении не удвоится.
    #[test]
    fn header_is_a_comment_and_does_not_accumulate() {
        let once = format!("{HEADER}sshHost: host\n");
        let parsed: serde_yaml::Value = serde_yaml::from_str(&once).unwrap();
        let twice = format!("{HEADER}{}", render(&parsed).unwrap());
        assert_eq!(
            twice.matches("settings window").count(),
            HEADER.matches("settings window").count()
        );
    }
}
