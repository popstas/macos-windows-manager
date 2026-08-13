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
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/pro.popstas.macos-windows-manager.plist
```

`launchctl load` — прежняя форма, она ещё работает, но всё остальное здесь
говорит с launchd современными командами (`bootstrap`, `bootout`,
`kickstart`), и мешать два набора незачем.

Разрешение Accessibility выдаётся **этому бинарю**, а не терминалу, из которого
его запустили.

⚠️ **Пересборку разрешение не переживает.** Проверено на живой машине: после
`cargo build --release` трекер молча перестаёт публиковать — процесс жив,
значок на месте, а файл окон стареет. Путь тут ни при чём: бинарь подписан
линковщиком «на ходу» (`codesign -dv` показывает `adhoc, linker-signed`), хеш
подписи меняется с каждой сборкой, а macOS помнит именно его.

Значит после каждой выкатки со сборкой разрешение надо выдавать заново — либо
подписать бинарь постоянным ключом. Второе делается один раз: в Keychain Access
завести самоподписанный сертификат типа Code Signing, а в `deploy-mac.sh` после
сборки добавить

```
codesign --force --sign "<имя сертификата>" \
  --identifier pro.popstas.macos-windows-manager \
  ./target/release/macos-windows-manager
```

Тогда TCC запоминает требование к подписи, а оно от сборки к сборке не меняется.

После переезда каталога разрешение придётся выдать заново в любом случае.
