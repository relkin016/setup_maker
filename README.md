# Setup Maker

Кроссплатформенный (Linux + Windows) конструктор установщиков Windows.
Редактор проектов устроен как в **Smart Install Maker**: разделы слева, простые формы справа,
обычные флажки («Ярлык на рабочем столе», «Запустить программу после установки») вместо скриптов.
Сам установщик выглядит как **Inno Setup**: классический мастер 497×360 с теми же страницами и текстами.

## Состав

| Файл | Что это |
|---|---|
| `setupmaker` / `setupmaker.exe` | редактор (GUI) + консольный сборщик |
| `stubs/smk-stub-x64.exe`, `smk-stub-x86.exe` | рантайм установщика (уже встроен в `setupmaker`) |
| `examples/*.smp` | примеры проектов |

## Быстрый старт

```sh
setupmaker                                  # графический редактор
setupmaker new "My App" --exe MyApp.exe --files bin -o myapp.smp
setupmaker build myapp.smp --verify         # собрать setup.exe и проверить CRC
setupmaker import "GTA ZA/setup.exe" -o gta                     # разбор готового Inno setup.exe в проект
setupmaker pack gamedata -o data1.smpk --method lzma2 --dict 256 # внешний архив
setupmaker verify compiled/setup.exe
```

В редакторе: **F9** — собрать, **Ctrl+S** — сохранить.

## Импорт готового установщика Inno Setup (`setup.exe`)

`setupmaker import setup.exe [-o папка]` (или кнопка «Импорт setup.exe» в редакторе) разбирает
скомпилированный установщик **Inno Setup 5.2.5 – 6.3** (ANSI и Unicode) и создаёт папку проекта
(по умолчанию `setup_project` рядом с setup.exe):

| Что | Куда |
|---|---|
| сведения о программе, папки, uninstall | `[app]`, `[setup]`, `[uninstall]` |
| встроенные файлы (LZMA/LZMA2/zlib, фильтр x86, проверка SHA-1) | `files/<корень>_<задачи>/…` + записи `[[files]]` с условиями |
| задачи, компоненты, типы, папки, ярлыки, реестр, `[Run]`, удаление | соответствующие списки |
| картинки мастера, иконка setup.exe, лицензия / инфо (RTF) | `wizard.bmp`, `wizard_small.bmp`, `setup.ico`, `*.rtf` |
| языки и `[CustomMessages]` | `[interface]`, `[messages]` |
| файлы `{tmp}` (кроме ISDone.dll, records.inf, скинов) | `tools/` → «Инструменты» |

Секция `[Code]` (Pascal) не переносится, её строки сохраняются в `code_strings.txt`. Из них
автоматически берутся: вызовы ISDone (`{src}\*.arc` → раздел «Архивы»), шрифт мастера (`Candara`…),
заголовки приветствия/завершения. Архивы, лежащие рядом с setup.exe, тоже добавляются.
Для `.arc` положите `unarc.dll` из FreeArc в `tools/` и добавьте в «Инструменты» —
ISDone.dll из оригинала не используется (прогресс, время, отмена и откат у нас встроены).

## Редактор

- **Информация** — название, версия, главный exe и быстрые флажки: ярлык в «Пуске», на рабочем столе
  (всегда / спросить пользователя), ярлык удаления, «Запустить программу» в конце установки.
- Папки выбираются из списка («Папка программы», «Program Files», «AppData», «Рабочий стол»…),
  условия — галочками компонентов и флажков.
- У ярлыка есть «Создавать: всегда / спросить (флажок отмечен / снят)» — флажок в установщике
  создаётся автоматически.
- Редкие настройки спрятаны в «Ещё».

## Проект (`.smp`, TOML)

В проекте есть разделы: `[app]` (AppName, AppVersion, AppId…), `[setup]` (страницы мастера,
папки, лицензия, пароль, место на диске), `[interface]` (картинки, иконка, шрифт мастера `font`/`font_size`, языки ru/en/uk), `[requirements]`,
`[compression]`, `[output]` (в т. ч. разбиение на тома), `[uninstall]`, а также списки
`types`, `components`, `tasks`, `files`, `archives`, `tools`, `dirs`, `shortcuts`, `registry`,
`run`, `uninstall_run`, `install_delete`, `uninstall_delete`, `messages`.

