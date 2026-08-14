# Settings Window Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Окно настроек в трее с тумблерами трёх фич (расстановка окон, снимки раскладки, просьбы по MQTT) и редактором основных полей конфига; всё включено по умолчанию.

**Architecture:** Флаги живут в `config.yaml` блоком `features:` и разбираются в `parse_config` покомандно с умолчанием `true`. Такт трекера берёт конфиг из разделяемой ячейки `Arc<Mutex<Config>>` в начале каждого оборота, поэтому сохранение из окна действует со следующего такта. Окно — второй webview, создаётся лениво командой из меню трея; страница самодостаточна и обходится без бандлера.

**Tech Stack:** Rust, Tauri 2.11.5, `serde_yaml` 0.9, `serde_json` 1, обычный HTML+JS без сборки.

**Spec:** `docs/superpowers/specs/2026-08-14-settings-window-design.md`

## Global Constraints

- **Всё, что видит человек, — по-английски.** Названия пунктов меню, подписи полей, тексты ошибок в трее и в окне. Комментарии и тесты — по-русски, как во всём проекте.
- **Комментарий объясняет причину, а не действие.** Правило проекта: в коде пишется, почему так, а не что делает строка.
- **Умолчание на поле, а не отказ на блок.** Мусор в одном ключе конфига стоит этого ключа, а не всех настроек.
- **Тесты гоняются на Linux:** `cargo test --workspace`. Ветки под `#[cfg(target_os = "macos")]` на машине разработки не компилируются вовсе — всё, что их касается, проверяется только выкаткой.
- **`data/scripts/deploy-mac.sh` выходит с кодом 0, даже когда сборка упала.** Смотреть последнюю строку вывода: дошло до `deployed to <хост>` — дошло; кончилось на выводе `cargo` — нет.
- **Хосты выкатки:** `mac.popstas.pro` (MacBook), `mac.popstas.ru` (Mac mini). Ветка задаётся `MWM_BRANCH`.
- **Непроверяемую ветку сверяют с исходниками зависимостей** в `~/.cargo/registry/src/`, а не с памятью.

---

### Task 1: Флаги в конфиге

**Files:**
- Modify: `crates/mwm-core/src/config.rs`

**Interfaces:**
- Consumes: ничего.
- Produces: `pub struct Features { pub placement: bool, pub snapshots: bool, pub requests: bool }` с `impl Default` (все `true`); поле `pub features: Features` в `Config`; `pub fn to_json(cfg: &Config) -> serde_json::Value`.

- [ ] **Step 1: Написать падающие тесты**

В конец `mod tests` в `crates/mwm-core/src/config.rs`:

```rust
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
```

- [ ] **Step 2: Убедиться, что тесты падают**

Run: `cargo test -p mwm-core config::`
Expected: FAIL — `cannot find type Features in this scope`, `cannot find function to_json in this scope`.

- [ ] **Step 3: Реализовать**

В `crates/mwm-core/src/config.rs`, рядом с `MqttConfig`:

```rust
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
```

В `Config` — новое поле:

```rust
    /// Выключатели фич. Пустой блок значит «всё включено».
    pub features: Features,
```

В `parse_config`, перед сборкой `Config`:

```rust
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
```

и `features` в список полей возвращаемого `Config`.

В конец файла, перед `mod tests`:

```rust
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
```

- [ ] **Step 4: Убедиться, что тесты проходят**

Run: `cargo test --workspace`
Expected: PASS, все прежние тесты тоже зелёные.

- [ ] **Step 5: Коммит**

```bash
git add crates/mwm-core/src/config.rs
git commit -m "feat(config): блок features: с тремя флагами, все включены по умолчанию"
```

---

### Task 2: Три гейта в такте трекера

**Files:**
- Modify: `src-tauri/src/main.rs`

**Interfaces:**
- Consumes: `mwm_core::config::Features` из задачи 1.
- Produces: локальные `let features = cfg.features.clone();` и `let focus = link.is_live() && features.requests;` в теле цикла `run_tracker` — задача 4 подменяет первую строку чтением из ячейки, а тест-сторож ниже держит вторую на месте.

- [ ] **Step 1: Написать падающий тест-сторож**

В конец `src-tauri/src/main.rs`:

