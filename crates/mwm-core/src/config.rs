//! Настройки трекера.

use serde::Deserialize;

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

#[derive(Deserialize, Default)]
#[serde(default)]
struct Raw {
    ssh_host: Option<String>,
    #[serde(rename = "sshHost")]
    ssh_host_camel: Option<String>,
    #[serde(rename = "remoteDir")]
    remote_dir: Option<String>,
    #[serde(rename = "windowHost")]
    window_host: Option<String>,
    terminals: Option<Vec<String>>,
    #[serde(rename = "tickMs")]
    tick_ms: Option<u64>,
    #[serde(rename = "dumpCacheMs")]
    dump_cache_ms: Option<u64>,
}

/// Разобрать конфиг, подставив умолчания всему, чего в нём нет.
///
/// Испорченный файл стоит настроек, а не запуска: трекер без настроек хотя бы
/// скажет об этом в трее, а не поднявшийся не скажет ничего.
pub fn parse_config(text: &str, hostname: &str) -> Config {
    let raw: Raw = serde_yaml::from_str(text).unwrap_or_default();
    Config {
        ssh_host: raw.ssh_host_camel.or(raw.ssh_host).unwrap_or_default(),
        remote_dir: raw.remote_dir.unwrap_or_else(|| "~/.ccfzf/windows".to_string()),
        host: raw
            .window_host
            .filter(|h| !h.trim().is_empty())
            .unwrap_or_else(|| hostname.to_string()),
        terminals: raw.terminals.filter(|t| !t.is_empty()).unwrap_or_else(|| {
            vec![
                "net.kovidgoyal.kitty".to_string(),
                "com.mitchellh.ghostty".to_string(),
                "com.googlecode.iterm2".to_string(),
            ]
        }),
        tick_ms: raw.tick_ms.unwrap_or(1_000),
        dump_cache_ms: raw.dump_cache_ms.unwrap_or(15_000),
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
}
