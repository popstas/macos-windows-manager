//! Доставка файла окон на машину агрегатора.

use mwm_core::config::Config;
use std::io::Write;
use std::process::{Command, Stdio};

/// Положить файл рядом с чужими и подменить атомарно.
///
/// Временный файл и `mv` в том же каталоге — потому что читатель опрашивает
/// раз в секунду, и без атомарной подмены он рано или поздно прочитает
/// половину json. Половина json — это не «неполные данные», это исключение
/// вместо списка сессий у человека на экране.
///
/// Имя файла — имя машины: по нему и человек, и агрегатор понимают, чей это
/// файл, а два трекера с одним именем — это одна машина, дважды запущенная,
/// и второй экземпляр обязан перетереть первый, а не завести соседний файл.
///
/// `;` в удалённой команде не ставится: у Windows Terminal это разделитель
/// панелей. Здесь та сторона не Windows, но правило связки одно на все
/// удалённые команды, и исключение из него пришлось бы помнить.
pub fn send(cfg: &Config, payload: &serde_json::Value) -> Result<(), String> {
    if cfg.ssh_host.trim().is_empty() {
        return Err("sshHost is not set".to_string());
    }
    let dir = &cfg.remote_dir;
    let name = safe_name(&cfg.host);
    let remote = format!(
        "mkdir -p {dir} && cat > {dir}/{name}.json.tmp && mv {dir}/{name}.json.tmp {dir}/{name}.json"
    );
    let mut child = Command::new("ssh")
        .args([
            "-o", "BatchMode=yes",
            "-o", "ConnectTimeout=8",
            "-o", "ControlMaster=auto",
            "-o", "ControlPath=~/.ssh/mwm-%r@%h-%p",
            "-o", "ControlPersist=300",
            &cfg.ssh_host, &remote,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(payload.to_string().as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Имя машины в имени файла: буквы, цифры, точка, дефис — и ничего больше.
///
/// Строка едет в команду чужого шелла, и кавычить её было бы половиной защиты:
/// hostname с пробелом или кавычкой законен, а команда от него разваливается.
fn safe_name(host: &str) -> String {
    let s: String = host
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '-' })
        .collect();
    if s.is_empty() { "unnamed".to_string() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostname_becomes_a_safe_file_name() {
        // Строка едет в команду чужого шелла. Пробел или кавычка в имени
        // машины законны, а команда от них разваливается.
        assert_eq!(safe_name("my-mac.local"), "my-mac.local");
        assert_eq!(safe_name("my mac; rm -rf /"), "my-mac--rm--rf--");
        assert_eq!(safe_name("   "), "unnamed");
    }
}
