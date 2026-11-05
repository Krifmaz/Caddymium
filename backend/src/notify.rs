//! Delivers alerts to webhooks and preset services.

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use crate::store::{Channel, Store};

// Events the panel emits.
pub const EVENT_APPLY_FAILED: &str = "apply_failed";
pub const EVENT_CERT_EXPIRING: &str = "cert_expiring";
pub const EVENT_CERT_ERROR: &str = "cert_error";
pub const EVENT_UPSTREAM_DOWN: &str = "upstream_down";
pub const EVENT_UPSTREAM_UP: &str = "upstream_up";
pub const EVENT_UPDATE_AVAILABLE: &str = "update_available";
pub const EVENT_UPDATE_INSTALLED: &str = "update_installed";
pub const EVENT_UPDATE_FAILED: &str = "update_failed";
pub const EVENT_BACKUP_FAILED: &str = "backup_failed";
pub const EVENT_CADDY_DOWN: &str = "caddy_down";
pub const EVENT_CADDY_RECOVERED: &str = "caddy_recovered";
pub const EVENT_DISCOVERY: &str = "discovery";
pub const EVENT_BUG_REPORT: &str = "bug_report";

/// Events for the UI.
pub const ALL_EVENTS: &[&str] = &[
    EVENT_APPLY_FAILED,
    EVENT_CERT_EXPIRING,
    EVENT_CERT_ERROR,
    EVENT_UPSTREAM_DOWN,
    EVENT_UPSTREAM_UP,
    EVENT_UPDATE_AVAILABLE,
    EVENT_UPDATE_INSTALLED,
    EVENT_UPDATE_FAILED,
    EVENT_BACKUP_FAILED,
    EVENT_DISCOVERY,
    EVENT_CADDY_DOWN,
    EVENT_CADDY_RECOVERED,
    EVENT_BUG_REPORT,
];

/// Supported channel types and their config fields.
pub const TYPES: &[(&str, &[&str])] = &[
    ("webhook", &["url", "header"]),
    ("ntfy", &["url", "token", "priority"]),
    ("gotify", &["url", "token", "priority"]),
    ("discord", &["url"]),
    ("slack", &["url"]),
    (
        "email",
        &["host", "port", "username", "password", "from", "to", "tls"],
    ),
];

/// Fields redacted in API responses.
pub fn is_secret_field(k: &str) -> bool {
    matches!(k, "token" | "password" | "header")
}

pub fn valid_type(t: &str) -> bool {
    TYPES.iter().any(|(k, _)| *k == t)
}

pub fn types_map() -> BTreeMap<&'static str, &'static [&'static str]> {
    TYPES.iter().cloned().collect()
}

/// A notification.
#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub event: String,
    pub title: String,
    pub message: String,
    pub time: DateTime<Utc>,
}

/// Fans out messages to channels.
pub struct Notifier {
    pub store: Arc<Store>,
    pub client: reqwest::Client,
}

impl Notifier {
    pub fn new(store: Arc<Store>) -> Notifier {
        Notifier {
            store,
            client: crate::httpc::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("http client"),
        }
    }

