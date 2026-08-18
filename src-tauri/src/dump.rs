//! Дамп сессий с машины агрегатора.
//!
//! Ходим за ним по ssh и редко: заголовок меняется на каждый ответ агента, а
//! дамп и так отстаёт до тридцати секунд. Спрашивают его тогда, когда трекеру
//! попался незнакомый заголовок, и не чаще срока годности.

use mwm_core::config::Config;
use mwm_core::index::{merge_index, parse_index, SessionRef};
use std::collections::BTreeMap;
use std::process::Command;

/// Одно ssh-соединение на всё: мультиплексирование делает второй и третий
/// вызовы почти бесплатными, а без него каждый стоил бы рукопожатия.
fn ssh(host: &str, remote: &str) -> Result<String, String> {
    let out = Command::new("ssh")
        .args([
            "-o", "BatchMode=yes",
            "-o", "ConnectTimeout=8",
            "-o", "ControlMaster=auto",
            "-o", "ControlPath=~/.ssh/mwm-%r@%h-%p",
            "-o", "ControlPersist=300",
            host, remote,
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn fetch(ssh_host: &str) -> Result<String, String> {
    if ssh_host.trim().is_empty() {
        return Err("sshHost is not set".to_string());
    }
    ssh(ssh_host, "cat ~/.ccfzf.sessions.json")
}

/// Дамп агрегатора этой же машины.
///
/// Читается файл, а не запускается `ccfzf`: на маке бинарь агрегатора живёт
/// только внутри каталога пикера, и звать его оттуда значило бы привязать
/// трекер к чужой установке. Дамп пикер и так переписывает на каждый свой
/// опрос.
///
/// Цена — свежесть, и она названа: у скрытого пикера такт уходит до восьми
/// минут, столько же ждёт привязки только что открытая местная сессия.
/// Открытый пикер возвращает такт к секунде, и окно появляется.
///
/// `Ok(None)` — файла нет: на этой машине агрегатора не держат, и сказать тут
/// нечего. Жаловаться на это в строку состояния значило бы жаловаться каждый
/// такт на честно выключенную половину. Ошибка чтения — другое дело: файл
/// есть, а прочесть его нечем, и это видно.
fn read_local(path: &str) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{path}: {e}")),
    }
}

/// Индекс со сроком годности.
///
/// Половинки хранятся врозь, а слитый индекс пересчитывается: правило «пустой
/// разбор прежний индекс не затирает» теперь своё у каждой дороги, и отказ
/// одной не имеет права стереть то, что привезла другая.
#[derive(Default)]
pub struct Cache {
    remote: BTreeMap<String, SessionRef>,
    local: BTreeMap<String, SessionRef>,
    index: BTreeMap<String, SessionRef>,
    fetched_ms: u64,
    pub last_error: Option<String>,
    /// Отказ чтения местного дампа. Отдельное поле, а не общее с удалённым:
    /// починки у них разные — там чинят ssh и агрегатор той машины, здесь
    /// права на файл у себя, — и одна ошибка, прячущая другую, отправила бы
    /// человека не туда.
    pub local_error: Option<String>,
}

impl Cache {
    /// Отдать индекс, освежив его, если пора и если есть зачем.
    ///
    /// `wanted` — трекеру попался заголовок, которого в индексе нет. Без этого
    /// признака ходили бы каждые пятнадцать секунд впустую: у машины, где
    /// ничего не открывали, индекс не меняется часами.
    pub fn get(&mut self, cfg: &Config, now_ms: u64, wanted: bool) -> &BTreeMap<String, SessionRef> {
        let stale = now_ms.saturating_sub(self.fetched_ms) >= cfg.dump_cache_ms;
        if (wanted && stale) || self.fetched_ms == 0 {
            match fetch(&cfg.ssh_host) {
                Ok(text) => {
                    let idx = parse_index(&text);
                    // Пустой разбор прежний индекс не затирает: дамп мог не
                    // дочитаться, а привязка, живущая на прежних слотах, —
                    // единственное, что держит окна в списке до следующей
                    // удачи.
                    if !idx.is_empty() {
                        self.remote = idx;
                    }
                    self.last_error = None;
                }
                Err(e) => self.last_error = Some(e),
            }
            if cfg.features.local_source {
                match read_local(&mwm_core::config::local_dump_path(&home())) {
                    Ok(Some(text)) => {
                        let idx = parse_index(&text);
                        if !idx.is_empty() {
                            self.local = idx;
                        }
                        self.local_error = None;
                    }
                    // Файла нет — половина честно выключена, и прежнее
                    // содержимое не трогаем: пикер могли просто закрыть.
                    Ok(None) => self.local_error = None,
                    Err(e) => self.local_error = Some(e),
                }
            } else {
                self.local = BTreeMap::new();
                self.local_error = None;
            }
            self.index = merge_index(self.remote.clone(), self.local.clone());
            self.fetched_ms = now_ms;
        }
        &self.index
    }
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::read_local;

    #[test]
    fn a_missing_local_dump_is_not_a_failure() {
        // На машине без местного агрегатора файла нет, и это норма, а не
        // поломка: жалоба на него шла бы в строку состояния каждый такт.
        assert_eq!(read_local("/nonexistent/ccfzf.sessions.json"), Ok(None));
    }

    #[test]
    fn a_local_dump_is_read_whole() {
        let dir = std::env::temp_dir().join("mwm-dump-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sessions.json");
        std::fs::write(&path, "{\"sessions\":[]}").unwrap();
        assert_eq!(
            read_local(path.to_str().unwrap()),
            Ok(Some("{\"sessions\":[]}".to_string()))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unreadable_local_dump_names_itself() {
        // Каталог вместо файла — ошибка не «нет файла», и она обязана доехать
        // до человека вместе с путём: чинить её будут здесь, а не на машине
        // агрегатора.
        let dir = std::env::temp_dir().join("mwm-dump-not-a-file");
        std::fs::create_dir_all(&dir).unwrap();
        let err = read_local(dir.to_str().unwrap()).unwrap_err();
        assert!(err.contains("mwm-dump-not-a-file"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
