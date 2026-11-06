//! GUI-free core: turns decoded chat messages into overlay items and translation jobs,
//! plus the persisted preferences. Ported from `app/src/core` (pixel-strip path only).

pub mod channels;
pub mod emoji;
pub mod language;
pub mod links;
pub mod pipeline;
pub mod prefs;
