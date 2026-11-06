# Third-party notices

WoW Chat Translator is MIT licensed (see `LICENSE`). It includes or builds on the
following third-party work.

## Graphics and fonts

- **Twemoji** — emoji images rendered through the `egui-twemoji` crate.
  Twemoji graphics by Twitter, Inc and other contributors, licensed under CC-BY 4.0
  (https://creativecommons.org/licenses/by/4.0/). Source: https://github.com/twitter/twemoji
  (maintained fork: https://github.com/jdecked/twemoji).
- **egui default fonts** (`epaint_default_fonts`): Hack (MIT), Noto Emoji (OFL-1.1),
  Ubuntu Light (Ubuntu Font Licence 1.0), emoji-icon-font (OFL-1.1 / MIT).
  See https://github.com/emilk/egui/tree/main/crates/epaint_default_fonts.

## Design references

No code from these projects is included; their documented designs informed ours.

- **Parley** (MIT) — pixel strip read from the game window plus clipboard replies:
  https://github.com/Duhlude/parley (protocol: https://github.com/Duhlude/parley/blob/main/docs/HOW_IT_WORKS.md)
- **LibSerpix** (MIT) — pixel serialization: https://github.com/alex-berliner/LibSerpix

## Rust crates with terms other than MIT / Apache-2.0

All other crates in the release build are MIT and/or Apache-2.0. Full license texts
are in each crate's source (`cargo tree` lists the exact versions).

| Crate(s) | License |
| --- | --- |
| `emojis` (emoji data), `unicode-ident` | (MIT OR Apache-2.0) AND Unicode-3.0 |
| `icu_*`, `litemap`, `potential_utf`, `tinystr`, `writeable`, `yoke`, `yoke-derive`, `zerofrom`, `zerofrom-derive`, `zerotrie`, `zerovec`, `zerovec-derive` (ICU4X) | Unicode-3.0 |
| `ring` | Apache-2.0 AND ISC |
| `rustls-webpki`, `untrusted`, `libloading` | ISC |
| `webpki-roots` (Mozilla root certificates) | CDLA-Permissive-2.0 |
| `epaint_default_fonts` | (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 |
| `clipboard-win`, `error-code` | BSL-1.0 |
| `subtle`, `tiny-skia`, `tiny-skia-path` | BSD-3-Clause |
| `moxcms`, `pxfm` | BSD-3-Clause OR Apache-2.0 |
| `arrayref` | BSD-2-Clause |
| `zlib-rs` | Zlib |

## Installer

The Windows installer is built with Inno Setup (https://jrsoftware.org/isinfo.php),
Copyright (C) 1997-2026 Jordan Russell and Martijn Laan, under the Inno Setup License.
