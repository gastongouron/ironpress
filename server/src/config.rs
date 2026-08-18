//! Operator-controlled server configuration, fixed at startup.
//!
//! Everything that affects the process's security posture lives here and is
//! read once from the environment when the server boots. These values are
//! deliberately NOT part of the per-request form surface: an untrusted caller
//! must never be able to weaken sanitization or the network policy through a
//! request field.

use ironpress::{NetworkPolicy, RemoteHost};

/// Remote-network policy environment variables. They take effect only when
/// `IRONPRESS_REMOTE_ENABLED` is true; otherwise setting one is a no-op worth a
/// warning.
const REMOTE_POLICY_ENV_VARS: &[&str] = &[
    "IRONPRESS_REMOTE_ALLOW_HOSTS",
    "IRONPRESS_REMOTE_DENY_HOSTS",
    "IRONPRESS_REMOTE_DENY_PRIVATE_IPS",
    "IRONPRESS_REMOTE_DENY_PUBLIC_IPS",
    "IRONPRESS_REMOTE_MAX_REDIRECTS",
    "IRONPRESS_REMOTE_MAX_BODY_BYTES",
];

/// Server settings resolved from environment variables at startup.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// TCP port to listen on (`IRONPRESS_PORT`, default `3000`).
    pub port: u16,
    /// Whether HTML is sanitized before conversion (`IRONPRESS_SANITIZE`,
    /// default `true`). Disabling this trusts every request body; only do so
    /// for a fully trusted caller behind your own boundary.
    pub sanitize: bool,
    /// Maximum accepted request body size in bytes
    /// (`IRONPRESS_MAX_BODY_BYTES`, default 64 MiB).
    pub max_body_bytes: usize,
    /// Network policy for remote document resources. When remote fetching is
    /// disabled this is a block-all policy (no host can be reached); when
    /// enabled it is built entirely from the `IRONPRESS_REMOTE_*` variables.
    /// Never influenced by a request.
    pub network: NetworkPolicy,
    /// Whether outbound fetching of remote resources is enabled at runtime
    /// (`IRONPRESS_REMOTE_ENABLED`, default `false`).
    pub remote_enabled: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 3000,
            sanitize: true,
            max_body_bytes: 64 * 1024 * 1024,
            network: block_all_policy(),
            remote_enabled: false,
        }
    }
}

impl ServerConfig {
    /// Build the configuration from environment variables, falling back to the
    /// documented defaults for anything unset. Returns an error string if a
    /// remote host pattern is malformed.
    pub fn from_env() -> Result<Self, String> {
        let defaults = Self::default();
        let remote_enabled = env_bool("IRONPRESS_REMOTE_ENABLED").unwrap_or(defaults.remote_enabled);

        if !remote_enabled
            && REMOTE_POLICY_ENV_VARS
                .iter()
                .any(|k| std::env::var(k).is_ok())
        {
            eprintln!(
                "ironpress-server: warning: IRONPRESS_REMOTE_* policy variables are set but \
                 IRONPRESS_REMOTE_ENABLED is not true — they are ignored and no remote resource \
                 will be fetched."
            );
        }

        // When remote fetching is off, every remote reference is rejected before
        // any connection by a block-all policy. When on, the policy comes only
        // from the environment (SSRF-hardened defaults), never from a request.
        let network = if remote_enabled {
            network_policy_from_env()?
        } else {
            block_all_policy()
        };

        Ok(Self {
            port: env_parse("IRONPRESS_PORT").unwrap_or(defaults.port),
            sanitize: env_bool("IRONPRESS_SANITIZE").unwrap_or(defaults.sanitize),
            max_body_bytes: env_parse("IRONPRESS_MAX_BODY_BYTES").unwrap_or(defaults.max_body_bytes),
            network,
            remote_enabled,
        })
    }
}

/// A policy that rejects every remote address, blocking all outbound fetches
/// before any TCP connection is made. Denying both the non-public and public
/// address classes leaves no reachable target and no allow-list bypass.
fn block_all_policy() -> NetworkPolicy {
    NetworkPolicy::default()
        .deny_private_ips(true)
        .deny_public_ips(true)
}

/// Build the remote [`NetworkPolicy`] from `IRONPRESS_REMOTE_*` variables.
///
/// The base is the ironpress default (public hosts allowed, all non-public
/// address ranges — loopback, private, link-local, cloud metadata — denied), so
/// even a bare `--features remote` deployment is SSRF-hardened by default. Host
/// patterns are validated here so a typo fails fast at startup rather than
/// silently at request time.
fn network_policy_from_env() -> Result<NetworkPolicy, String> {
    let mut policy = NetworkPolicy::default();

    if let Some(hosts) = env_hosts("IRONPRESS_REMOTE_ALLOW_HOSTS")? {
        policy = policy.with_allow_list(hosts);
    }
    if let Some(hosts) = env_hosts("IRONPRESS_REMOTE_DENY_HOSTS")? {
        policy = policy.with_deny_list(hosts);
    }
    if let Some(v) = env_bool("IRONPRESS_REMOTE_DENY_PRIVATE_IPS") {
        policy = policy.deny_private_ips(v);
    }
    if let Some(v) = env_bool("IRONPRESS_REMOTE_DENY_PUBLIC_IPS") {
        policy = policy.deny_public_ips(v);
    }
    if let Some(v) = env_parse::<u32>("IRONPRESS_REMOTE_MAX_REDIRECTS") {
        policy = policy.max_redirects(v);
    }
    if let Some(v) = env_parse::<u64>("IRONPRESS_REMOTE_MAX_BODY_BYTES") {
        policy = policy.max_body_size(v);
    }

    Ok(policy)
}

/// Parse a comma-separated host list into [`RemoteHost`] patterns.
fn env_hosts(key: &str) -> Result<Option<Vec<RemoteHost>>, String> {
    let Ok(raw) = std::env::var(key) else {
        return Ok(None);
    };
    let mut hosts = Vec::new();
    for part in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let host = part
            .parse::<RemoteHost>()
            .map_err(|e| format!("{key}: invalid host `{part}`: {e}"))?;
        hosts.push(host);
    }
    Ok(Some(hosts))
}

fn env_parse<T: std::str::FromStr>(key: &str) -> Option<T> {
    std::env::var(key).ok()?.trim().parse().ok()
}

fn env_bool(key: &str) -> Option<bool> {
    match std::env::var(key).ok()?.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}
