# Проверки

Запуск из корня проекта:

В меню `./build.sh` выберите «Тесты», затем нужную группу. Группы запускаются
по отдельности; Android открывает своё подменю.

| Команда | Что проверяет |
| --- | --- |
| `./build.sh check all` | Форматирование, Clippy Linux и Android |
| `./build.sh test unit` | Rust: модульные и интеграционные тесты |
| `./build.sh test tooling` | Группы меню, коды ошибок и перезапись логов |
| `./build.sh test media` | Аудиоформаты, импорт и воспроизведение |
| `./build.sh test sonic` | Sonic и санитайзеры |
| `./build.sh test ui` | Самопроверка интерфейса |
| `./build.sh test desktop` | Linux: окно, MPRIS и трей |
| `./build.sh test visual` | Снимки интерфейса в niri |
| `./build.sh test performance` | Производительность Linux |
| `./build.sh test portable` | Переносимый Linux-архив после сборки |
| `./build.sh test windows` | Установщик под Wine после сборки |
| `./build.sh test android session` | Медиасессия; нужна выбранная аудиокнига |
| `./build.sh test android sources` | SAF-источники; нужна выбранная папка Carlitos-sources |
| `./build.sh test android import` | Импорт Android |
| `./build.sh test android playback` | Android-плеер и жизненный цикл процесса |
| `./build.sh test android ui` | Интерфейс и жесты Android |

Аргументы Cargo можно передать после группы: `./build.sh test unit --lib`.
Вывод идёт в терминал и `target/logs/test-<группа>.log`; для Android имя включает
подгруппу, например `test-android-ui.log`. Файл перезаписывается при новом запуске.

Структура проверок:

- `tests/integration/` — Rust-интеграции; имена Cargo targets сохранены.
- `tests/tooling/` — проверки единого скрипта сборки.
- `tests/audio/` — проверки аудиоформатов и Sonic.
- `tests/linux/` — UI, рабочий стол, производительность и переносимая сборка.
- `tests/windows/` — зависимости EXE и жизненный цикл установщика.
- `tests/android/` — подготовка данных, ADB-проверки и Kotlin-тесты в `device/`: `ui/`, `playback/`, `sources/`.
- `tests/android/fixtures/` — Kotlin-провайдеры тестовых документов и аудио, manifest отдельного APK `Carlitos Fixtures`.
- `tests/fixtures/` и `tests/support/` — общие данные и вспомогательный код.
- `src/**/tests/` — модульные тесты рядом с кодом, которому нужен доступ к приватным деталям.

Android UI/playback тесты используют отдельное приложение `.playbacktest`
и очищают его данные. Провайдеры устанавливаются в `.fixtures` со своим Kotlin
runtime и отдельным UID: это позволяет проверять реальные разрешения Android.
В основной APK они не входят. Отчёты и снимки: `target/tests/`.
Проверка смерти процесса использует debug-only receiver в `.playbacktest`:
он посылает себе SIGKILL даже на устройствах, где SELinux запрещает `run-as kill`.
Перед холодным запуском тест ждёт до 90 секунд, пока пользователь разблокирует экран.
После теста экран телефона гасится автоматически, в том числе при ошибке.
На Windows: `cargo test --locked --target x86_64-pc-windows-msvc`.

Установщик после `./build.sh windows` можно проверить под Wine:
`./build.sh test windows`.
Тест создаёт отдельный Wine-префикс и проверяет запуск, обновление, переносимую
копию и удаление с сохранением библиотеки. Отчёт: `target/tests/windows-installer/`.

После установки UI-тестов отдельно проверить жесты библиотеки можно командой:

```sh
adb shell am instrument -e suite ui -e library-only true -w io.github.mny315.carlitos.test/io.github.mny315.carlitos.DeviceInstrumentation
```

Для кнопок глав, паузы/продолжения и переноса выделения при перемотке или
переходе через границу главы замените `library-only` на `chapters-only`.
Для выделения, композиции и пакетного ввода с клавиатуры используйте `input-only`.

После установки UI/playback тестов длительную проверку пауз, скоростей и
сохранения позиции с выключенным экраном можно запустить отдельно (1–120 минут):

```sh
adb shell am instrument -e soak-minutes 100 -w io.github.mny315.carlitos.test/io.github.mny315.carlitos.DeviceInstrumentation
```

Результаты и замеры памяти сохраняются в `playback-soak.json` во внешнем
каталоге файлов тестового приложения. Для повторного прогона очистите данные
только тестового пакета: `adb shell pm clear io.github.mny315.carlitos.playbacktest`.

Для замера кадров через `dumpsys SurfaceFlinger --latency` есть равномерный
свайп с шагом 8 мс. Сначала откройте нужный экран, затем передайте координаты
в физических пикселях и длительность в мс:

```sh
adb shell am instrument -e timed-swipe 600,1800,600,900,600 -w io.github.mny315.carlitos.test/io.github.mny315.carlitos.DeviceInstrumentation
```

Режим не запускает Activity: жест получает приложение на переднем плане.
Системный индикатор герцовки показывает частоту экрана; фактические интервалы
кадров нужно смотреть на слое приложения в SurfaceFlinger.
