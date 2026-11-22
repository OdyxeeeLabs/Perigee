# Internationalisation — issue #700

## What was already here

`next-intl` was wired (`lib/i18n.ts` + `NextIntlClientProvider`) but only
`messages/en.json` existed, so the app was structurally i18n-ready and
practically monolingual. Everything below closes the gap between those two
states.

## Locale registry

`lib/locale/locales.ts` is the single source of truth:

| | |
|---|---|
| Supported | `en`, `es`, `zh`, `ar`, `hi`, `pt`, `fr`, `de` |
| Default | `en` |
| Direction | `ar` is `rtl`, everything else is `ltr` |
| Persistence key | `perigee:locale` (localStorage) |
| BCP 47 tags | `en-GB`, `es-ES`, `zh-Hans`, `ar`, `hi-IN`, `pt-BR`, `fr-FR`, `de-DE` |

Nothing else hardcodes a locale. `normalizeLocale` accepts regional variants
(`pt-PT`, `pt_BR`) and maps them onto the catalogue we ship, and falls back to
`en` for anything unrecognised rather than rendering an empty UI.

Storage access is wrapped in try/catch throughout: Safari private mode and
blocked third-party storage throw on `localStorage`, and a language picker
that crashes on a cookie-acceptance prompt is not shippable.

## Formatting

`lib/locale/format.ts` wraps the `Intl` constructors that were previously
implicit:

| Helper | Purpose |
|---|---|
| `formatNumber` / `formatCompactNumber` | Grouping and decimal marks per locale (`1.234.567,89` in `de`) |
| `formatPercent` | Fixed precision so a stat tile cannot show `12.3456%` |
| `formatCurrency` | Currency is a parameter, not a locale default — the app holds XLM and USDC |
| `formatDate` / `formatDateTime` | Locale calendars, with an em dash for invalid input |
| `formatRelativeTime` | "3 minutes ago", degrading to an absolute date |
| `pluralCategory` / `pluralize` | CLDR plural rules |

Two deliberate choices worth recording:

- **`other` is the only plural fallback.** Falling back to the singular form
  renders "1 vault" for three vaults, which is worse than showing the raw
  count.
- **Hours are formatted with the `hour` unit.** This was a real bug during
  development: a two-hour-old timestamp was reported in days.

## Pluralisation

`pluralize` takes either a flat template (`"{count} vaults"`) or a map keyed
by CLDR category (`{one, other, zero, two, few, many}`). Arabic needs six
categories, Chinese needs one, French and Portuguese treat zero as singular —
so a single "s"-suffix convention does not survive contact with the locale
list, and the catalogue has to be keyed by category.

## Language switcher

`components/LanguageSwitcher.tsx` lists locales by **endonym** (`Español`,
`中文`, `العربية`). A reader who cannot read the current UI language still has
to find their own language in the list; an English-labelled picker defeats the
point of having one.

## Persistence and hydration

`components/LocaleSync.tsx` owns the document-level `lang` and `dir`
attributes and cross-tab synchronisation via the `storage` event.

Both it and `HighContrastToggle` apply their preference in an effect rather
than during the first client render. This is deliberate: the server has no
access to `localStorage`, so rendering the stored value immediately would
produce a hydration mismatch on every page for every user who chose a
non-default language. The cost is one frame of the default locale; the
alternative — a blocking inline script before paint — penalises all users to
fix a warning. Documented rather than hidden because it is a real tradeoff,
not an oversight.

## Message catalogues

`messages/{es,zh,ar,hi,pt,fr,de}.json` seed the `app`, `nav`, `palette`,
`a11y`, `common` and `connectButton` namespaces. The other ~290 keys still
resolve through `en.json`, which is the intended behaviour: a partially
translated UI in the reader's language beats a complete UI in English.

Translating the remaining keys is mechanical once the strings are
feature-complete, and deliberately left out of a code change — machine
translations of financial terminology ("rebalancing", "transaction fee",
"maturity") are a correctness problem, not a typing problem.

## Adding a locale

1. Add the tag to `SUPPORTED_LOCALES` and the row to `LOCALE_NAMES`,
   `LOCALE_TAGS`, `PLURAL_CATEGORIES` in `lib/locale/locales.ts`.
2. Add `messages/<tag>.json`.
3. Register the locale in `i18n.ts`'s routing if it is exposed as a route.

The test file (`lib/locale/locales.test.cjs`) asserts the exact locale set, so
a missing step fails loudly rather than shipping an unreachable language.
