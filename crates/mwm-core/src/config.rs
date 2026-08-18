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

/// Что трекеру разрешено делать.
///
/// Всё включено по умолчанию, и это не вкусовщина: блока `features:` нет ни в
/// одном конфиге, который уже лежит на маках, и появление флагов не имеет права
/// ничего у них выключить.
#[derive(Debug, Clone, PartialEq)]
pub struct Features {
    /// Ставить ли появившееся окно на запомненное место.
    pub placement: bool,
    /// Вести ли снимки раскладки.
    pub snapshots: bool,
    /// Исполнять ли просьбы, приехавшие по MQTT.
    pub requests: bool,
}

impl Default for Features {
    fn default() -> Self {
        Self { placement: true, snapshots: true, requests: true }
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
    /// Где лежат слоты сессий. Не рядом с конфигом намеренно: конфиг человек
    /// правит руками, состояние пишет машина, и соседство приглашает спутать
    /// резервную копию одного с рабочим файлом другого.
    pub state_path: String,
    pub snapshots_path: String,
    pub snapshots_keep: usize,
    pub snapshots_debounce_ms: u64,
    /// Выключатели фич. Пустой блок значит «всё включено».
    pub features: Features,
    /// Глобальный хоткей раскладки плиткой — строкой, как её пишет человек.
    /// Пустое поле значит «взять умолчание», а не «без хоткея»: выключателем
    /// служит блок `features`, и второго способа выключить одно и то же
    /// заводить незачем.
    pub tile_hotkey: String,
}

/// Терминалы, чьи окна считаются терминалами, пока человек не сказал иначе.
///
/// Список один и живёт здесь: разойдись он с копией, часть проверок доказывала
/// бы умолчание, которого нет. Не названного здесь терминала трекер не видит
/// вовсе — `list_windows` перебирает только приложения из этого списка, — и
/// отказ этот молчащий: окна нет ни в файле, ни в логе. Так пропал WezTerm,
/// на который пикер переехал раньше трекера.
pub fn default_terminals() -> Vec<String> {
    vec![
        "net.kovidgoyal.kitty".to_string(),
        "com.mitchellh.ghostty".to_string(),
        "com.googlecode.iterm2".to_string(),
        "com.github.wez.wezterm".to_string(),
    ]
}

/// Хоткей плитки, пока человек не сказал иначе.
///
/// Строка, а не разобранная комбинация: `mwm-core` про клавиатуру ничего не
/// знает и знать не должен — разбирает её `main.rs`, где живёт плагин. Здесь
/// умолчание лежит затем, чтобы его называли одним и тем же и конфиг, и
/// подпись пункта меню.
pub const DEFAULT_TILE_HOTKEY: &str = "Cmd+Alt+Ctrl+C";

/// Разобрать конфиг, подставив умолчания всему, чего в нём нет.
///
/// Испорченный файл стоит настроек, а не запуска: трекер без настроек хотя бы
/// скажет об этом в трее, а не поднявшийся не скажет ничего.
///
/// Парсит каждое поле независимо: если одно поле неправильного типа, остальные
/// остаются нетронутыми, только это поле получает умолчание.
pub fn parse_config(text: &str, hostname: &str) -> Config {
    // `HOME` на маке выставлен всегда — в отличие от Windows, где `load_config`
    // в пикере знает про запасной `USERPROFILE`. Пустая строка дала бы
    // относительный путь, и это заметили бы на первом же запуске: файл лёг бы
    // рядом с бинарём.
    let home = std::env::var("HOME").unwrap_or_default();

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
        .unwrap_or_else(default_terminals);

    let tick_ms = map
        .get("tickMs")
        .and_then(|v| v.as_u64())
        .unwrap_or(1_000);

    let dump_cache_ms = map
        .get("dumpCacheMs")
        .and_then(|v| v.as_u64())
        .unwrap_or(15_000);

    // Проверять комбинацию здесь нечем: понимает её плагин хоткеев, а он
    // живёт в приложении. Непонятая строка откатывается на умолчание там же,
    // где разбирается, — и там же о ней говорит в лог.
    let tile_hotkey = map
        .get("tileHotkey")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_TILE_HOTKEY)
        .to_string();

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

