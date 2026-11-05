use anyhow::{Result, anyhow};
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// A client for Caddy's admin API. Address may be "host:port",
/// "http://host:port" or "unix//path/to/socket".
#[derive(Debug, Clone)]
pub struct Admin {
    target: Target,
    /// Host header (must match admin origins)
    host: String,
    pub timeout: Duration,
}

#[derive(Debug, Clone)]
enum Target {
    Tcp(String),
    #[allow(dead_code)]
    Unix(String),
}

/// A non-2xx admin response.
#[derive(Debug, thiserror::Error)]
pub struct ApiError {
    pub status: u16,
    pub body: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut msg = self.body.trim().to_string();
        if let Ok(j) = serde_json::from_str::<serde_json::Value>(&msg) {
            if let Some(e) = j.get("error").and_then(|v| v.as_str()) {
                if !e.is_empty() {
                    msg = e.to_string();
                }
            }
        }
        write!(f, "admin API {}: {}", self.status, msg)
    }
}

/// An entry of /reverse_proxy/upstreams.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Upstream {
    pub address: String,
    pub num_requests: i64,
    pub fails: i64,
}

impl Admin {
    /// Creates an admin client.
    pub fn new(address: &str) -> Admin {
        if let Some(sock) = address.strip_prefix("unix/") {
            let sock = sock.split('|').next().unwrap_or(sock).to_string(); // strip file mode suffix
            return Admin {
                target: Target::Unix(sock),
                host: "localhost".into(),
                timeout: Duration::from_secs(30),
            };
        }
        let addr = address
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .trim_end_matches('/')
            .to_string();
        Admin {
            target: Target::Tcp(addr.clone()),
            host: addr,
            timeout: Duration::from_secs(30),
        }
    }

    async fn do_req(
        &self,
        method: &str,
        path: &str,
        ctype: &str,
        body: Option<Vec<u8>>,
        hdr: &[(&str, &str)],
    ) -> Result<Vec<u8>> {
        tokio::time::timeout(
            self.timeout,
            self.do_req_inner(method, path, ctype, body, hdr),
        )
        .await
        .map_err(|_| anyhow!("admin API request timed out"))?
    }

    async fn do_req_inner(
        &self,
        method: &str,
        path: &str,
        ctype: &str,
        body: Option<Vec<u8>>,
        hdr: &[(&str, &str)],
    ) -> Result<Vec<u8>> {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("Host", &self.host);
        if !ctype.is_empty() {
            req = req.header("Content-Type", ctype);
        }
        for (k, v) in hdr {
            req = req.header(*k, *v);
        }
        let req = req.body(Full::new(Bytes::from(body.unwrap_or_default())))?;
        let resp = match &self.target {
            Target::Tcp(addr) => {
                let stream = tokio::net::TcpStream::connect(addr)
                    .await
                    .map_err(|e| anyhow!("connect {addr}: {e}"))?;
                send(stream, req).await?
            }
            #[cfg(unix)]
            Target::Unix(sock) => {
                let stream = tokio::net::UnixStream::connect(sock)
                    .await
                    .map_err(|e| anyhow!("connect {sock}: {e}"))?;
                send(stream, req).await?
            }
            #[cfg(not(unix))]
            Target::Unix(sock) => {
                return Err(anyhow!(
                    "unix sockets are not supported on this platform ({sock})"
                ));
            }
        };
        let status = resp.status().as_u16();
        let b = resp.into_body().collect().await?.to_bytes().to_vec();
        if !(200..300).contains(&status) {
            return Err(ApiError {
                status,
                body: String::from_utf8_lossy(&b).to_string(),
            }
            .into());
        }
        Ok(b)
    }

    /// Checks that the admin API answers.
    pub async fn ping(&self) -> Result<()> {
        self.do_req("GET", "/config/", "", None, &[])
            .await
            .map(|_| ())
    }

    /// Loads a Caddyfile via POST /load. Caddy adapts it with the running
    /// binary and atomically rolls back on failure. `force` sets
    /// must-revalidate so an unchanged config is still reloaded (used to
    /// re-trigger certificate management).
    pub async fn load_caddyfile(&self, caddyfile: &str, force: bool) -> Result<()> {
        let hdr: &[(&str, &str)] = if force {
            &[("Cache-Control", "must-revalidate")]
        } else {
            &[]
        };
        self.do_req(
            "POST",
            "/load",
            "text/caddyfile",
            Some(caddyfile.as_bytes().to_vec()),
            hdr,
        )
        .await
        .map(|_| ())
    }

    /// Loads a JSON config (used to restore an exact previous state).
    pub async fn load_json(&self, cfg: &[u8]) -> Result<()> {
        self.do_req(
            "POST",
            "/load",
            "application/json",
            Some(cfg.to_vec()),
            &[("Cache-Control", "must-revalidate")],
        )
        .await
        .map(|_| ())
    }

    /// The running JSON config (GET /config/).
    pub async fn config(&self) -> Result<Vec<u8>> {
        self.do_req("GET", "/config/", "", None, &[]).await
    }

    /// Reverse proxy upstream status.
    pub async fn upstreams(&self) -> Result<Vec<Upstream>> {
        let b = self
            .do_req("GET", "/reverse_proxy/upstreams", "", None, &[])
            .await?;
        Ok(serde_json::from_slice(&b)?)
    }

    /// The Prometheus exposition text from the admin endpoint.
    #[allow(dead_code)]
    pub async fn metrics(&self) -> Result<String> {
        let b = self.do_req("GET", "/metrics", "", None, &[]).await?;
        Ok(String::from_utf8_lossy(&b).to_string())
    }

    /// Asks Caddy to exit (the container entrypoint / restart policy brings
    /// it back).
    pub async fn stop(&self) -> Result<()> {
        self.do_req("POST", "/stop", "", None, &[])
            .await
            .map(|_| ())
    }

    /// Polls until the admin API responds or the timeout elapses.
    #[cfg_attr(not(test), allow(dead_code))]
    pub async fn wait_ready(&self, timeout: Duration) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut last = String::new();
        while tokio::time::Instant::now() < deadline {
            let probe = Admin {
                timeout: Duration::from_secs(3),
                ..self.clone()
            };
            match probe.ping().await {
                Ok(()) => return Ok(()),
                Err(e) => last = e.to_string(),
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Err(anyhow!(
            "caddy admin API not ready after {:?}: {last}",
            timeout
        ))
    }
}

async fn send<S>(
    stream: S,
    req: Request<Full<Bytes>>,
) -> Result<hyper::Response<hyper::body::Incoming>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    Ok(sender.send_request(req).await?)
}
