//! Composite capabilities handler.
//!
//! `CapabilitiesHandler` bundles all optional capability sub-handlers into one
//! struct that is passed to `AstridClientHandler`.

use super::elicitation::{ElicitationHandler, UrlElicitationHandler};
use super::roots::RootsHandler;
use super::sampling::SamplingHandler;

/// Composite handler that combines all capability handlers.
pub(crate) struct CapabilitiesHandler {
    /// Sampling handler.
    pub sampling: Option<Box<dyn SamplingHandler>>,
    /// Roots handler.
    pub roots: Option<Box<dyn RootsHandler>>,
    /// Elicitation handler.
    pub elicitation: Option<Box<dyn ElicitationHandler>>,
    /// URL elicitation handler.
    pub url_elicitation: Option<Box<dyn UrlElicitationHandler>>,
}

impl Default for CapabilitiesHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CapabilitiesHandler {
    /// Create an empty capabilities handler.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            sampling: None,
            roots: None,
            elicitation: None,
            url_elicitation: None,
        }
    }

    /// Check if sampling is available.
    #[must_use]
    pub(crate) fn has_sampling(&self) -> bool {
        self.sampling.is_some()
    }

    /// Check if roots is available.
    #[must_use]
    pub(crate) fn has_roots(&self) -> bool {
        self.roots.is_some()
    }

    /// Check if elicitation is available.
    #[must_use]
    pub(crate) fn has_elicitation(&self) -> bool {
        self.elicitation.is_some()
    }

    /// Check if URL elicitation is available.
    #[must_use]
    pub(crate) fn has_url_elicitation(&self) -> bool {
        self.url_elicitation.is_some()
    }
}

impl std::fmt::Debug for CapabilitiesHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapabilitiesHandler")
            .field("sampling", &self.has_sampling())
            .field("roots", &self.has_roots())
            .field("elicitation", &self.has_elicitation())
            .field("url_elicitation", &self.has_url_elicitation())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capabilities_handler_builder() {
        let handler = CapabilitiesHandler::new();
        assert!(!handler.has_sampling());
        assert!(!handler.has_roots());
        assert!(!handler.has_elicitation());
        assert!(!handler.has_url_elicitation());
    }
}