    let state_map = map
        .get("state")
        .and_then(|v| v.as_mapping())
        .cloned()
        .unwrap_or_default();
    let state_text = |key: &str| {
        state_map.get(key).and_then(|v| v.as_str()).unwrap_or_default().trim().to_string()
    };
    let state_dir = format!("{home}/.local/state/macos-windows-manager");
    let state_path = {
        let p = state_text("path");
        if p.is_empty() { format!("{state_dir}/state.json") } else { p }
    };
    let snapshots_path = {
        let p = state_text("snapshotsPath");
        if p.is_empty() { format!("{state_dir}/snapshots.json") } else { p }
    };
    let snapshots_keep = state_map
        .get("keep")
        .and_then(|v| v.as_u64())
        .and_then(|v| usize::try_from(v).ok())
        .filter(|v| *v > 0)
        .unwrap_or(crate::snapshots::KEEP);
    let snapshots_debounce_ms = state_map
        .get("debounceMs")
        .and_then(|v| v.as_u64())
        .unwrap_or(crate::snapshots::DEBOUNCE_MS);

    // Флаги читаются по одному, а не структурой целиком: нечитаемое значение
    // одного не должно включать обратно два соседних, которые человек выключил
    // осознанно.
    let features_map = map
        .get("features")
        .and_then(|v| v.as_mapping())
        .cloned()
        .unwrap_or_default();
    let flag = |key: &str| features_map.get(key).and_then(|v| v.as_bool()).unwrap_or(true);
    let features = Features {
        placement: flag("placement"),
        snapshots: flag("snapshots"),
        requests: flag("requests"),
    };

    Config {
        ssh_host,
        remote_dir,
        host,
        terminals,
        tick_ms,
        dump_cache_ms,
        mqtt,
        state_path,
        snapshots_path,
        snapshots_keep,
        snapshots_debounce_ms,
        features,
        tile_hotkey,
    }
}

