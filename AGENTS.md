# AGENTS — Warp i18n Project Rules

> Локальные правила для AI-агентов. Не коммитить — в `.gitignore`.
> Общие правила грузятся из WikiDB/MCP Config: `agent_startup` → `gates` → `base_rules` → role rules.

## Project

Warp i18n — интернационализация Warp (терминал от warpdotdev).
YAML-формат (совместим с ZacharyZcR), русская локаль.

## Where Code Runs

- **Development:** `/nob/dmitry/git/warp` (локальная машина)
- **Production:** не настроен

## Current Agent Role

| Роль | Назначена |
|------|-----------|
| **Coordinator (root)** | 2026-05-14 |

## Ветка

- **Рабочая ветка:** `i18n`
- **Upstream:** `warpdotdev/warp` (HTTPS, read-only)
- **Форк:** `ErshovDmitry/warp-i18n` (push через `push-https`)
- **PR:** [#11382](https://github.com/warpdotdev/warp/pull/11382)

### 🔴 Merge Policy

- **НЕ мёржим master в i18n.** Никогда.
- **НЕ мёржим i18n в master.** Это делает upstream.
- Совместимость с master — ручная адаптация кода, без merge-коммитов.

## Скилы

| Скил | Назначение |
|------|-----------|
| `warp-i18n-upstream-sync` | Синхронизация с upstream |
| `warp-i18n-wrap-strings` | Поиск и оборачивание новых строк |
| `warp-i18n-pr-monitor` | Мониторинг PR #11382 |
| `warp-i18n-autonomous-agent` | Оркестратор полного цикла |

## Ключевые страницы wiki (схема `project_warp`)

| Страница | ID |
|----------|----|
| Agent Context — 2026-06-03 | `24eec969-32d3-4a71-bc2b-222d8a469c81` |
| Warp i18n — Final State | `c8542373-4b17-4a21-9bde-f39434ef50d1` |
| Decision: Switch to ZacharyZcR YAML | `62fc189c-294e-49f9-904a-889220c939f7` |
| Trace: agent — полный цикл — 2026-06-03 | `33e6d917-3bce-4cec-aed2-01dd78cdb4a0` |
| Agent Memory | `8595c8b4-ca9f-4834-8517-853a5df6d0e4` |

## Совместимость с ZacharyZcR

- Формат YAML (`resources/bundled/locales/{en,ru}.yml`)
- API: `crate::menu_label()`, `i18n::lookup()`, `TranslationLookup`
- При расхождениях — править у нас

## Соглашения по коду

- **i18n паттерн:** `crate::menu_label("key.path", "English fallback")`
- **Локали:** `resources/bundled/locales/{en,ru}.yml`
- **Проверка:** `cargo check -p warp` (0 warnings) + `cargo test -p i18n` (11/11 pass)

## Уведомления

- **Gotify:** `https://gotify.derhp.ru`, token `A7BzFZmfiu3nqDe`
- 🇷🇺 Русский — Gotify, 🇬🇧 English — GitHub

## Сборка

```bash
cargo check -p warp
cargo test -p i18n

# AppImage
cargo build -p warp --profile release-lto --bin warp-oss --features "release_bundle,gui"
NO_STRIP=1 script/bundle --channel oss --packages appimage --skip-build
# → target/release-lto/bundle/linux/WarpOss-x86_64.AppImage
```

## Golden Rule

Если этого нет в wiki — этого не было.