```rust
/// Тесты-сторожа: они читают исходник, а не зовут код.
///
/// Такт трекера не разложить на чистые функции без переписывания всего файла, а
/// проверить эти две связки надо: обе рвутся молча, и обе видны только на живом
/// маке. Приём взят у соседнего ccfzf-picker, где точно так же сторожится пункт
/// меню трея.
#[cfg(test)]
mod tests {
    #[test]
    fn focus_is_gated_by_the_requests_flag() {
        // Объявить умение поднимать окно, не собираясь его поднимать, значит
        // подарить человеку молчащий Enter в пикере — а это хуже открытого
        // терминала.
        let src = include_str!("main.rs");
        assert!(
            src.contains("let focus = link.is_live() && features.requests;"),
            "focus должен считаться от живого соединения И включённого флага"
        );
        // Отпечаток считается от того же значения: иначе выключенный тумблер
        // доехал бы до файла окон только со следующим сердцебиением, до
        // полуминуты спустя.
        assert!(src.contains("fingerprint(&bound, focus)"), "отпечаток берёт то же focus");
        assert!(src.contains("focus,\n"), "build_file получает то же focus");
    }
}
```

- [ ] **Step 2: Убедиться, что тест падает**

Run: `cargo test -p macos-windows-manager`
Expected: FAIL — `focus должен считаться от живого соединения И включённого флага`.

- [ ] **Step 3: Реализовать три гейта**

В `run_tracker`, первой строкой тела `loop`:

```rust
    loop {
        // Настройки берутся один раз за оборот, а не по месту: иначе половина
        // такта работала бы по старым флагам, половина по новым, и объяснить
        // человеку увиденное было бы нечем.
        let features = cfg.features.clone();
```

В разборе очереди просьб, внутри `for req in pending`, первой строкой:

```rust
                for req in pending {
                    // Очередь вычитывается всегда, а исполняется по флагу:
                    // невычитанные просьбы копились бы в канале и хлынули бы все
                    // разом в тот момент, когда человек вернёт тумблер.
                    if !features.requests {
                        continue;
                    }
```

Расстановка:

```rust
        let mut place_note = String::new();
        // Выключается только сама расстановка. Слоты продолжают вестись, и
        // `state.json` продолжает писаться: иначе выключенный тумблер стирал бы
        // человеку запомненные места, и вернувший его обратно не вернул бы их.
        if features.placement {
            for (window_id, want) in tracker.placements() {
                let target = mwm_core::geometry::clamp_to_displays(want, &screens);
                if let Err(e) = ax::place(&registry, window_id, target) {
                    eprintln!("mwm: place failed: {e}");
                    place_note = format!("place failed: {e}");
                }
            }
        }
```

Снимки — весь блок от `let open = tracker.open_session_ids();` до закрывающей скобки `if let Some(d) = decision { … }` оборачивается в:

```rust
        if features.snapshots {
            // … существующий блок целиком, без изменений внутри …
        }
```

Ниже, перед `let print = …`, появляется `focus`, и он же уходит в отпечаток и в файл:

```rust
        // Одно значение на оба применения. Разойдись они — отпечаток не заметил
        // бы смены флага, и файл окон дожил бы со старым `focus` до
        // сердцебиения.
        let focus = link.is_live() && features.requests;

        let print = fingerprint(&bound, focus);
```

и в `build_file` вместо `link.is_live()`:

```rust
            let payload = build_file(
                &bound,
                &cfg.host,
                pid,
                now,
                focus,
                &cfg.mqtt.base,
                // Выключенные снимки не только не пишутся на диск, но и не
                // публикуются: иначе пикер показывал бы в `^S` раскладки,
                // которые эта машина больше не ведёт.
                if features.snapshots { &snaps } else { &[] },
            );
```

- [ ] **Step 4: Убедиться, что тесты проходят**

Run: `cargo test --workspace && cargo check --workspace`
Expected: PASS, сборка чистая.

- [ ] **Step 5: Коммит**

```bash
git add src-tauri/src/main.rs
git commit -m "feat(tracker): три флага гасят расстановку, снимки и просьбы"
```

---

### Task 3: Запись конфига