/// Конфиг как его увидит окно настроек.
///
/// Ключи — те же, что читает `parse_config`, и круговой тест это сторожит:
/// окно показывает человеку действующие значения, и разъехавшееся имя ключа
/// означало бы, что показано не то, что подхвачено.
///
/// Пароля здесь нет намеренно. Он едет только в одну сторону — от человека в
/// файл; форма показывает пустое поле и шлёт его, лишь когда в него что-то
/// ввели.
pub fn to_json(cfg: &Config) -> serde_json::Value {
    serde_json::json!({
        "sshHost": cfg.ssh_host,
        "remoteDir": cfg.remote_dir,
        "windowHost": cfg.host,
        "terminals": cfg.terminals,
        "tickMs": cfg.tick_ms,
        "dumpCacheMs": cfg.dump_cache_ms,
        "tileHotkey": cfg.tile_hotkey,
        "mqtt": {
            "host": cfg.mqtt.host,
            "port": cfg.mqtt.port,
            "user": cfg.mqtt.user,
            "base": cfg.mqtt.base,
        },
        "state": {
            "path": cfg.state_path,
            "snapshotsPath": cfg.snapshots_path,
            "keep": cfg.snapshots_keep,
            "debounceMs": cfg.snapshots_debounce_ms,
        },
        "features": {
            "placement": cfg.features.placement,
            "snapshots": cfg.features.snapshots,
            "requests": cfg.features.requests,
        },
    })
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
        // Список выписан здесь дословно, а не сверен с `default_terminals()`:
        // сверка с самой собой не доказала бы ничего, а терминал, выпавший из
        // умолчания, не виден трекеру вовсе и молча.
        assert_eq!(c.terminals, vec![
            "net.kovidgoyal.kitty".to_string(),
            "com.mitchellh.ghostty".to_string(),
            "com.googlecode.iterm2".to_string(),
            "com.github.wez.wezterm".to_string(),
        ]);
    }

    #[test]
    fn tile_hotkey_falls_back_to_the_default_when_nothing_is_said() {
        // Три способа не сказать ничего, и все три значат одно. Пустая строка
        // — не «без хоткея»: выключателем служит блок `features`, а пустое
        // поле в окне настроек человек оставляет, не выбирая ничего.
        for text in ["", "tileHotkey: \"\"\n", "tileHotkey: \"   \"\n"] {
            let c = parse_config(text, "mac-host");
            assert_eq!(c.tile_hotkey, DEFAULT_TILE_HOTKEY, "конфиг {text:?}");
        }
    }

    #[test]
    fn tile_hotkey_from_the_config_wins() {
        // Комбинация едет наружу как написана: понимает её плагин хоткеев, а
        // приводить строку к какому-нибудь общему виду значило бы показывать
        // человеку в меню не то, что он написал в файле.
        let c = parse_config("tileHotkey: Cmd+Shift+T\n", "mac-host");
        assert_eq!(c.tile_hotkey, "Cmd+Shift+T");
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
        assert_eq!(c.terminals, default_terminals(),
            "пустой список терминалов должен быть заменён на умолчания");
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
        assert_eq!(c.terminals, default_terminals(),
            "неправильно типизированное поле terminals должно получить умолчание");
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

    #[test]
    fn state_paths_have_defaults() {
        // Трекер обязан работать с конфигом, в котором про состояние не сказано
        // ни слова: этап 3 добавил файлы, а конфиги у людей остались прежние.
        let c = parse_config("sshHost: remote-host\n", "mac-host");
        assert!(c.state_path.ends_with("macos-windows-manager/state.json"), "{}", c.state_path);
        assert!(c.snapshots_path.ends_with("macos-windows-manager/snapshots.json"), "{}", c.snapshots_path);
        assert_eq!(c.snapshots_keep, 20);
        assert_eq!(c.snapshots_debounce_ms, 60_000);
    }

    #[test]
    fn state_paths_can_be_moved() {
        let c = parse_config(
            "state:\n  path: /tmp/s.json\n  snapshotsPath: /tmp/snap.json\n  keep: 3\n  debounceMs: 5000\n",
            "mac-host",
        );
        assert_eq!(c.state_path, "/tmp/s.json");
        assert_eq!(c.snapshots_path, "/tmp/snap.json");
        assert_eq!(c.snapshots_keep, 3);
        assert_eq!(c.snapshots_debounce_ms, 5_000);
    }

    #[test]
    fn junk_in_one_state_field_does_not_cost_the_others() {
        // То же правило, что у остальных полей конфига: опечатка стоит поля, а
        // не всех настроек.
        let c = parse_config("state:\n  path: /tmp/s.json\n  keep: \"не число\"\n", "mac-host");
        assert_eq!(c.state_path, "/tmp/s.json");
        assert_eq!(c.snapshots_keep, 20);
    }

    #[test]
    fn features_are_all_on_when_the_block_is_missing() {
        // Конфиги, которые уже лежат на маках, обязаны вести себя ровно как
        // раньше: блока `features:` в них нет и не будет, пока человек его не
        // напишет.
        let c = parse_config("sshHost: remote-host\n", "mac-host");
        assert_eq!(c.features, Features { placement: true, snapshots: true, requests: true });
    }

    #[test]
    fn features_are_read() {
        let c = parse_config(
            "features:\n  placement: false\n  snapshots: false\n  requests: false\n",
            "mac-host",
        );
        assert_eq!(c.features, Features { placement: false, snapshots: false, requests: false });
    }

    #[test]
    fn junk_in_one_feature_does_not_cost_the_others() {
        // То же правило, что у остальных полей конфига: опечатка стоит поля, а
        // не всех настроек. Выключить человек хотел одно, и выключиться должно
        // ровно одно.
        let c = parse_config("features:\n  placement: \"нет\"\n  snapshots: false\n", "mac-host");
        assert!(c.features.placement, "нечитаемый флаг остаётся включённым");
        assert!(!c.features.snapshots);
        assert!(c.features.requests);
    }

    #[test]
    fn features_not_a_mapping_leaves_everything_on() {
        let c = parse_config("features: \"да\"\n", "mac-host");
        assert_eq!(c.features, Features::default());
    }

    #[test]
    fn to_json_uses_the_keys_parse_config_reads() {
        // Круговой тест: окно настроек показывает `to_json`, а трекер читает
        // `parse_config`. Разойдись они в имени хоть одного ключа — окно
        // показывало бы не то, что подхвачено, и заметить это можно было бы
        // только глазами на маке.
        let src = parse_config(
            "sshHost: remote-host\nremoteDir: /custom\nwindowHost: my-mac\ntickMs: 5000\ndumpCacheMs: 30000\nterminals:\n  - com.apple.Terminal\nmqtt:\n  host: broker.lan\n  port: 8883\n  user: picker\n  base: home/room/mac/windows\nfeatures:\n  snapshots: false\n",
            "mac-host",
        );
        let text = serde_yaml::to_string(&to_json(&src)).unwrap();
        assert_eq!(parse_config(&text, "mac-host"), src);
    }

    #[test]
    fn to_json_never_carries_the_password() {
        // Пароль уезжает в окно настроек только в одну сторону — от человека к
        // файлу. Показать его форме значило бы разложить его по webview и по
        // истории IPC ради поля, которое и так не показывается.
        let c = parse_config("mqtt:\n  host: broker.lan\n  password: secret\n", "mac-host");
        let text = serde_json::to_string(&to_json(&c)).unwrap();
        assert!(!text.contains("secret"), "{text}");
    }
}
