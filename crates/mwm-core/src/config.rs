//! Настройки трекера.

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Машина, где живут сессии и агрегатор. Умолчания нет намеренно.
    pub ssh_host: String,
    /// Каталог файлов трекеров на той машине.
    pub remote_dir: String,
    /// Имя этой машины — то самое, которое человек пишет в `windowHost`
    /// конфига пикера. По нему пикер решает, поднимать ли окно.
    pub host: String,
    /// Bundle id приложений, чьи окна считаются терминалами.
    pub terminals: Vec<String>,
    pub tick_ms: u64,
    /// Срок годности индекса сессий. Ходить за дампом на каждом такте незачем:
    /// заголовок меняется на каждый ответ агента, а дамп и так отстаёт.
    pub dump_cache_ms: u64,
}

/// Разобрать конфиг, подставив умолчания всему, чего в нём нет.
///
/// Испорченный файл стоит настроек, а не запуска: трекер без настроек хотя бы
/// скажет об этом в трее, а не поднявшийся не скажет ничего.
///
/// Парсит каждое поле независимо: если одно поле неправильного типа, остальные
/// остаются нетронутыми, только это поле получает умолчание.
pub fn parse_config(text: &str, hostname: &str) -> Config {
    // Разобрать документ один раз в Value; если текст не парсится, использовать пустой mapping.
    let value: serde_yaml::Value = serde_yaml::from_str(text)
        .unwrap_or(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));

    let map = match value {
        serde_yaml::Value::Mapping(m) => m,
        _ => serde_yaml::Mapping::new(),
    };

    // Читать каждое поле независимо по ключам.
    let ssh_host = map
        .get("sshHost")
        .and_then(|v| v.as_str())
        .or_else(|| map.get("ssh_host").and_then(|v| v.as_str()))
        .unwrap_or_default()
        .to_string();

    let remote_dir = map
        .get("remoteDir")
        .and_then(|v| v.as_str())
        .unwrap_or("~/.ccfzf/windows")
        .to_string();

    let host = map
        .get("windowHost")
        .and_then(|v| v.as_str())
        .filter(|h| !h.trim().is_empty())
        .unwrap_or(hostname)
        .to_string();

    let terminals = map
        .get("terminals")
        .and_then(|v| v.as_sequence())
        .and_then(|seq| {
            let vec: Vec<String> = seq
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            if vec.is_empty() {
                None
            } else {
                Some(vec)
            }
        })
        .unwrap_or_else(|| {
            vec![
                "net.kovidgoyal.kitty".to_string(),
                "com.mitchellh.ghostty".to_string(),
                "com.googlecode.iterm2".to_string(),
            ]
        });

    let tick_ms = map
        .get("tickMs")
        .and_then(|v| v.as_u64())
        .unwrap_or(1_000);

    let dump_cache_ms = map
        .get("dumpCacheMs")
        .and_then(|v| v.as_u64())
        .unwrap_or(15_000);

    Config {
        ssh_host,
        remote_dir,
        host,
        terminals,
        tick_ms,
        dump_cache_ms,
    }
}