**Files:**
- Create: `src-tauri/src/config_file.rs`
- Modify: `src-tauri/src/main.rs`, `src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: ничего из прошлых задач.
- Produces: `config_file::HEADER: &str`, `config_file::merge_patch(&mut serde_yaml::Value, &serde_json::Value) -> Result<(), String>`, `config_file::render(&serde_yaml::Value) -> Result<String, String>`; в `main.rs` — `fn reject_null_values(&serde_json::Value) -> Result<(), String>`, `fn restrict_permissions(&Path)`, `fn write_config(&Path, &serde_json::Value) -> Result<(), String>`.

- [ ] **Step 1: Написать падающие тесты**

Создать `src-tauri/src/config_file.rs` целиком — вместе с `mod tests`, портированным из `ccfzf-picker/src-tauri/src/config_file.rs`: `untouched_keys_survive`, `nested_maps_merge_key_by_key`, `lists_are_replaced_whole`, `empty_document_becomes_a_mapping`, `non_object_patch_is_refused`, `header_is_a_comment_and_does_not_accumulate`. Тексты тестов брать оттуда дословно, заменив в `HEADER` имя приложения и ссылку на пример конфига:

```rust
pub const HEADER: &str = "\
# This file is managed by the macos-windows-manager settings window: saving
# rewrites it whole, and comments in it are not preserved. The previous file is
# next to it, as config.yaml.bak. All keys are documented in the repository's
# config.example.yml.
";
```

В `mod tests` в `src-tauri/src/main.rs` добавить файловые тесты:

```rust
    /// Свой каталог во временной директории на каждый тест: `write_config`
    /// трогает настоящую файловую систему, и тесты не должны видеть файлы друг
    /// друга при параллельном запуске.
    fn temp_config_path(tag: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir()
            .join(format!("mwm-test-{}-{tag}-{n}", std::process::id()))
            .join("config.yaml")
    }

    #[test]
    fn write_config_creates_missing_directory_and_file() {
        let path = temp_config_path("missing-dir");
        assert!(!path.parent().unwrap().exists());
        super::write_config(&path, &serde_json::json!({"sshHost": "host"})).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("host"));
    }

    #[test]
    fn write_config_keeps_untouched_keys() {
        // Блок `state:` форма не показывает вовсе, и перезапись файла целиком
        // стёрла бы человеку настроенные пути молча.
        let path = temp_config_path("untouched");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "sshHost: old\nstate:\n  path: /tmp/s.json\n").unwrap();
        super::write_config(&path, &serde_json::json!({"sshHost": "new"})).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("new"), "{text}");
        assert!(!text.contains("old"), "{text}");
        assert!(text.contains("/tmp/s.json"), "чужой ключ пережил запись: {text}");
    }

    #[test]
    fn write_config_backs_up_once() {
        // Второе сохранение затёрло бы единственную копию исходного файла его
        // же, уже применённым, состоянием — и комментарии человека пропали бы
        // окончательно.
        let path = temp_config_path("backup-once");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "sshHost: original\n").unwrap();
        super::write_config(&path, &serde_json::json!({"sshHost": "first"})).unwrap();
        super::write_config(&path, &serde_json::json!({"sshHost": "second"})).unwrap();
        let backup = std::fs::read_to_string(path.with_extension("yaml.bak")).unwrap();
        assert!(backup.contains("original"), "{backup}");
    }

    #[test]
    fn write_config_does_not_back_up_whitespace_only_file() {
        // Иначе файл из пробелов занял бы единственный слот `.bak` навсегда.
        let path = temp_config_path("whitespace-only");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "   \n\n").unwrap();
        super::write_config(&path, &serde_json::json!({"sshHost": "host"})).unwrap();
        assert!(!path.with_extension("yaml.bak").exists());
    }

    #[test]
    fn write_config_rejects_null_at_any_depth() {
        // `{"mqtt": {"password": null}}` стирает ровно тот ключ, ради которого
        // написано слияние по ключам.
        let path = temp_config_path("null-nested");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "mqtt:\n  host: broker\n  password: secret\n").unwrap();
        let err = super::write_config(&path, &serde_json::json!({"mqtt": {"password": null}}))
            .unwrap_err();
        assert!(err.contains("password"), "{err}");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("secret"), "пароль пережил отказ: {text}");
    }

    #[cfg(unix)]
    #[test]
    fn write_config_keeps_files_private() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_config_path("permissions");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Права нарочно шире нужных: сохранение обязано их сузить, а не
        // унаследовать.
        std::fs::write(&path, "mqtt:\n  password: secret\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        super::write_config(&path, &serde_json::json!({"sshHost": "host"})).unwrap();
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600, "config.yaml");
        assert_eq!(mode(&path.with_extension("yaml.bak")), 0o600, "config.yaml.bak");
    }