Строки поддерживают константы Inno: `{app} {pf} {pf32} {pf64} {group} {src} {tmp} {sys} {win}
{commondesktop} {autodesktop} {userappdata} {localappdata} {commonappdata} {userdocs} {quicklaunch}
{uninstallexe} {%ENV|def} {reg:HKLM\Key,Value|def} {param:Name|def} {cm:LaunchProgram,App}`
и переменные сборки `{#Name}` (раздел `[defines]`).

В редакторе константы выбираются из списков, вручную их писать не нужно.
Условия (`components`, `tasks`, `check`) — выражения как в Inno: `voice\rus or voice\eng`,
`not Is64BitInstallMode`, `FileExists('{app}\x.exe')`, `IsWin64`, `IsAdmin`, `RegKeyExists('HKLM\…')`.

## Внешние архивы — просто по именам

```toml
archives = ["data1.arc", "data2.smpk", "voice.7z -> {app}\\sound"]
```
или подробно `[[archives]]` с `dest`, `password`, `components`, `disk_prompt` («вставьте диск 2»),
`pack_from` (папка, которую сборщик сам упакует в `.smpk` рядом с setup.exe).
Архивы ищутся рядом с setup.exe, поддерживаются маски (`data*.arc`).

Форматы: `.smpk` (свой: LZMA2 / Zstd / Brotli, solid, дедупликация, CRC), `.7z` (в т. ч. с паролем),
`.zip`, `.tar`, `.tar.gz/.xz/.zst/.br`, **FreeArc `.arc`** — через ваш `unarc.dll` (как ISDone)
или `unarc.exe`/`arc.exe`, добавленные в раздел «Инструменты». `unarc.dll` 32-битный, поэтому
при `arch = "auto"` сборщик сам выбирает 32-битный установщик.

## Сжатие

| Метод | Когда |
|---|---|
| `lzma2` (по умолчанию: ур. 9 extreme, словарь 64 МБ, BCJ x86 для exe/dll) | максимальное сжатие, ≈ FreeArc `-m4/-m5` без precomp/srep |
| `zstd` (1–22, long-distance matching) | чуть хуже сжатие, распаковка в 5–10 раз быстрее |
| `brotli` (0–11) | тексты/веб-контент |
| `none` | уже сжатые данные |

Сжатие многопоточное (число потоков подбирается по ОЗУ). Одинаковые файлы хранятся один раз.
Распаковке нужно ОЗУ примерно размером со словарь.

## Установщик

- Страницы: язык → приветствие → лицензия → пароль → информация → папка → компоненты →
  меню «Пуск» → задачи → готово → установка (прогресс, прошло/осталось, прогресс архива) →
  информация → завершение (флажки запуска, перезагрузка, время установки).
- Откат всех изменений при ошибке/отмене; Retry/Ignore/Abort при занятых файлах.
- Деинсталлятор `unins000.exe` + журнал, запись в «Программы и компоненты», повторная установка поверх.
- Ключи: `/SILENT /VERYSILENT /SUPPRESSMSGBOXES /DIR= /GROUP= /NOICONS /LANG= /TASKS= /COMPONENTS= /TYPE=`.
- Автоподъём прав через UAC, проверка версии Windows и объёма ОЗУ.

## Ограничения

- Собранный установщик работает на **Windows 7 SP1, 8, 8.1, 10 и 11** (x64 и x86).
  Минимальная версия по умолчанию — Windows 7 SP1 («Требования → Минимальная Windows»;
  в проектах, сохранённых до 0.5.0, там может стоять Windows 10 — поменяйте вручную).
- Сам редактор есть и для **Windows 7 SP1** (`setupmaker-*-windows7-x64.zip` / `-x86.zip`):
  импорт/разбор `setup.exe`, сборка и упаковка работают. Графическому
  редактору нужен драйвер с OpenGL 2.0+; без него (ВМ, «стандартный VGA») используйте консоль:
  `setupmaker import setup.exe -o папка`.
- Код `[Code]` на Pascal не исполняется. Импортёр переносит из него активные вызовы ISDone
  (`ISArcExtract`, `IS7ZipExtract`, `ShowChangeDiskWindow`) в список архивов.
- Скины ISSkin (`.cjstyles`) и RAR не поддерживаются; precomp/srep — через «Инструменты» + `[Run]`.

## Сборка из исходников

Нужны Rust и mingw-w64. См. `release.sh` (собирает стабы x64/x86, редактор для Linux и Windows).