/// Где лежит конфиг. Тот же вид пути, что у пикера, — человеку их настраивать
/// рядом.
pub fn config_path(home: &str) -> String {
    format!("{home}/.config/macos-windows-manager/config.yaml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_cover_everything_but_the_ssh_host() {
        // Умолчания отвечают на вопрос «что делать, когда не сказали ничего».
        // У имени машины с сессиями ответа нет и быть не может — пустое поле
        // значит «настроить забыли», и трекер об этом скажет.
        let c = parse_config("", "mac-host");
        assert_eq!(c.ssh_host, "");
        assert_eq!(c.remote_dir, "~/.ccfzf/windows");
        assert_eq!(c.host, "mac-host", "имя своей машины берётся у системы");
        assert_eq!(c.tick_ms, 1_000);
        assert_eq!(c.dump_cache_ms, 15_000);
        assert_eq!(c.terminals, vec![
            "net.kovidgoyal.kitty".to_string(),
            "com.mitchellh.ghostty".to_string(),
            "com.googlecode.iterm2".to_string(),
        ]);
    }

    #[test]
    fn fields_are_read() {
        let c = parse_config(
            "sshHost: remote-host\nwindowHost: my-mac\nterminals:\n  - com.apple.Terminal\n",
            "mac-host",
        );
        assert_eq!(c.ssh_host, "remote-host");
        assert_eq!(c.host, "my-mac", "имя из конфига главнее системного");
        assert_eq!(c.terminals, vec!["com.apple.Terminal".to_string()]);
    }

    #[test]
    fn broken_config_is_defaults_not_a_crash() {
        // Испорченный конфиг стоит настроек, а не запуска: трекер без настроек
        // хотя бы скажет об этом в трее, а не поднявшийся не скажет ничего.
        let c = parse_config("sshHost: [unclosed\n\t\tnonsense", "mac-host");
        assert_eq!(c.host, "mac-host");
        assert_eq!(c.tick_ms, 1_000);
    }

    #[test]
    fn config_path_formats_correctly() {
        // Проверка пути к файлу конфига.
        let path = config_path("/home/user");
        assert_eq!(path, "/home/user/.config/macos-windows-manager/config.yaml");
    }

    #[test]
    fn numeric_and_remote_dir_fields_are_read() {
        // Проверка, что tickMs, dumpCacheMs и remoteDir читаются правильно.
        // Renaming #[serde(rename)] для этих полей должно сломать этот тест.
        let c = parse_config(
            "sshHost: remote-host\nremoteDir: /custom/path\nticMs: 5000\ndumpCacheMs: 30000\n",
            "mac-host",
        );
        assert_eq!(c.remote_dir, "/custom/path", "remoteDir (snake_case) читается из remoteDir");
        assert_eq!(c.dump_cache_ms, 30000, "dump_cache_ms (snake_case) читается из dumpCacheMs");
    }

    #[test]
    fn ssh_host_snake_case_alias_works() {
        // Проверка, что ssh_host (snake_case) тоже читается, если sshHost не задан.
        let c = parse_config(
            "ssh_host: snake-host\nwindowHost: my-mac\n",
            "mac-host",
        );
        assert_eq!(c.ssh_host, "snake-host", "ssh_host (snake_case) должен быть прочитан");
        assert_eq!(c.host, "my-mac");
    }

    #[test]
    fn ssh_host_camel_case_takes_precedence() {
        // Проверка, что sshHost (camelCase) главнее ssh_host.
        let c = parse_config(
            "sshHost: camel-host\nssh_host: snake-host\n",
            "mac-host",
        );
        assert_eq!(c.ssh_host, "camel-host", "sshHost должен быть главнее ssh_host");
    }

    #[test]
    fn empty_window_host_falls_back_to_system_hostname() {
        // Проверка, что пустой windowHost не используется и берётся имя системы.
        let c = parse_config(
            "sshHost: remote-host\nwindowHost: \"\"\n",
            "system-hostname",
        );
        assert_eq!(c.host, "system-hostname", "пустой windowHost должен быть проигнорирован");
    }

    #[test]
    fn empty_terminals_list_falls_back_to_defaults() {
        // Проверка, что пустой список терминалов используется по умолчанию.
        let c = parse_config(
            "sshHost: remote-host\nterminals: []\n",
            "mac-host",
        );
        assert_eq!(c.terminals, vec![
            "net.kovidgoyal.kitty".to_string(),
            "com.mitchellh.ghostty".to_string(),
            "com.googlecode.iterm2".to_string(),
        ], "пустой список терминалов должен быть заменён на умолчания");
    }

    #[test]
    fn ssh_host_survives_wrong_typed_field() {
        // Проверка, что валидный sshHost не теряется, если другое поле неправильного типа.
        // Это тест per-field parsing: terminals неправильного типа не должен повредить sshHost.
        let c = parse_config(
            "sshHost: remote-host\nterminals: \"not-a-list\"\n",
            "mac-host",
        );
        assert_eq!(c.ssh_host, "remote-host", "sshHost должен выжить при ошибке в другом поле");
        assert_eq!(c.terminals, vec![
            "net.kovidgoyal.kitty".to_string(),
            "com.mitchellh.ghostty".to_string(),
            "com.googlecode.iterm2".to_string(),
        ], "неправильно типизированное поле terminals должно получить умолчание");
    }
}