```

- [ ] **Step 2: Убедиться, что тесты падают**

Run: `cargo test --workspace`
Expected: FAIL — `cannot find function write_config`, `file not found for module config_file`.

- [ ] **Step 3: Реализовать**

В `src-tauri/Cargo.toml`, в `[dependencies]`:

```toml
# Конфиг теперь не только читается, но и пишется — окном настроек.
serde_yaml = "0.9"
```

В `src-tauri/src/main.rs` — `mod config_file;` к остальным модулям, и три функции рядом с `load_config`. Тела `merge_patch`, `render`, `reject_null_values`, `restrict_permissions`, `write_config` переносятся из `ccfzf-picker` дословно (`src-tauri/src/config_file.rs` и `src-tauri/src/main.rs:429-541` там), с двумя правками: имя приложения в сообщениях об ошибке `restrict_permissions` и путь конфига берётся из `mwm_core::config::config_path`, а не строится заново:

```rust
/// Путь к config.yaml. Общий для чтения и записи: разойдись они двумя копиями,
/// один читал бы не тот файл, что другой пишет.
fn config_file_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(mwm_core::config::config_path(&home))
}
```

- [ ] **Step 4: Убедиться, что тесты проходят**

Run: `cargo test --workspace`
Expected: PASS — 6 тестов в `config_file`, 6 файловых в `main`.

- [ ] **Step 5: Коммит**

```bash
git add src-tauri/src/config_file.rs src-tauri/src/main.rs src-tauri/Cargo.toml
git commit -m "feat(config): запись config.yaml патчем, с бэкапом и правами 0600"
```

---

### Task 4: Ячейка конфига и команды

**Files:**
- Modify: `src-tauri/src/main.rs`

**Interfaces:**
- Consumes: `to_json` (задача 1), `write_config`/`config_file_path` (задача 3), `let features = cfg.features.clone();` (задача 2).
- Produces: `struct Shared(Arc<Mutex<Config>>)` с методом `fn get(&self) -> Config`; команды `load_settings` → `{"file": …, "effective": …}` и `save_settings(patch)` → `Result<(), String>`; `run_tracker(status: Status, trusted: Trusted, shared: Shared)`.

- [ ] **Step 1: Написать падающий тест-сторож**

В `mod tests` в `src-tauri/src/main.rs`:

```rust
    #[test]
    fn the_tick_rereads_the_shared_config() {
        // Без этого сохранённый тумблер молчал бы до перезапуска, а молча не
        // подействовавший тумблер хуже отсутствующего. Проверяется текстом:
        // такт не разложить на чистые функции, не переписав файл целиком.
        let src = include_str!("main.rs");
        assert!(
            src.contains("let cfg = shared.get();"),
            "такт обязан брать конфиг из ячейки, а не из копии, прочитанной на старте"
        );
        assert!(
            src.contains("*shared.0.lock().unwrap() = load_config();"),
            "сохранение обязано класть в ячейку перечитанный с диска конфиг"
        );
    }
```

- [ ] **Step 2: Убедиться, что тест падает**

Run: `cargo test -p macos-windows-manager the_tick_rereads`
Expected: FAIL — `такт обязан брать конфиг из ячейки`.

- [ ] **Step 3: Реализовать**

Рядом со `struct Trusted`:

```rust
/// Действующие настройки.
///
/// Ячейка, а не копия при старте: окно настроек пишет файл на ходу, и такт
/// обязан узнать об этом со следующего оборота. Читает её трекер, пишет —
/// команда сохранения.
#[derive(Clone)]
struct Shared(Arc<Mutex<Config>>);

impl Shared {
    /// Снимок настроек на один оборот такта. Копия, а не блокировка на весь
    /// оборот: держать мьютекс, пока идёт ssh за дампом, значило бы подвесить
    /// на это время окно настроек.
    fn get(&self) -> Config {
        self.0.lock().unwrap().clone()
    }
}
```

`run_tracker` принимает третьим аргументом `shared: Shared`. Строка `let cfg = load_config();` в её начале остаётся — по ней заводятся вещи, которые на лету не меняются (пути состояния, поток MQTT). Внутри `loop`, вместо `let features = cfg.features.clone();` из задачи 2:

```rust
        // Настройки берутся один раз за оборот, а не по месту: иначе половина
        // такта работала бы по старым, половина по новым.
        //
        // Затеняет внешний `cfg` намеренно: всё, что ниже, обязано смотреть на
        // свежие настройки, и «забыл переименовать» здесь означало бы тихо
        // работающий по старому кусок такта.
        let cfg = shared.get();
        let features = cfg.features.clone();
