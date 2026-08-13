# Автозапуск

`~/Library/LaunchAgents/pro.popstas.macos-windows-manager.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>pro.popstas.macos-windows-manager</string>
  <key>ProgramArguments</key>
  <array><string>/Users/USER/projects/js/macos-windows-manager/target/release/macos-windows-manager</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
</dict>
</plist>
```

```
launchctl load -w ~/Library/LaunchAgents/pro.popstas.macos-windows-manager.plist
```

Разрешение Accessibility выдаётся **этому бинарю**, а не терминалу, из которого
его запустили. После пересборки путь тот же, и разрешение переживает выкатку;
после переезда каталога — не переживает, и его придётся выдать заново.

⚠️ **TODO:** На реальном маке подтвердить — пересобрать и посмотреть, просит ли
Accessibility заново. На новых macOS разрешение может быть привязано к подписи
бинаря, а не пути.
