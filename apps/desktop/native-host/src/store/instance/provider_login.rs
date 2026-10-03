//! Fixed CLI login recipes for the Design 37 provider instances.
//!
//! This module is data-only. The native host owns process creation, raw output
//! custody, visible progress, and reconciliation. This module never opens a
//! browser. Its explicit browser behavior distinguishes the fixed OpenCode
//! printed-URL handoff from CLI-owned and unknown browser flows.

use std::path::Path;

#[path = "provider_login_preparation.rs"]
mod preparation;
pub(crate) use preparation::{prepare_registered_provider_login, LoginPreparation,
    resolve_registered_login_home, PreparedProviderLogin, PreparedStatusObservation, ProviderLoginPreparationError,
    StatusObservation};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LoginProvider {
    Claude,
    OpenCode,
    Grok,
    Antigravity,
    Codex,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EnvironmentValue {
    InstanceHome,
    InstanceHomeChild(&'static [&'static str]),
    PreserveUserValue,
    RemoveInherited,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EnvironmentIntent {
    pub name: &'static str,
    pub value: EnvironmentValue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BrowserBehavior {
    /// The CLI may open the default browser itself. The host must not open the
    /// parsed URL automatically; expose it only as an Owner-controlled fallback.
    CliMayOpenAutomatically,
    /// The pinned CLI prints a complete authorization URL but does not open
    /// it. The host may open that exact URL once on the User-visible path.
    HostOpensPrintedAuthorization,
    /// No exact-version browser handoff behavior is evidenced.
    Unknown,
    /// This recipe cannot safely isolate the account from the Windows keyring.
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompletionBehavior {
    /// Process exit is not login proof. The host must reconcile through a
    /// separately evidenced status observation before settling the operation.
    HostReconciliationRequired,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecipeAvailability {
    /// The command form is evidenced; this does not qualify its runtime flow.
    Supported,
    Unsupported(&'static str),
    OutOfScope,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StatusContract {
    Unsupported,
    /// Official CLI reference documents these values for `auth status`.
    /// Other exit codes remain unknown to the caller.
    DocumentedExitCodes { logged_in: i32, logged_out: i32 },
    /// OpenCode prints local credential inventory only; the host never reads
    /// the listed auth file or treats login exit zero as account evidence.
    OpenCodeCredentialList,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProviderLoginRecipe {
    pub provider: LoginProvider,
    pub pinned_version: &'static str,
    pub executable: &'static str,
    pub argv: &'static [&'static str],
    pub instructions: &'static str,
    pub availability: RecipeAvailability,
    /// Apply the same selectors to the login and `status_argv` child.
    pub environment: &'static [EnvironmentIntent],
    pub browser: BrowserBehavior,
    pub completion: CompletionBehavior,
    pub status_argv: Option<&'static [&'static str]>,
    pub status_contract: StatusContract,
}

const COMMON_HOME: &[EnvironmentIntent] = &[
    EnvironmentIntent { name: "HOME", value: EnvironmentValue::InstanceHome },
    EnvironmentIntent { name: "USERPROFILE", value: EnvironmentValue::InstanceHome },
    EnvironmentIntent { name: "APPDATA", value: EnvironmentValue::PreserveUserValue },
    EnvironmentIntent { name: "LOCALAPPDATA", value: EnvironmentValue::PreserveUserValue },
];

const CLAUDE_ENV: &[EnvironmentIntent] = &[
    EnvironmentIntent { name: "HOME", value: EnvironmentValue::InstanceHome },
    EnvironmentIntent { name: "USERPROFILE", value: EnvironmentValue::InstanceHome },
    EnvironmentIntent { name: "APPDATA", value: EnvironmentValue::PreserveUserValue },
    EnvironmentIntent { name: "LOCALAPPDATA", value: EnvironmentValue::PreserveUserValue },
    EnvironmentIntent { name: "CLAUDE_CONFIG_DIR", value: EnvironmentValue::InstanceHome },
];

const GROK_ENV: &[EnvironmentIntent] = &[
    EnvironmentIntent { name: "HOME", value: EnvironmentValue::InstanceHome },
    EnvironmentIntent { name: "USERPROFILE", value: EnvironmentValue::InstanceHome },
    EnvironmentIntent { name: "APPDATA", value: EnvironmentValue::PreserveUserValue },
    EnvironmentIntent { name: "LOCALAPPDATA", value: EnvironmentValue::PreserveUserValue },
    EnvironmentIntent { name: "GROK_HOME", value: EnvironmentValue::InstanceHome },
];

const OPENCODE_ENV: &[EnvironmentIntent] = &[
    EnvironmentIntent { name: "HOME", value: EnvironmentValue::InstanceHome },
    EnvironmentIntent { name: "USERPROFILE", value: EnvironmentValue::InstanceHome },
    EnvironmentIntent { name: "APPDATA", value: EnvironmentValue::PreserveUserValue },
    EnvironmentIntent { name: "LOCALAPPDATA", value: EnvironmentValue::PreserveUserValue },
    EnvironmentIntent { name: "XDG_CONFIG_HOME", value: EnvironmentValue::InstanceHomeChild(&[".config"]) },
    EnvironmentIntent { name: "XDG_DATA_HOME", value: EnvironmentValue::InstanceHomeChild(&[".local", "share"]) },
    EnvironmentIntent { name: "XDG_CACHE_HOME", value: EnvironmentValue::InstanceHomeChild(&[".cache"]) },
    EnvironmentIntent { name: "XDG_STATE_HOME", value: EnvironmentValue::InstanceHomeChild(&[".local", "state"]) },
    EnvironmentIntent { name: "OPENCODE_CONFIG_DIR", value: EnvironmentValue::InstanceHomeChild(&[".opencode"]) },
    EnvironmentIntent { name: "OPENCODE_CONFIG", value: EnvironmentValue::InstanceHomeChild(&[".opencode", "opencode.json"]) },
];

const CLAUDE: ProviderLoginRecipe = ProviderLoginRecipe {
    provider: LoginProvider::Claude,
    pinned_version: "2.1.196",
    executable: "claude",
    argv: &["auth", "login"],
    instructions: "Run the CLI-owned authentication command. The CLI may open the browser itself; the host must not open an output URL automatically.",
    availability: RecipeAvailability::Supported,
    environment: CLAUDE_ENV,
    browser: BrowserBehavior::CliMayOpenAutomatically,
    completion: CompletionBehavior::HostReconciliationRequired,
    status_argv: Some(&["auth", "status", "--json"]),
    status_contract: StatusContract::DocumentedExitCodes { logged_in: 0, logged_out: 1 },
};

const OPENCODE: ProviderLoginRecipe = ProviderLoginRecipe {
    provider: LoginProvider::OpenCode,
    pinned_version: "1.18.32",
    executable: "opencode",
    argv: &["auth", "login", "--pure", "--provider", "openai", "--method", "ChatGPT Pro/Plus (browser)"],
    instructions: "The fixed OpenAI browser method prints a complete authorization URL and waits on localhost:1455. The host opens that exact printed URL once for the User; no CLI selection, device code, or API key entry is required.",
    availability: RecipeAvailability::Supported,
    environment: OPENCODE_ENV,
    browser: BrowserBehavior::HostOpensPrintedAuthorization,
    completion: CompletionBehavior::HostReconciliationRequired,
    status_argv: Some(&["auth", "list", "--pure"]),
    status_contract: StatusContract::OpenCodeCredentialList,
};

const GROK: ProviderLoginRecipe = ProviderLoginRecipe {
    provider: LoginProvider::Grok,
    pinned_version: "1.0.41",
    executable: "grok",
    argv: &["login", "--oauth"],
    instructions: "The exact fixed binary's help confirms this login command and OAuth selection. Browser launch and successful completion are not runtime-qualified; do not infer either from argv or process exit.",
    availability: RecipeAvailability::Supported,
    environment: GROK_ENV,
    browser: BrowserBehavior::Unknown,
    completion: CompletionBehavior::Unsupported,
    status_argv: None,
    status_contract: StatusContract::Unsupported,
};

const ANTI_GRAVITY: ProviderLoginRecipe = ProviderLoginRecipe {
    provider: LoginProvider::Antigravity,
    pinned_version: "1.2.11",
    executable: "agy",
    argv: &[],
    instructions: "The fixed CLI signs in on first launch, but this is unsupported as an instance login: it uses the current Windows user's shared Credential Manager and has no evidenced login-only command.",
    availability: RecipeAvailability::Unsupported("login is bound to the shared Windows-user keyring and no login-only flow is evidenced"),
    environment: COMMON_HOME,
    browser: BrowserBehavior::CliMayOpenAutomatically,
    completion: CompletionBehavior::Unsupported,
    status_argv: None,
    status_contract: StatusContract::Unsupported,
};

const CODEX: ProviderLoginRecipe = ProviderLoginRecipe {
    provider: LoginProvider::Codex,
    pinned_version: "0.160",
    executable: "codex",
    argv: &[],
    instructions: "Out of scope: keep the existing Codex login path unchanged.",
    availability: RecipeAvailability::OutOfScope,
    environment: &[],
    browser: BrowserBehavior::Unknown,
    completion: CompletionBehavior::Unsupported,
    status_argv: None,
    status_contract: StatusContract::Unsupported,
};

pub(crate) fn recipe(provider: LoginProvider) -> &'static ProviderLoginRecipe {
    match provider {
        LoginProvider::Claude => &CLAUDE,
        LoginProvider::OpenCode => &OPENCODE,
        LoginProvider::Grok => &GROK,
        LoginProvider::Antigravity => &ANTI_GRAVITY,
        LoginProvider::Codex => &CODEX,
    }
}

impl EnvironmentIntent {
    /// Resolve a documented directory intent without touching the filesystem.
    pub(crate) fn value_for(self, instance_home: &Path) -> Option<String> {
        match self.value {
            EnvironmentValue::InstanceHome => Some(instance_home.to_string_lossy().into_owned()),
            EnvironmentValue::InstanceHomeChild(components) => {
                let path = components.iter().fold(instance_home.to_path_buf(), |path, component| {
                    path.join(*component)
                });
                Some(path.to_string_lossy().into_owned())
            }
            EnvironmentValue::PreserveUserValue | EnvironmentValue::RemoveInherited => None,
        }
    }
}

/// Extract one plain HTTPS URL for display as a manual fallback.
///
/// The full host output remains under H's existing stdout/stderr custody. This
/// function does not classify login success, persist the URL, or open it.
pub(crate) fn https_url_candidate_for_manual_owner_display(output: &str) -> Option<String> {
    let start = output.find("https://")?;
    let tail = &output[start..];
    let end = tail.find(|ch: char| ch.is_whitespace() || ch.is_control() || ch == '"' || ch == '\'')
        .unwrap_or(tail.len());
    let candidate = tail[..end].trim_end_matches(|ch| matches!(ch, '.' | ',' | ';' | ')' | ']' | '}'));
    let authority = candidate.strip_prefix("https://")?
        .split(|ch| matches!(ch, '/' | '?' | '#')).next()?;
    if candidate.len() <= "https://".len() || authority.is_empty()
        || authority.contains('@') || authority.starts_with('.') || authority.ends_with('.')
        || candidate.bytes().any(|byte| byte.is_ascii_control()) {
        return None;
    }
    Some(candidate.to_owned())
}
