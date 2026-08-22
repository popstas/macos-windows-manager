#!/bin/sh
# Сверяет тег с версией, а версия записана в двух местах сразу: cargo читает
# `Cargo.toml`, а бандл и каска — `tauri.conf.json`. Разойтись они могут молча.
#
# Проверка стоит в релизном workflow не для порядка. У соседнего
# windows11-manager образец тега разошёлся с настроенным, и три релиза вышли
# без единого артефакта — не сработавший workflow ничем себя не проявляет,
# релиз при этом выглядит обычным.
set -eu

tag="${1:?usage: check-version.sh vX.Y.Z}"
want="${tag#v}"

cargo_version=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' Cargo.toml)
tauri_version=$(sed -n 's/^  "version": "\(.*\)",$/\1/p' src-tauri/tauri.conf.json)

status=0
for pair in "Cargo.toml=$cargo_version" "src-tauri/tauri.conf.json=$tauri_version"; do
  file=${pair%%=*}
  have=${pair#*=}
  if [ -z "$have" ]; then
    echo "$file: версия не вычитывается — сломан разбор, а не версия" >&2
    status=1
  elif [ "$have" != "$want" ]; then
    echo "$file: версия $have, а тег говорит $want" >&2
    status=1
  fi
done

exit $status