    /// Delivers asynchronously to subscribed, enabled channels.
    pub fn send(self: &Arc<Self>, event: &str, title: &str, msg: &str) {
        let Ok(chans) = self.store.channels() else {
            return;
        };
        let m = Message {
            event: event.into(),
            title: title.into(),
            message: msg.into(),
            time: Utc::now(),
        };
        for c in chans {
            if !c.enabled || !subscribed(&c.events, event) {
                continue;
            }
            let n = self.clone();
            let m = m.clone();
            tokio::spawn(async move {
                let r = tokio::time::timeout(Duration::from_secs(30), n.deliver(&c, &m)).await;
                match r {
                    Ok(Err(e)) => {
                        tracing::warn!(channel = c.name, kind = c.r#type, err = %e, "notification failed")
                    }
                    Err(_) => {
                        tracing::warn!(channel = c.name, kind = c.r#type, "notification timed out")
                    }
                    Ok(Ok(())) => {}
                }
            });
        }
    }

    /// Sends a test message synchronously.
    pub async fn test(&self, c: &Channel) -> Result<()> {
        self.deliver(
            c,
            &Message {
                event: "test".into(),
                title: "Caddymium test".into(),
                message: "If you can read this, notifications work.".into(),
                time: Utc::now(),
            },
        )
        .await
    }

    /// Sends one message to one channel.
    pub async fn deliver(&self, c: &Channel, m: &Message) -> Result<()> {
        let cfg = &c.config;
        let get = |k: &str| cfg.get(k).cloned().unwrap_or_default();
        match c.r#type.as_str() {
            "webhook" => {
                let body = serde_json::to_vec(m)?;
                let mut h = vec![("Content-Type".to_string(), "application/json".to_string())];
                let hv = get("header");
                if let Some((k, v)) = hv.split_once(':') {
                    h.push((k.trim().to_string(), v.trim().to_string()));
                }
                self.post(&get("url"), body, &h).await
            }
            "ntfy" => {
                let mut h = vec![
                    ("Title".to_string(), m.title.clone()),
                    ("Tags".to_string(), tag_for(&m.event).to_string()),
                ];
                let p = get("priority");
                if !p.is_empty() {
                    h.push(("Priority".into(), p));
                }
                let t = get("token");
                if !t.is_empty() {
                    h.push(("Authorization".into(), format!("Bearer {t}")));
                }
                self.post(&get("url"), m.message.clone().into_bytes(), &h)
                    .await
            }
            "gotify" => {
                let u = format!("{}/message", get("url").trim_end_matches('/'));
                let pr: i64 = get("priority").parse().unwrap_or(5);
                let body = serde_json::to_vec(
                    &serde_json::json!({"title": m.title, "message": m.message, "priority": pr}),
                )?;
                self.post(
                    &u,
                    body,
                    &[
                        ("Content-Type".into(), "application/json".into()),
                        ("X-Gotify-Key".into(), get("token")),
                    ],
                )
                .await
            }
            "discord" => {
                let body = serde_json::to_vec(
                    &serde_json::json!({"content": format!("**{}**\n{}", m.title, m.message)}),
                )?;
                self.post(
                    &get("url"),
                    body,
                    &[("Content-Type".into(), "application/json".into())],
                )
                .await
            }
            "slack" => {
                let body = serde_json::to_vec(
                    &serde_json::json!({"text": format!("*{}*\n{}", m.title, m.message)}),
                )?;
                self.post(
                    &get("url"),
                    body,
                    &[("Content-Type".into(), "application/json".into())],
                )
                .await
            }
            "email" => send_mail(cfg, m).await,
            other => Err(anyhow!("unknown channel type {other:?}")),
        }
    }

    async fn post(&self, u: &str, body: Vec<u8>, h: &[(String, String)]) -> Result<()> {
        if u.is_empty() {
            return Err(anyhow!("url is required"));
        }
        let pu = url::Url::parse(u).map_err(|_| anyhow!("invalid url"))?;
        if pu.scheme() != "http" && pu.scheme() != "https" {
            return Err(anyhow!("invalid url"));
        }
        let mut req = self.client.post(u).body(body);
        for (k, v) in h {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let b = resp.text().await.unwrap_or_default();
            let b: String = b.chars().take(512).collect();
            return Err(anyhow!("HTTP {}: {}", status.as_u16(), b.trim()));
        }
        Ok(())
    }
}

fn subscribed(events: &[String], e: &str) -> bool {
    events.is_empty() || events.iter().any(|x| x == e || x == "*")
}

fn tag_for(event: &str) -> &'static str {
    match event {
        EVENT_APPLY_FAILED | EVENT_CERT_ERROR | EVENT_UPSTREAM_DOWN | EVENT_BACKUP_FAILED
        | EVENT_UPDATE_FAILED | EVENT_CADDY_DOWN => "warning",
        EVENT_CADDY_RECOVERED => "leftwards_arrow_with_hook",
        EVENT_UPSTREAM_UP | EVENT_UPDATE_INSTALLED => "white_check_mark",
        EVENT_UPDATE_AVAILABLE => "package",
        _ => "bell",
    }
}

// ---------------------------------------------------------------- SMTP

fn sanitize_header(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
}

