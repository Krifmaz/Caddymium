//! Configuration comes entirely from environment variables (and `*_FILE`
//! Docker secrets). Defaults match the shipped docker-compose.yml; nothing is
//! specific to any one deployment.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    pub listen: String,
    pub data_dir: PathBuf,
    pub secret_key: String,
    pub secret_key_file: String,

    /// Binary used for adapt/validate (shared from the caddy container).
    pub caddy_bin: PathBuf,
    /// Address the panel uses to reach the admin API.
    pub caddy_admin: String,
    /// Address Caddy's admin endpoint binds to (written into the config).
    pub admin_listen: String,
    /// Allowed admin origins.
    pub admin_origins: Vec<String>,
    /// Persisted Caddyfile Caddy starts from.
    pub caddyfile_path: PathBuf,
    /// Shared secrets directory (same path in both containers).
    pub secrets_dir: PathBuf,
    /// Caddy log directory (same path in both containers).
    pub log_dir: String,
    /// Caddy's data dir mounted into the panel (certificates).
    pub caddy_data_dir: PathBuf,
    /// Caddy's config dir mounted read-only (autosave.json, for import).
    pub caddy_config_dir: PathBuf,
    /// Optional old Caddyfile mounted for import.
    pub import_caddyfile: PathBuf,
    /// Caddy listeners to probe after apply.
    pub probe_addrs: Vec<String>,
    /// How Caddy reaches the panel (when proxying the panel).
    pub panel_upstream: String,
    /// CPU architecture of the Caddy container for the plugin installer
    /// (amd64, arm64, ...). Empty = detect from the Caddy binary.
    pub caddy_arch: String,

    /// All-in-one mode: the panel starts and supervises Caddy itself
    /// instead of talking to a separate container.
    pub manage_caddy: bool,
    /// The Caddy binary shipped in the image (used when no custom build is
    /// installed), in all-in-one mode.
    pub caddy_image_bin: PathBuf,

    /// Public URL of the panel (cookies).
    pub base_url: String,
    /// Trust X-Forwarded-For from Caddy for client IPs.
    pub trust_proxy_headers: bool,
    /// Restart Caddy when it stops serving for a minute.
    pub watchdog: bool,
    /// Optional fixed setup token.
    pub setup_token: String,
    pub initial_admin_user: String,
    pub initial_admin_pass: String,

    /// Optional, raises GitHub API rate limits for update checks.
    pub github_token: String,

    /// Never set in production; for local UI development only.
    pub dev_no_auth: bool,
}

/// Prefix of every variable, and the one used before the project was
/// renamed from CaddyHub. Each variable is read under the current name
/// first, then under the old one, so existing deployments keep working.
const PREFIX: &str = "CADDYMIUM_";
const LEGACY_PREFIX: &str = "CADDYHUB_";

/// The names a variable is read under, in order of precedence.
fn names(key: &str) -> Vec<String> {
    let mut v = vec![key.to_string()];
    if let Some(rest) = key.strip_prefix(PREFIX) {
        v.push(format!("{LEGACY_PREFIX}{rest}"));
    }
    v
}

/// The first non-empty value among the names of `key`.
fn lookup(key: &str) -> Option<String> {
    names(key)
        .into_iter()
        .find_map(|n| std::env::var(n).ok().filter(|v| !v.is_empty()))
}

fn env(key: &str, def: &str) -> String {
    lookup(key).unwrap_or_else(|| def.to_string())
}

/// Reads KEY, or the file named by KEY_FILE (Docker secrets), under the
/// current name first and then the old one.
fn env_secret(key: &str) -> String {
    for n in names(key) {
        if let Some(v) = std::env::var(&n).ok().filter(|v| !v.is_empty()) {
            return v;
        }
        if let Some(b) = std::env::var(format!("{n}_FILE"))
            .ok()
            .filter(|f| !f.is_empty())
            .and_then(|f| std::fs::read_to_string(f).ok())
        {
            return b.trim().to_string();
        }
    }
    String::new()
}

fn env_list(key: &str, def: &str) -> Vec<String> {
    env(key, def)
        .split(',')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect()
}

fn env_bool(key: &str, def: bool) -> bool {
    match lookup(key) {
        Some(v) => match v.to_ascii_lowercase().as_str() {
            "1" | "t" | "true" | "yes" | "y" | "on" => true,
            "0" | "f" | "false" | "no" | "n" | "off" => false,
            _ => def,
        },
        None => def,
    }
}

/// The database file: the current name, unless only a database from
/// before the rename exists, which is then used where it is.
pub fn database_path(data_dir: &Path) -> PathBuf {
    let current = data_dir.join("caddymium.db");
    let legacy = data_dir.join("caddyhub.db");
    if !current.exists() && legacy.exists() {
        legacy
    } else {
        current
    }
}

