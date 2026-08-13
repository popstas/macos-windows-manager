# macos-windows-manager

Оконный трекер claude-wt для macOS. Следит за окнами терминалов, привязывает
их заголовки к сессиям и кладёт список на машину, где живёт агрегатор `ccfzf`.
Окон у самого приложения нет — только значок в трее.

Спека: `ccfzf-picker/docs/superpowers/specs/2026-08-13-macos-windows-manager-design.md`.

## Сборка и тесты

```
cargo test -p mwm-core       # логика, гоняется на любой машине
cargo build --release        # приложение, только на macOS
```

## Разрешения

Нужно одно — Accessibility. Без него окна не перечисляются и файл не пишется
вовсе: пустой файл означал бы «окон нет» и погасил бы чужие пометки.

## Выкатка

```
MWM_HOST=<ssh host> ./data/scripts/deploy-mac.sh
MWM_HOST=<ssh host> ./data/scripts/deploy-mac.sh --no-build   # только перезапустить
```

Разово на маке: `rustup`, разрешение Accessibility, LaunchAgent (см.
`docs/launchagent.md`).

## Проверка, что доехало

На машине агрегатора:

```
python3 -c "import json,time;o=json.load(open('$HOME/.ccfzf/windows/<host>.json'));print(time.time()-o['generated'],len(o['windows']))"
```

Первое число меньше тридцати — трекер публикует. Второе — сколько окон он
видит.
