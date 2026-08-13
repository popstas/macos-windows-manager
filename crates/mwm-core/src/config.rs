//! Настройки трекера.

/// Брокер, через который приезжают просьбы о подъёме окна и о пометке
/// непрочитанным.
///
/// Пароль живёт здесь, а не приезжает откуда-то ещё: у трея нет ни фронтенда,
/// ни аргументов командной строки, а argv виден в списке процессов.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    /// Префикс топиков этой машины. Подписка идёт на `<base>/#`.
    pub base: String,
}

impl MqttConfig {
    /// Настроен, если известны и адрес, и префикс: без второго подписываться
    /// некуда, а угадывать чужой префикс нельзя. То же правило, что у
    /// `Broker::is_configured` в пикере.
    pub fn is_configured(&self) -> bool {
        !self.host.is_empty() && !self.base.is_empty()
    }
}

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
    /// Брокер просьб. Пустой блок значит «просьб не будет», и тогда трекер не
    /// объявляет умения поднимать окно.
    pub mqtt: MqttConfig,
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

    // Блок читается по полям, как и всё остальное: опечатка в порту не должна
    // стоить адреса брокера. Отсутствующий или не-словарь блок даёт
    // выключенный брокер, а не отказ.
    let mqtt_map = map
        .get("mqtt")
        .and_then(|v| v.as_mapping())
        .cloned()
        .unwrap_or_default();
    let mqtt_text = |key: &str| {
        mqtt_map
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let mqtt = MqttConfig {
        host: mqtt_text("host"),
        port: mqtt_map
            .get("port")
            .and_then(|v| v.as_u64())
            .and_then(|v| u16::try_from(v).ok())
            .unwrap_or(1883),
        user: mqtt_text("user"),
        password: mqtt_map
            .get("password")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        base: mqtt_text("base").trim_end_matches('/').to_string(),
    };

    Config {
        ssh_host,
        remote_dir,
        host,
        terminals,
        tick_ms,
        dump_cache_ms,
        mqtt,
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
            "sshHost: remote-host\nremoteDir: /custom/path\ntickMs: 5000\ndumpCacheMs: 30000\n",
            "mac-host",
        );
        assert_eq!(c.remote_dir, "/custom/path", "remoteDir читается из remoteDir");
        assert_eq!(c.tick_ms, 5000, "tick_ms читается из tickMs");
        assert_eq!(c.dump_cache_ms, 30000, "dumpCacheMs читается из dumpCacheMs");
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

    #[test]
    fn mqtt_block_is_read() {
        let c = parse_config(
            "sshHost: remote-host\nmqtt:\n  host: broker.lan\n  port: 8883\n  user: picker\n  password: secret\n  base: home/room/mac/windows\n",
            "mac-host",
        );
        assert_eq!(c.mqtt.host, "broker.lan");
        assert_eq!(c.mqtt.port, 8883);
        assert_eq!(c.mqtt.user, "picker");
        assert_eq!(c.mqtt.password, "secret");
        assert_eq!(c.mqtt.base, "home/room/mac/windows");
        assert!(c.mqtt.is_configured());
    }

    #[test]
    fn missing_mqtt_block_is_a_switched_off_broker() {
        // Трекер без брокера работает: он просто не объявляет умения поднимать
        // окно, и Enter на маке остаётся тем, чем был.
        let c = parse_config("sshHost: remote-host\n", "mac-host");
        assert!(!c.mqtt.is_configured());
        assert_eq!(c.mqtt.port, 1883, "порт по умолчанию нужен и выключенному");
    }

    #[test]
    fn mqtt_without_base_is_not_configured() {
        // Угадывать чужой префикс топиков нельзя: публиковать было бы некуда,
        // а выглядело бы это настроенным брокером.
        let c = parse_config("mqtt:\n  host: broker.lan\n", "mac-host");
        assert!(!c.mqtt.is_configured());
    }

    #[test]
    fn trailing_slash_in_base_is_cut() {
        // Топик склеивается как `<base>/claude-focus`; лишняя косая дала бы
        // двойную, и подписка разошлась бы с публикацией на один символ.
        let c = parse_config("mqtt:\n  host: broker.lan\n  base: home/room/mac/windows/\n", "mac-host");
        assert_eq!(c.mqtt.base, "home/room/mac/windows");
    }

    #[test]
    fn junk_in_one_mqtt_field_does_not_cost_the_others() {
        // То же правило, что и у остальных полей конфига: опечатка стоит поля,
        // а не всех настроек.
        let c = parse_config(
            "mqtt:\n  host: broker.lan\n  port: \"не число\"\n  base: home/room/mac/windows\n",
            "mac-host",
        );
        assert_eq!(c.mqtt.host, "broker.lan");
        assert_eq!(c.mqtt.port, 1883);
        assert!(c.mqtt.is_configured());
    }

    #[test]
    fn ssh_host_survives_a_broken_mqtt_block() {
        let c = parse_config("sshHost: remote-host\nmqtt: \"not-a-map\"\n", "mac-host");
        assert_eq!(c.ssh_host, "remote-host");
        assert!(!c.mqtt.is_configured());
    }
}
