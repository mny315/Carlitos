#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
if [[ $# -gt 0 ]]; then
  echo 'Для меню запустите ./build.sh без дополнительных аргументов.' >&2
  exit 2
fi
main_items=(
  'run|Запустить Carlitos'
  'linux|Собрать Linux — переносимый архив'
  'windows|Собрать Windows — установщик'
  'android release|Собрать Android — подписанный релизный APK'
  'all|Собрать релизы для всех платформ'
  'check all|Проверить форматирование и код Linux/Android'
  '@tests|Тесты — выбрать группу'
  '@android|Android — отладка и ключ подписи'
  '@more|Другие действия'
)
test_items=(
  'test unit|Rust — модульные и интеграционные тесты'
  'test tooling|Скрипт сборки — группы, ошибки и перезапись логов'
  'test media|Аудиоформаты, импорт и воспроизведение'
  'test sonic|Sonic — обработка звука и санитайзеры'
  'test ui|Интерфейс — самопроверка'
  'test desktop|Linux — окно, MPRIS и трей'
  'test visual|Linux — снимки интерфейса'
  'test performance|Linux — производительность'
  'test portable|Linux — переносимая сборка'
  'test windows|Windows — установка, обновление и удаление под Wine'
  '@android_tests|Android — выбрать группу'
)
android_test_items=(
  'test android session|Медиасессия (нужна выбранная аудиокнига)'
  'test android sources|Источники (нужна папка Carlitos-sources)'
  'test android import|Импорт и форматы'
  'test android playback|Воспроизведение и жизненный цикл'
  'test android ui|Интерфейс и жесты'
)
android_items=(
  'android debug|Собрать отладочный APK'
  'android install|Собрать и установить отладочный APK'
  'android run|Собрать, установить и запустить'
  'android devices|Показать ADB-устройства'
  'android keygen|Создать ключ релизной подписи (один раз)'
)
extra_items=(
  'demo|Запустить демонстрацию'
  'build|Собрать нативный release-бинарник'
  'package|Собрать пакет Nix'
  'install|Установить Linux-версию'
  'format|Форматировать Rust'
  'sources|Собрать исходники переносимой сборки'
  'fetch|Скачать зависимости'
  'versions|Показать версии инструментов'
)
screen=main
while true; do
  case "$screen" in
    main) items=("${main_items[@]}"); title=Carlitos; parent=exit ;;
    tests) items=("${test_items[@]}"); title='Carlitos — группы тестов'; parent=main ;;
    android_tests) items=("${android_test_items[@]}"); title='Carlitos — тесты Android'; parent=tests ;;
    android) items=("${android_items[@]}"); title='Carlitos — Android'; parent=main ;;
    more) items=("${extra_items[@]}"); title='Carlitos — другие действия'; parent=main ;;
  esac
  printf '\n%s\n\n' "$title"
  for index in "${!items[@]}"; do
    printf '  %2d) %s\n' "$((index + 1))" "${items[index]#*|}"
  done
  printf '   0) %s\n\nВыберите пункт: ' "$([[ "$screen" == main ]] && echo Выйти || echo Назад)"
  if ! read -r choice; then printf '\n'; exit 0; fi
  if [[ "$choice" == 0 ]]; then
    if [[ "$parent" == exit ]]; then exit 0; fi
    screen="$parent"
    continue
  fi
  selected=
  for index in "${!items[@]}"; do
    if [[ "$choice" == "$((index + 1))" ]]; then selected="${items[index]%%|*}"; break; fi
  done
  if [[ -z "$selected" ]]; then printf '\nНет такого пункта. Введите номер из меню.\n'; continue; fi
  if [[ "$selected" == @* ]]; then screen="${selected#@}"; continue; fi
  read -r -a command <<< "$selected"
  if ./build.sh "${command[@]}"; then
    printf '\nГотово.\n'
  else
    status=$?
    printf '\nОшибка (код %s). Подробности выше и в target/logs/.\n' "$status" >&2
  fi
done
