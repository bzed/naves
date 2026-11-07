use crate::DesignTokens;

/// Returned by [`crate::try_set_design_tokens`] when the design tokens have already been initialized.
#[derive(Clone, Copy, Debug)]
pub struct DesignTokensAlreadyInitializedError;

impl std::fmt::Display for DesignTokensAlreadyInitializedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Design tokens have already been initialized")
    }
}

impl std::error::Error for DesignTokensAlreadyInitializedError {}

struct DesignTokensPerTheme {
    dark: DesignTokens,
    light: DesignTokens,
}

impl DesignTokensPerTheme {
    fn load() -> anyhow::Result<Self> {
        Ok(Self {
            dark: DesignTokens::load(egui::Theme::Dark, include_str!("../data/dark_theme.ron"))?,
            light: DesignTokens::load(egui::Theme::Light, include_str!("../data/light_theme.ron"))?,
        })
    }
}

mod design_token_access {
    use std::sync::OnceLock;

    use crate::DesignTokens;

    use super::{DesignTokensAlreadyInitializedError, DesignTokensPerTheme};

    static DESIGN_TOKENS: OnceLock<DesignTokensPerTheme> = OnceLock::new();

    pub fn design_tokens_per_theme() -> &'static DesignTokensPerTheme {
        DESIGN_TOKENS
            .get_or_init(|| DesignTokensPerTheme::load().expect("Failed to load design tokens"))
    }

    pub fn try_set_design_tokens(
        dark: DesignTokens,
        light: DesignTokens,
    ) -> Result<(), DesignTokensAlreadyInitializedError> {
        if DESIGN_TOKENS
            .set(DesignTokensPerTheme { dark, light })
            .is_err()
        {
            return Err(DesignTokensAlreadyInitializedError);
        }
        Ok(())
    }
}

pub fn design_tokens_of(theme: egui::Theme) -> &'static DesignTokens {
    match theme {
        egui::Theme::Dark => &design_token_access::design_tokens_per_theme().dark,
        egui::Theme::Light => &design_token_access::design_tokens_per_theme().light,
    }
}

pub(crate) use design_token_access::try_set_design_tokens;
