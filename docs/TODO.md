# next

- [x] Ignore minimized windows while tail
  - [x] add `minimized` flag to window model
  - [x] add filter "show minimized", default true
  - [x] show "N windows minimized" in the tracked message in menu
  - [x] add menu checkbox: ignore minimized windows

- [ ] Автозапуск галочкой в меню трея
  - каска ставит только `.app`, а трекеру место в автозапуске
  - LaunchAgent из каски не ставится намеренно: для cask это нестандартно и
    поспорило бы с выкаткой на своей машине
