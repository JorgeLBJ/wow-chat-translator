//! Tries the primary translator and, if it fails, the secondary one, so a down LLM
//! server never leaves chat untranslated. The result says which provider answered.
//! Port of `fallback.js`.

use crate::core::pipeline::{Translation, Translator};

pub struct FallbackTranslator {
    primary_name: String,
    primary: Box<dyn Translator + Send + Sync>,
    secondary_name: String,
    secondary: Box<dyn Translator + Send + Sync>,
}

impl FallbackTranslator {
    pub fn new(
        primary_name: &str,
        primary: Box<dyn Translator + Send + Sync>,
        secondary_name: &str,
        secondary: Box<dyn Translator + Send + Sync>,
    ) -> Self {
        Self { primary_name: primary_name.into(), primary, secondary_name: secondary_name.into(), secondary }
    }
}

impl Translator for FallbackTranslator {
    fn translate(&self, text: &str, target: &str) -> Result<Translation, String> {
        match self.primary.translate(text, target) {
            Ok(res) => Ok(Translation { provider: Some(self.primary_name.clone()), ..res }),
            Err(reason) => {
                let res = self.secondary.translate(text, target)?;
                Ok(Translation { provider: Some(self.secondary_name.clone()), fallback_reason: Some(reason), ..res })
            }
        }
    }
}

/// Tags every result with a provider name (Google-only mode).
pub struct Named {
    name: String,
    inner: Box<dyn Translator + Send + Sync>,
}

impl Named {
    pub fn new(name: &str, inner: Box<dyn Translator + Send + Sync>) -> Self {
        Self { name: name.into(), inner }
    }
}

impl Translator for Named {
    fn translate(&self, text: &str, target: &str) -> Result<Translation, String> {
        let res = self.inner.translate(text, target)?;
        Ok(Translation { provider: Some(self.name.clone()), ..res })
    }
}