```

Две команды:

```rust
/// Настройки, как их покажет окно.
///
/// Две картины, а не одна. `file` — то, что лежит в config.yaml: по нему форма
/// заполняет поля и по нему же считает, что человек тронул. `effective` — то,
/// что подхватил трекер, с умолчаниями: оно показывается подсказкой. Второй
/// копии умолчаний в JS нет намеренно — она разошлась бы с `parse_config` на
/// первой же правке.
#[tauri::command]
fn load_settings(shared: tauri::State<'_, Shared>) -> Result<serde_json::Value, String> {
    let path = config_file_path();
    let file: serde_yaml::Value = match std::fs::read_to_string(&path) {
        Ok(text) if !text.trim().is_empty() => serde_yaml::from_str(&text)
            .map_err(|e| format!("bad yaml in {}: {e}", path.display()))?,
        // Файла нет или он пуст — не ошибка: окно настроек и заводится затем,
        // чтобы его создать.
        Ok(_) => serde_yaml::Value::Null,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_yaml::Value::Null,
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let file = serde_json::to_value(&file).map_err(|e| format!("cannot convert yaml: {e}"))?;
    Ok(serde_json::json!({
        "file": file,
        "effective": mwm_core::config::to_json(&shared.get()),
    }))
}

/// Сохранить присланное формой и объявить это трекеру.
///
/// Ячейка обновляется перечитыванием файла, а не патчем поверх прежнего
/// конфига: разбирает YAML `parse_config`, и только он знает, во что
/// превращаются мусор и умолчания. Собери мы новый `Config` из патча — окно
/// показывало бы одно, а трекер работал бы по другому.
#[tauri::command]
fn save_settings(shared: tauri::State<'_, Shared>, patch: serde_json::Value) -> Result<(), String> {
    write_config(&config_file_path(), &patch)?;
    *shared.0.lock().unwrap() = load_config();
    Ok(())
}
```

В `setup`, до запуска потока трекера:

```rust
            let shared = Shared(Arc::new(Mutex::new(load_config())));
            app.manage(shared.clone());
```

и `std::thread::spawn(move || run_tracker(worker, worker_trusted, shared));`

В `tauri::Builder::default()` — первым звеном:

```rust
        .invoke_handler(tauri::generate_handler![load_settings, save_settings])
```

- [ ] **Step 4: Убедиться, что тесты проходят**

Run: `cargo test --workspace && cargo check --workspace`
Expected: PASS.

- [ ] **Step 5: Коммит**

```bash
git add src-tauri/src/main.rs
git commit -m "feat(settings): ячейка настроек и команды чтения-записи"
```

---

### Task 5: Окно, пункт трея и ветка выхода

**Files:**
- Create: `src-tauri/capabilities/default.json`, `frontend/settings.html`
- Modify: `src-tauri/src/main.rs`, `src-tauri/tauri.conf.json`

**Interfaces:**
- Consumes: `load_settings`/`save_settings` (задача 4).
- Produces: команда `open_settings`; пункт меню `Settings…`; ветка `RunEvent::ExitRequested { code: None, .. }`.

- [ ] **Step 1: Написать падающий тест-сторож**

В `mod tests` в `src-tauri/src/main.rs`:

```rust
    #[test]
    fn only_the_windowless_exit_is_prevented() {
        // Закрытие последнего окна и `app.exit(0)` приезжают одним событием, и
        // отличаются только кодом: `None` у первого, `Some(0)` у второго
        // (tauri-runtime-wry 2.10.0, src/lib.rs:4177 и 4217). Глухая ветка на
        // все коды сделала бы трей неубиваемым, а отсутствие ветки — крестик на
        // окне настроек гасил бы трекер.
        let src = include_str!("main.rs");
        assert!(
            src.contains("RunEvent::ExitRequested { code: None, api, .. }"),
            "ветка выхода обязана быть только для code: None"
        );
        assert!(src.contains("api.prevent_exit()"), "и обязана звать prevent_exit");
    }

    #[test]
    fn the_tray_has_a_settings_item() {
        let src = include_str!("main.rs");
        assert!(src.contains("\"settings\" =>"), "пункт settings обязан быть в обработчике меню");
    }
```

- [ ] **Step 2: Убедиться, что тесты падают**

Run: `cargo test -p macos-windows-manager`
Expected: FAIL — `ветка выхода обязана быть только для code: None`.

- [ ] **Step 3: Реализовать**

`src-tauri/capabilities/default.json`:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "settings window",
  "windows": ["settings"],
  "permissions": ["core:default"]
}
```

`src-tauri/tauri.conf.json` — в блок `"app"`:

```json
    "withGlobalTauri": true,
```

(даёт `window.__TAURI__.core.invoke` без бандлера — иначе странице нечем звать команды, а сборщика в проекте нет.)

`frontend/settings.html` — вертикальный срез: три галки, чтение и запись через те же команды, что понадобятся полной форме.

```html
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>macos-windows-manager Settings</title>
<style>
  :root { color-scheme: dark; }
  body {
    margin: 0; padding: 18px 22px;
    font: 13px -apple-system, system-ui, sans-serif;
    background: #1c1c1e; color: #e6e6e6;
  }
  h2 { font-size: 13px; font-weight: 600; margin: 0 0 10px; }
  .field { margin-bottom: 10px; }
  .hint { color: #999; margin-top: 3px; }
  #status { color: #999; margin-left: 10px; }
  #status.bad { color: #ff7b72; }
</style>
</head>
<body>
<h2>Features</h2>
<div id="features"></div>
<button id="save">Save</button><span id="status"></span>
<script>
(async function () {
  const invoke = (cmd, args) => (window.__TAURI__
    ? window.__TAURI__.core.invoke(cmd, args)
    : Promise.resolve());

  const FLAGS = [
    ['placement', 'Place windows where they were'],
    ['snapshots', 'Keep layout snapshots'],
    ['requests', 'Serve raise and unread requests over MQTT'],
  ];
  const status = document.getElementById('status');
  let loaded = {};

  function render(file) {
    document.getElementById('features').innerHTML = FLAGS.map(([key, label]) => {
      // Пусто в файле значит «включено»: тот же ответ, что даёт parse_config.
      const on = (file.features || {})[key] !== false;
      return `<div class="field"><label>
        <input type="checkbox" id="f-${key}" ${on ? 'checked' : ''}> ${label}
      </label></div>`;
    }).join('');
  }

  async function load() {
    const out = await invoke('load_settings') || {};
    loaded = out.file || {};
    render(loaded);
  }

  document.getElementById('save').addEventListener('click', async () => {
    // Патч только из тронутого: иначе умолчания впечатывались бы в файл
    // человеку, который их не выбирал.
    const patch = {};
    const features = {};
    for (const [key] of FLAGS) {
      const now = document.getElementById(`f-${key}`).checked;
      const was = (loaded.features || {})[key] !== false;
      if (now !== was) features[key] = now;
    }
    if (Object.keys(features).length) patch.features = features;
    if (!Object.keys(patch).length) { status.textContent = 'nothing changed'; return; }
    try {
      await invoke('save_settings', { patch });
      status.className = '';
      status.textContent = 'saved';
      await load();
    } catch (e) {
      status.className = 'bad';
      status.textContent = String(e);
    }
  });

  await load();
})();
</script>
</body>
</html>
```

В `src-tauri/src/main.rs` — команда открытия:

```rust
/// Открыть окно настроек.
///
/// Создаётся лениво: объявленное в `tauri.conf.json` окно поднималось бы на
/// старте, и второй webview висел бы в памяти у каждого, кто в настройки не
/// заходит.
///
/// `async` здесь не украшение. Синхронную команду Tauri исполняет прямо в
/// потоке цикла событий, а создание webview этот же цикл и ждёт: `build()`
/// возвращает Ok, окно появляется, а страница в нём не загружается никогда —
/// белый прямоугольник. Заплачено этим в соседнем ccfzf-picker.
#[tauri::command]
async fn open_settings(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(
        &app,
        "settings",
        tauri::WebviewUrl::App("settings.html".into()),
    )
    .title("macos-windows-manager Settings")
    .inner_size(560.0, 620.0)
    .center()
    .resizable(true)
    .build()
    .map_err(|e| format!("cannot open settings window: {e}"))?;
    Ok(())
}
```

Пункт меню — рядом с `quit`:

```rust
            let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
```

и в оба варианта сборки меню, между `grant`/`state` и `quit`:

```rust
            let menu = if trusted_now {
                Menu::with_items(app, &[&state, &settings, &quit, &version])?
            } else {
                Menu::with_items(app, &[&state, &grant, &settings, &quit, &version])?
            };
```

В обработчик событий меню:

```rust
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "grant" => ax::prompt_for_trust(),
                    "settings" => {
                        // Команда `async`, а обработчик — нет; отправляем её в
                        // пул той же причины ради, что расписана у
                        // `open_settings`.
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = open_settings(app).await {
                                eprintln!("mwm: {e}");
                            }
                        });
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
```

`open_settings` — в список `generate_handler!`. И `main()` меняет форму:

```rust
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![load_settings, save_settings, open_settings])
        .setup(|app| { /* … без изменений … */ })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            // Закрытие последнего окна гасит приложение по умолчанию
            // (tauri-runtime-wry 2.10.0, src/lib.rs:4177): у трея окон не было
            // вовсе, и заметить это стало возможно только с появлением первого.
            // Крестик на форме настроек не имеет права снять трекер.
            //
            // Только `code: None`. `app.exit(0)` из пункта `Quit` приезжает тем
            // же событием с `code: Some(0)`, и глухая ветка сделала бы трей
            // неубиваемым.
            if let tauri::RunEvent::ExitRequested { code: None, api, .. } = event {
                api.prevent_exit();
            }
        });
```

- [ ] **Step 4: Убедиться, что тесты проходят и код собирается**

Run: `cargo test --workspace && cargo check --workspace`
Expected: PASS.

- [ ] **Step 5: Выкатить и проверить на маке**

```bash
MWM_BRANCH=feat/settings-window MWM_HOST=mac.popstas.pro ./data/scripts/deploy-mac.sh
```

Смотреть последнюю строку вывода — `deployed to …`, а не вывод `cargo`. На маке проверить руками:
- пункт `Settings…` есть в меню и открывает окно;
- окно выходит вперёд (если нет — правка в `open_settings`, см. открытый вопрос 1 в спеке);
- значка в доке не появилось;
- **крестик на окне не убрал значок из строки меню**, публикация продолжается;
- `Quit` по-прежнему гасит приложение;
- галка снимается, `Save` пишет файл: `ls -l ~/.config/macos-windows-manager/config.yaml*` — есть `.bak`, права `-rw-------`.

- [ ] **Step 6: Коммит**

```bash
git add src-tauri/src/main.rs src-tauri/tauri.conf.json src-tauri/capabilities/default.json frontend/settings.html
git commit -m "feat(tray): окно настроек с тумблерами фич"
```

---

### Task 6: Полная форма и документация

**Files:**
- Modify: `frontend/settings.html`, `src-tauri/src/main.rs`, `config.example.yml`, `README.md`

**Interfaces:**
- Consumes: `load_settings` (обе картины), `save_settings` (задача 4).
- Produces: ничего для следующих задач — это последняя.

- [ ] **Step 1: Написать падающий тест-сторож**

В `mod tests` в `src-tauri/src/main.rs`:

```rust
    #[test]
    fn the_form_covers_every_field_it_promises() {
        // Поле, забытое в форме, выглядит не поломкой, а отсутствующей
        // настройкой: человек ищет его глазами и не находит. Дешёвый сторож
        // ровно на этот случай.
        let page = include_str!("../../frontend/settings.html");
        for key in [
            "placement", "snapshots", "requests",
            "sshHost", "remoteDir", "windowHost",
            "terminals", "tickMs", "dumpCacheMs",
            "host", "port", "user", "password", "base",
        ] {
            assert!(page.contains(key), "поле {key} пропало из формы");
        }
        assert!(
            page.contains("after restart"),
            "группа MQTT обязана честно говорить, что действует после перезапуска"
        );
    }
```

- [ ] **Step 2: Убедиться, что тест падает**

Run: `cargo test -p macos-windows-manager the_form_covers`
Expected: FAIL — `поле sshHost пропало из формы`.

- [ ] **Step 3: Дописать форму**

В `frontend/settings.html` описание полей становится таблицей, а разметка — общей для всех типов:

```javascript
  const GROUPS = [
    { title: 'Features', note: '', fields: [
      ['features.placement', 'check', 'Place windows where they were'],
      ['features.snapshots', 'check', 'Keep layout snapshots'],
      ['features.requests', 'check', 'Serve raise and unread requests over MQTT'],
    ]},
    { title: 'Connection', note: '', fields: [
      ['sshHost', 'text', 'Machine where sessions and the aggregator live'],
      ['remoteDir', 'text', 'Directory of tracker files on that machine'],
      ['windowHost', 'text', 'Name of this machine, as the picker knows it'],
    ]},
    { title: 'Windows', note: '', fields: [
      ['terminals', 'lines', 'Bundle ids counted as terminals, one per line'],
      ['tickMs', 'number', 'Poll interval, milliseconds'],
      ['dumpCacheMs', 'number', 'Session index cache age, milliseconds'],
    ]},
    { title: 'MQTT', note: 'Takes effect after restart.', fields: [
      ['mqtt.host', 'text', 'Broker address'],
      ['mqtt.port', 'number', 'Broker port'],
      ['mqtt.user', 'text', 'User'],
      ['mqtt.password', 'password', 'Leave empty to keep the stored one'],
      ['mqtt.base', 'text', 'Topic prefix of this machine'],
    ]},
  ];
```

Правила, которые форма обязана соблюдать, — каждое одной причиной:

- значение поля берётся из `file` по пути с точкой (`mqtt.host`); отсутствующее — пустая строка, а не умолчание: пустое поле честно значит «в файле не сказано»;
- рядом показывается `effective` подсказкой (`in effect: 1000`) — так окно отвечает на вопрос «какой конфиг сейчас в работе», с которого начинается расследование молчащего трекера;
- `lines` разбирается как список непустых строк, `number` — как целое; нечисло в числовом поле — отказ на месте, без записи файла;
- патч собирается **только из отличий от `file`**;
- пустое поле пароля в патч не попадает вовсе — иначе сохранение стирало бы сохранённый пароль каждому, кто открыл форму и нажал `Save`;
- галка `features.*` считается тронутой, если её состояние отличается от `file.features[key] !== false`.

- [ ] **Step 4: Убедиться, что тесты проходят**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Документация**

В `config.example.yml`, после блока `terminals`:

```yaml
# Feature switches. Every one of them defaults to on: a config written before
# this block existed must keep behaving exactly as it did.
#
# The settings window writes them here. Editing by hand works the same way.
# features:
#   # Put an appearing window back where its session had it last time.
#   placement: true
#   # Keep layout snapshots (the ^S list in the picker).
#   snapshots: true
#   # Serve requests arriving over MQTT: raise a window, mark a session unread.
#   # Switched off, the tracker also stops announcing `focus` in its window
#   # file — a picker that offers Enter and does nothing is worse than a
#   # terminal opened by hand.
#   requests: true
```

В `README.md`, новым разделом после «Разрешения»:

```markdown
## Настройки

Пункт `Settings…` в меню трея открывает окно настроек: три тумблера фич
(расстановка окон, снимки раскладки, просьбы по MQTT — все включены по
умолчанию) и основные поля конфига.

**Сохранение переписывает `config.yaml` целиком, и комментарии в нём не
сохраняются.** Прежний файл кладётся рядом как `config.yaml.bak` — один раз,
перед первой перезаписью, чтобы второе сохранение не затёрло его уже применённым
состоянием. Ключи документированы в `config.example.yml`.

Всё, кроме блока `mqtt:`, действует со следующего такта — поток подписки поднят
на старте и останавливаться не умеет, поэтому поля брокера помечены в форме
«takes effect after restart».

Выключенная расстановка не стирает запомненные места: `state.json` продолжает
вестись, и включённая обратно расставляет окна, появившиеся после включения.
Выключенные просьбы убирают `focus` из публикуемого файла окон — пикер перестаёт
предлагать Enter, вместо того чтобы предлагать его впустую.

Крестик на окне настроек не гасит трекер: закрытие последнего окна по умолчанию
завершает Tauri-приложение, и в `main()` стоит ветка, которая этому мешает.
Гасит только `Quit`.
```

- [ ] **Step 6: Выкатить на оба мака и проверить**

```bash
MWM_BRANCH=feat/settings-window MWM_HOST=mac.popstas.pro ./data/scripts/deploy-mac.sh
MWM_BRANCH=feat/settings-window MWM_HOST=mac.popstas.ru ./data/scripts/deploy-mac.sh
```

Чеклист (он же уедет в PR):
- все четырнадцать полей на месте и заполнены тем, что в файле;
- подсказка `in effect:` показывает действующее значение там, где поле пусто;
- правка `tickMs` действует со следующего такта, правка `mqtt.host` — только после перезапуска;
- пустое поле пароля не стирает сохранённый пароль (после `Save` брокер по-прежнему подключается);
- выключенная расстановка перестаёт двигать окна, `state.json` продолжает вестись;
- выключенные просьбы убирают `focus` в файле окон на агрегаторе;
- выключенные снимки перестают писать `snapshots.json`.

- [ ] **Step 7: Коммит**

```bash
git add frontend/settings.html src-tauri/src/main.rs config.example.yml README.md
git commit -m "feat(settings): полная форма конфига и документация"
```

---

## Самопроверка плана

Пройдено по спеке раздел за разделом:

| Раздел спеки | Где реализуется |
|---|---|
| 1. Флаги, разбор, умолчания | Задача 1 |
| 1. Применение на лету, ячейка | Задача 4 |
| 1. Три гейта, `focus` по флагу | Задача 2 |
| 2. Ветка `ExitRequested` | Задача 5 |
| 2. Пункт трея, `async open_settings` | Задача 5 |
| 2. `withGlobalTauri`, `capabilities` | Задача 5 |
| 3. Запись патчем, `.bak`, права, отказ на `null` | Задача 3 |
| 4. Состав формы, `file` + `effective`, пароль | Задачи 5 (срез) и 6 (полностью) |
| 5. Тесты, сторожа, ручная проверка | В каждой задаче свои шаги |

Одна поправка к спеке, найденная при разборе `tracker.placements()`: список расстановок живёт ровно один такт и собирается внутри `tick`, значит выключенный тумблер расстановки **не откладывает** расстановку до включения — окно, появившееся при выключенном тумблере, останется там, где его открыла система. Формулировку в чеклисте спеки («включённая обратно расставляет по запомненным местам») план уточняет до «расставляет окна, появившиеся после включения». Это ожидаемое поведение, а не дефект: расстановка — про момент появления окна.