impl Config {
    /// Reads configuration from the environment.
    ///
    /// Defaults depend on the deployment layout. In all-in-one mode
    /// (CADDYMIUM_MANAGE_CADDY=true, the default image) everything lives
    /// under the data dir and Caddy's admin API is on localhost; in the
    /// two-container layout the paths are the volumes shared with the caddy
    /// container.
    pub fn load() -> Config {
        // The image sets CADDYMIUM_MANAGE_CADDY_DEFAULT=true; a MANAGE_CADDY
        // set by the user (under either name) overrides it.
        let manage_caddy = env_bool(
            "CADDYMIUM_MANAGE_CADDY",
            env_bool("CADDYMIUM_MANAGE_CADDY_DEFAULT", false),
        );
        let data_dir = PathBuf::from(env("CADDYMIUM_DATA_DIR", "/data"));
        let d = |rel: &str| data_dir.join(rel).to_string_lossy().replace('\\', "/");
        let (
            def_admin,
            def_bin,
            def_caddyfile,
            def_secrets,
            def_logs,
            def_cdata,
            def_cconfig,
            def_probe,
            def_upstream,
        ) = if manage_caddy {
            (
                "127.0.0.1:2019".to_string(),
                d("caddy/bin/caddy"),
                d("caddy/Caddyfile"),
                d("caddy/secrets"),
                d("caddy/logs"),
                d("caddy-data"),
                d("caddy-config"),
                "127.0.0.1:80",
                "127.0.0.1:8080",
            )
        } else {
            (
                "caddy-admin:2019".to_string(),
                "/caddy-bin/caddy".into(),
                "/etc/caddyhub/Caddyfile".into(),
                "/run/caddyhub/secrets".into(),
                "/var/log/caddy".into(),
                "/caddy-data".into(),
                "/caddy-config".into(),
                "caddy:80",
                "panel:8080",
            )
        };
        let caddy_admin = env("CADDYMIUM_CADDY_ADMIN", &def_admin);
        let admin_listen = env("CADDYMIUM_ADMIN_LISTEN", &caddy_admin);
        let mut admin_origins = env_list("CADDYMIUM_ADMIN_ORIGINS", &caddy_admin);
        if admin_listen.starts_with("unix/") {
            admin_origins = Vec::new();
        }
        Config {
            listen: env("CADDYMIUM_LISTEN", ":8080"),
            data_dir,
            secret_key: lookup("CADDYMIUM_SECRET_KEY").unwrap_or_default(),
            secret_key_file: lookup("CADDYMIUM_SECRET_KEY_FILE").unwrap_or_default(),

            caddy_bin: PathBuf::from(env("CADDYMIUM_CADDY_BIN", &def_bin)),
            caddy_admin,
            admin_listen,
            admin_origins,
            caddyfile_path: PathBuf::from(env("CADDYMIUM_CADDYFILE", &def_caddyfile)),
            secrets_dir: PathBuf::from(env("CADDYMIUM_SECRETS_DIR", &def_secrets)),
            log_dir: env("CADDYMIUM_LOG_DIR", &def_logs),
            caddy_data_dir: PathBuf::from(env("CADDYMIUM_CADDY_DATA", &def_cdata)),
            caddy_config_dir: PathBuf::from(env("CADDYMIUM_CADDY_CONFIG", &def_cconfig)),
            import_caddyfile: PathBuf::from(env("CADDYMIUM_IMPORT_CADDYFILE", "/import/Caddyfile")),
            probe_addrs: env_list("CADDYMIUM_PROBE", def_probe),
            panel_upstream: env("CADDYMIUM_PANEL_UPSTREAM", def_upstream),
            caddy_arch: env("CADDYMIUM_CADDY_ARCH", ""),
            manage_caddy,
            caddy_image_bin: PathBuf::from(env("CADDYMIUM_CADDY_IMAGE_BIN", "/usr/bin/caddy")),

            base_url: lookup("CADDYMIUM_BASE_URL")
                .unwrap_or_default()
                .trim_end_matches('/')
                .to_string(),
            trust_proxy_headers: env_bool("CADDYMIUM_TRUST_PROXY_HEADERS", false),
            watchdog: env_bool("CADDYMIUM_WATCHDOG", true),
            setup_token: env_secret("CADDYMIUM_SETUP_TOKEN"),
            initial_admin_user: lookup("CADDYMIUM_INITIAL_ADMIN_USER").unwrap_or_default(),
            initial_admin_pass: env_secret("CADDYMIUM_INITIAL_ADMIN_PASSWORD"),

            github_token: env_secret("CADDYMIUM_GITHUB_TOKEN"),
            dev_no_auth: env_bool("CADDYMIUM_DEV_NO_AUTH", false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_names_are_read_as_fallback() {
        assert_eq!(
            names("CADDYMIUM_LISTEN"),
            ["CADDYMIUM_LISTEN", "CADDYHUB_LISTEN"]
        );
        assert_eq!(names("XDG_DATA_HOME"), ["XDG_DATA_HOME"]);
    }

    #[test]
    fn database_path_keeps_an_existing_legacy_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(database_path(dir.path()), dir.path().join("caddymium.db"));
        std::fs::write(dir.path().join("caddyhub.db"), b"x").unwrap();
        assert_eq!(database_path(dir.path()), dir.path().join("caddyhub.db"));
        std::fs::write(dir.path().join("caddymium.db"), b"x").unwrap();
        assert_eq!(database_path(dir.path()), dir.path().join("caddymium.db"));
    }
}
