//! Small shared config helpers used by anything that talks to the Go
//! backend (`commands::auth`'s login call, `sync`'s push/pull worker) or
//! the fiscal provider (`fiscal_client`).

pub const DEFAULT_BACKEND_URL: &str = "http://localhost:8080";

/// Backend base URL, overridable via the `PDV_BACKEND_URL` env var so dev
/// and future multi-store deployments don't require a rebuild.
pub fn backend_url() -> String {
    std::env::var("PDV_BACKEND_URL").unwrap_or_else(|_| DEFAULT_BACKEND_URL.to_string())
}

const FOCUS_NFE_HOMOLOGACAO_URL: &str = "https://homologacao.focusnfe.com.br";
const FOCUS_NFE_PRODUCAO_URL: &str = "https://api.focusnfe.com.br";

/// Focus NFe API token (plan section 5 — "Provider API (Focus NFe)"),
/// used as the Basic Auth username with an empty password, per the
/// provider's documented convention. `None` means the pilot store hasn't
/// been onboarded with the provider yet — `fiscal_client` treats this as
/// "stay in the local queue" rather than an error, since a sale must
/// never be blocked on fiscal emission (plan section 1.2).
pub fn fiscal_provider_token() -> Option<String> {
    std::env::var("FISCAL_PROVIDER_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
}

/// `FISCAL_ENV=producao` selects the live SEFAZ-authorizing endpoint;
/// anything else (including unset, the safe default) selects Focus NFe's
/// homologation/sandbox environment, per plan section 9's rule to never
/// test against production with a real CNPJ.
pub fn fiscal_provider_base_url() -> String {
    match std::env::var("FISCAL_ENV").as_deref() {
        Ok("producao") => FOCUS_NFE_PRODUCAO_URL.to_string(),
        _ => FOCUS_NFE_HOMOLOGACAO_URL.to_string(),
    }
}

/// The issuing store's CNPJ. The plan's schema (section 3) has no
/// store/company table — Focus NFe's own dashboard already ties one
/// account/token to one CNPJ and holds the rest of the issuer's
/// registration data (razão social, address, digital certificate; see
/// plan section 5.3), so this is the only issuer field the payload
/// itself needs to carry.
pub fn fiscal_issuer_cnpj() -> String {
    std::env::var("FISCAL_ISSUER_CNPJ").unwrap_or_default()
}
