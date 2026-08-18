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
    let dir = safe_dir(&cfg.remote_dir)?;
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

/// Положить тот же файл рядом с собой — туда, где его прочтёт агрегатор этой
/// же машины.
///
/// Второй адрес, а не замена первому: файл нужен обеим сторонам. На машине
/// агрегатора он рассказывает про окна ssh-сессий, здесь — про окна сессий,
/// которые работают тут же и в удалённом дампе не значатся вовсе.
///
/// Подмена атомарная и по той же причине, что у ssh-дороги: читатель
/// опрашивает раз в секунду, и половина json у него на экране — это
/// исключение вместо списка сессий. Шелла здесь нет, поэтому и просеивать
/// нечего: `create_dir_all` и `rename` получают путь аргументом, а не строкой
/// команды. Имя файла всё равно идёт через `safe_name` — оно то же самое, что
/// у файла на той стороне, и разъехаться этим двум именам нельзя: агрегатор
/// сливает файлы по имени машины внутри, а человек ищет их глазами по имени
/// снаружи.
pub fn write_local(dir: &str, host: &str, payload: &serde_json::Value) -> Result<(), String> {
    let dir = std::path::Path::new(dir);
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = safe_name(host);
    let tmp = dir.join(format!("{name}.json.tmp"));
    let final_path = dir.join(format!("{name}.json"));
    let text = serde_json::to_string(payload).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &final_path).map_err(|e| format!("{}: {e}", final_path.display()))
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

/// Путь к каталогу трекеров: буквы, цифры, точка, дефис, подчёркивание, слэш и тильда.
///
/// Строка едет в команду чужого шелла, и кавычить её было бы половиной защиты.
/// Кавычки не спасают от `$(...)`, который исполняется внутри двойных кавычек.
/// Это не просто расширение переменной, а выполнение кода на удалённой машине
/// под идентичностью пользователя ssh. Запретить это полностью может только
/// allowlist: убрать все символы, кроме необходимых для пути.
fn safe_dir(path: &str) -> Result<String, String> {
    let s: String = path
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' || c == '/' || c == '~'
            {
                c
            } else {
                '-'
            }
        })
        .collect();
    if s.is_empty() {
        Err("remoteDir is not set".to_string())
    } else {
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_local_file_lands_under_the_machine_name() {
        // Имя обязано совпасть с тем, под которым файл лежит на той стороне:
        // агрегатор сливает файлы по имени машины внутри, а человек ищет их
        // глазами по имени снаружи.
        let dir = std::env::temp_dir().join("mwm-local-publish");
        std::fs::remove_dir_all(&dir).ok();
        let payload = serde_json::json!({"host": "mac", "windows": {}});
        write_local(dir.to_str().unwrap(), "mac", &payload).unwrap();
        let text = std::fs::read_to_string(dir.join("mac.json")).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&text).unwrap(), payload);
        assert!(!dir.join("mac.json.tmp").exists(), "временный файл не остаётся");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_second_write_replaces_the_first() {
        // Два трекера с одним именем — это одна машина, запущенная дважды, и
        // второй обязан перетереть первого, а не завести соседний файл. То же
        // правило, что у ssh-дороги.
        let dir = std::env::temp_dir().join("mwm-local-publish-twice");
        std::fs::remove_dir_all(&dir).ok();
        write_local(dir.to_str().unwrap(), "mac", &serde_json::json!({"n": 1})).unwrap();
        write_local(dir.to_str().unwrap(), "mac", &serde_json::json!({"n": 2})).unwrap();
        let text = std::fs::read_to_string(dir.join("mac.json")).unwrap();
        assert!(text.contains("\"n\":2"), "{text}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "соседних файлов нет");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unwritable_directory_names_itself() {
        // Отказ обязан назвать путь: чинить его будут здесь, у себя, а не на
        // машине агрегатора — тем и отличается от отказа ssh-дороги.
        let blocked = std::env::temp_dir().join("mwm-local-blocked");
        std::fs::remove_dir_all(&blocked).ok();
        std::fs::write(&blocked, "не каталог").unwrap();
        let err = write_local(blocked.join("under").to_str().unwrap(), "mac",
                              &serde_json::json!({}))
            .unwrap_err();
        assert!(err.contains("mwm-local-blocked"), "{err}");
        std::fs::remove_file(&blocked).ok();
    }

    #[test]
    fn hostname_becomes_a_safe_file_name() {
        // Строка едет в команду чужого шелла. Пробел или кавычка в имени
        // машины законны, а команда от них разваливается.
        assert_eq!(safe_name("my-mac.local"), "my-mac.local");
        assert_eq!(safe_name("my mac; rm -rf /"), "my-mac--rm--rf--");
        assert_eq!(safe_name("   "), "unnamed");
    }

    #[test]
    fn directory_path_with_space_becomes_dash() {
        // Пробел в пути — узаконенный случай в имёнах файлов, но не в имёнах
        // аргументов интерпретатора. Путь вставляется в формат-строку и едет в
        // команду чужого шелла. Пробел там развалил бы строку на два аргумента.
        // Allowlist заменяет пробел на дефис.
        assert_eq!(safe_dir("/home/user/my dir/").unwrap(), "/home/user/my-dir/");
    }

    #[test]
    fn directory_path_with_command_substitution_becomes_inert() {
        // `$(...)` внутри двойных кавычек исполняется как код. Allowlist это
        // прерывает.
        assert_eq!(
            safe_dir("/path/$(whoami)/file").unwrap(),
            "/path/--whoami-/file"
        );
    }

    #[test]
    fn directory_path_with_backtick_becomes_inert() {
        // Backtick — историческая форма подстановки команд, и она тоже исполняется
        // внутри двойных кавычек.
        assert_eq!(safe_dir("/path/`whoami`/file").unwrap(), "/path/-whoami-/file");
    }

    #[test]
    fn directory_path_with_quote_becomes_inert() {
        // Кавычка развалит оболочку из строки в команде.
        assert_eq!(safe_dir("/path/\"quoted\"/file").unwrap(), "/path/-quoted-/file");
    }

    #[test]
    fn default_remote_dir_survives_unchanged() {
        // Это умолчание, и его не должна поломать ни одна санитизация. Если
        // allowlist его изменит, все установки с настройками по умолчанию сломаны.
        assert_eq!(safe_dir("~/.ccfzf/windows").unwrap(), "~/.ccfzf/windows");
    }

    #[test]
    fn whitespace_only_directory_produces_error() {
        // Пустой путь ничего не означает. Вернуть ошибку лучше, чем молча
        // писать куда-то, куда пользователь не просил.
        assert!(safe_dir("   ").is_err());
        let err = safe_dir("   ").unwrap_err();
        assert_eq!(err, "remoteDir is not set");
    }
}
