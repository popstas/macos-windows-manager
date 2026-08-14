use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // Штамп времени сборки виден в пункте меню трея. Нужен он затем, что
    // `deploy-mac.sh` обновляет бинарь на месте: после перезапуска нечем
    // проверить, что поднялось именно новое, — версия у всех сборок между
    // релизами одна, и убеждаться приходится `git log`-ом по ssh.
    //
    // Признак «это релиз» объявляет сборщик переменной окружения, а не
    // cargo-профиль: `deploy-mac.sh` собирает `--release`, и на
    // `debug_assertions` штамп пропал бы ровно там, где он и нужен. Сейчас
    // переменную не выставляет никто — релиза у проекта пока нет вовсе, — так
    // что штамп есть везде, а место под будущий CI готово.
    println!("cargo:rerun-if-env-changed=MWM_RELEASE");
    // Без явного rerun-if-changed штамп застыл бы на первой сборке: любая
    // директива `cargo:rerun-if-*` отменяет умолчание «пересобирать скрипт на
    // любую правку в пакете», а его отменяет уже `tauri_build::build()`
    // своими. Проверено на пикере, где это ровно так и вышло.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    let stamp = if std::env::var_os("MWM_RELEASE").is_some() {
        0
    } else {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    };
    println!("cargo:rustc-env=MWM_BUILD_UNIX={stamp}");
    tauri_build::build()
}
