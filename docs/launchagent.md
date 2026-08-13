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
  <key>StandardErrorPath</key><string>/tmp/mwm.err.log</string>
  <key>StandardOutPath</key><string>/tmp/mwm.out.log</string>
</dict>
</plist>
```

Без этих двух ключей stderr трекера уходит в никуда — ни строки ни в
`Console.app`, ни в `log show`. Отказ соединения с брокером выглядит тогда
просто отсутствием причины: процесс жив, файл окон публикуется, а почему
`focus` не поднимается — неизвестно неоткуда, кроме этого файла.

```
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/pro.popstas.macos-windows-manager.plist
```

`launchctl load` — прежняя форма, она ещё работает, но всё остальное здесь
говорит с launchd современными командами (`bootstrap`, `bootout`,
`kickstart`), и мешать два набора незачем.

Разрешение Accessibility выдаётся **этому бинарю**, а не терминалу, из которого
его запустили.

Если в конфиге настроен `mqtt:`, нужно ещё одно, отдельное разрешение —
Local Network (System Settings → Privacy & Security → Local Network). Без
него сокет к брокеру не проходит вовсе, а отказ выглядит не отказом в
правах, а ошибкой маршрутизации: в stderr — `mqtt connection lost: I/O: No
route to host (os error 65)`, при живом брокере и верных учётных данных.
Держится это разрешение так же, как Accessibility, за подпись бинаря, и
слетает с неё точно так же на следующей же сборке.

⚠️ **Неподписанный бинарь теряет разрешение на каждой сборке.** Проверено на
живой машине: после `cargo build --release` трекер молча перестаёт публиковать —
процесс жив, значок на месте, а файл окон стареет. Путь тут ни при чём: бинарь
подписан линковщиком «на ходу» (`codesign -dv` показывает `adhoc,
linker-signed`), и требование к подписи выходит по хешу содержимого.

Разовая настройка, после которой это прекращается:

1. Keychain Access → Certificate Assistant → Create a Certificate…, тип
   **Code Signing**, самоподписанный. Имя запомнить.
2. **В терминале на самом маке** (не по ssh) подписать один раз:

   ```
   codesign --force --sign "<имя сертификата>" \
     --identifier pro.popstas.macos-windows-manager \
     ~/projects/js/macos-windows-manager/target/release/macos-windows-manager
   ```

   На вопрос о доступе к ключу ответить **Always Allow**. Иначе связка будет
   спрашивать при каждой подписи, а спросить у выкатки не у кого.
3. Выдать Accessibility этому — уже подписанному — бинарю.
4. Если имя сертификата не `popstas`, передавать его выкатке:
   `MWM_SIGN_ID=<имя> ./data/scripts/deploy-mac.sh`.

Дальше `deploy-mac.sh` подписывает сам после каждой сборки, и требование
остаётся прежним: `identifier "…" and certificate leaf = H"…"`.

После переезда каталога разрешение придётся выдать заново в любом случае.
