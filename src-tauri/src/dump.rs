//! Дамп сессий с машины агрегатора.
//!
//! Ходим за ним по ssh и редко: заголовок меняется на каждый ответ агента, а
//! дамп и так отстаёт до тридцати секунд. Спрашивают его тогда, когда трекеру
//! попался незнакомый заголовок, и не чаще срока годности.

use mwm_core::config::Config;
use mwm_core::index::{parse_index, SessionRef};
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

/// Индекс со сроком годности.
#[derive(Default)]
pub struct Cache {
    index: BTreeMap<String, SessionRef>,
    fetched_ms: u64,
    pub last_error: Option<String>,
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
                        self.index = idx;
                    }
                    self.last_error = None;
                }
                Err(e) => self.last_error = Some(e),
            }
            self.fetched_ms = now_ms;
        }
        &self.index
    }
}