async fn send_mail(cfg: &BTreeMap<String, String>, m: &Message) -> Result<()> {
    let get = |k: &str| cfg.get(k).cloned().unwrap_or_default();
    let host = get("host");
    let mut port = get("port");
    if port.is_empty() {
        port = "587".into();
    }
    let (from, to_raw) = (get("from"), get("to"));
    if host.is_empty() || from.is_empty() || to_raw.is_empty() {
        return Err(anyhow!("host, from and to are required"));
    }
    let to: Vec<String> = to_raw
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    let msg = format!(
        "From: {from}\r\nTo: {}\r\nSubject: [Caddymium] {}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{}\r\n",
        to.join(", "),
        sanitize_header(&m.title),
        m.message
    );
    let tls_mode = get("tls");
    let implicit = tls_mode == "implicit" || port == "465";
    let addr = format!("{host}:{port}");
    let tcp = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    .map_err(|_| anyhow!("connect {addr}: timeout"))??;
    let mut conn = smtp::Conn::new(tcp);
    if implicit {
        conn = conn.upgrade(&host).await?;
    }
    conn.expect(220).await?;
    let me = "caddymium";
    let ehlo = conn.cmd(&format!("EHLO {me}"), 250).await?;
    if !implicit
        && tls_mode != "none"
        && ehlo
            .iter()
            .any(|l| l.to_ascii_uppercase().contains("STARTTLS"))
    {
        conn.cmd("STARTTLS", 220).await?;
        conn = conn.upgrade(&host).await?;
        conn.cmd(&format!("EHLO {me}"), 250).await?;
    }
    let user = get("username");
    if !user.is_empty() {
        let plain = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("\0{user}\0{}", get("password")),
        );
        conn.cmd(&format!("AUTH PLAIN {plain}"), 235).await?;
    }
    conn.cmd(&format!("MAIL FROM:<{from}>"), 250).await?;
    for r in &to {
        conn.cmd(&format!("RCPT TO:<{r}>"), 250).await?;
    }
    conn.cmd("DATA", 354).await?;
    let mut data = String::new();
    for line in msg.split("\r\n") {
        if line.starts_with('.') {
            data.push('.');
        }
        data.push_str(line);
        data.push_str("\r\n");
    }
    data.push_str(".\r\n");
    conn.write_raw(&data).await?;
    conn.expect(250).await?;
    let _ = conn.cmd("QUIT", 221).await;
    Ok(())
}

mod smtp {
    use anyhow::{Result, anyhow};
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

    type Stream = Box<dyn AsyncReadWrite>;
    pub trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
    impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

    pub struct Conn {
        r: BufReader<Stream>,
    }

    impl Conn {
        pub fn new<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(s: S) -> Conn {
            Conn {
                r: BufReader::new(Box::new(s)),
            }
        }

        /// Wraps the connection in TLS.
        pub async fn upgrade(self, host: &str) -> Result<Conn> {
            crate::httpc::init_tls();
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let cfg = rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();
            let name = rustls::pki_types::ServerName::try_from(host.to_string())
                .map_err(|_| anyhow!("invalid SMTP host name"))?;
            let inner = self.r.into_inner();
            let tls = tokio_rustls::TlsConnector::from(Arc::new(cfg))
                .connect(name, inner)
                .await?;
            Ok(Conn::new(tls))
        }

        /// Reads one (possibly multi-line) reply and checks its code.
        pub async fn expect(&mut self, code: u16) -> Result<Vec<String>> {
            let mut lines = Vec::new();
            loop {
                let mut line = String::new();
                let n = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    self.r.read_line(&mut line),
                )
                .await
                .map_err(|_| anyhow!("SMTP read timeout"))??;
                if n == 0 {
                    return Err(anyhow!("SMTP connection closed"));
                }
                let line = line.trim_end().to_string();
                if line.len() < 3 {
                    return Err(anyhow!("bad SMTP reply {line:?}"));
                }
                let got: u16 = line[..3]
                    .parse()
                    .map_err(|_| anyhow!("bad SMTP reply {line:?}"))?;
                let more = line.as_bytes().get(3) == Some(&b'-');
                lines.push(line.clone());
                if !more {
                    if got != code {
                        return Err(anyhow!("SMTP error: {line}"));
                    }
                    return Ok(lines);
                }
            }
        }

        pub async fn cmd(&mut self, c: &str, code: u16) -> Result<Vec<String>> {
            self.write_raw(&format!("{c}\r\n")).await?;
            self.expect(code).await
        }

        pub async fn write_raw(&mut self, s: &str) -> Result<()> {
            self.r.get_mut().write_all(s.as_bytes()).await?;
            self.r.get_mut().flush().await?;
            Ok(())
        }
    }
}
