use super::*;
use crate::apply::{DefaultHealth, Pipeline, PipelineParts};
use crate::secrets::{Materializer, SecretBox};
use serde_json::{Value, json};

struct TestServer {
    base: String,
    _dir: tempfile::TempDir,
    store: Arc<Store>,
}

async fn start() -> TestServer {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open_memory(Arc::new(
            SecretBox::new(b"test-key-material-test-key-material").unwrap(),
        ))
        .unwrap(),
    );
    let store_handle = store.clone();
    let runner = Arc::new(Runner::new(""));
    let admin = Arc::new(Admin::new("127.0.0.1:1"));
    let caps = Arc::new(RwLock::new(Capabilities::default()));
    let pipeline = Arc::new(Pipeline::new(PipelineParts {
        store: store.clone(),
        validator: runner.clone(),
        admin: admin.clone(),
        health: Arc::new(DefaultHealth {
            admin: admin.clone(),
            probe_addrs: vec![],
            settle: Duration::ZERO,
        }),
        secrets: Some(Arc::new(Materializer::new(dir.path().join("secrets")))),
        env: Default::default(),
        persist_path: dir.path().join("Caddyfile"),
        caps: caps.clone(),
        notify: None,
    }));
    let plugins = Arc::new(plugins::Manager::new(
        store.clone(),
        admin.clone(),
        caps.clone(),
        pipeline.clone(),
        dir.path().join("caddy"),
        String::new(),
        String::new(),
    ));
    let crowdsec = Arc::new(crate::crowdsec::Manager::new(
        store.clone(),
        dir.path().join("crowdsec"),
        String::new(),
        String::new(),
        false,
    ));
    let st = AppState::new(AppParts {
        store: store.clone(),
        crowdsec,
        log_dir: String::new(),
        pipeline,
        runner,
        admin,
        notifier: Arc::new(Notifier::new(store.clone())),
        caps,
        plugins,
        version: "test".into(),
        setup_token: String::new(),
        trust_xff: false,
        secure_cookie: false,
        dev_no_auth: false,
        imports: ImportPaths::default(),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = router(st);
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    TestServer {
        base,
        _dir: dir,
        store: store_handle,
    }
}

struct TestClient {
    base: String,
    http: reqwest::Client,
    csrf: String,
}

impl TestClient {
    fn new(base: &str) -> TestClient {
        TestClient {
            base: base.to_string(),
            http: crate::httpc::builder().cookie_store(true).build().unwrap(),
            csrf: String::new(),
        }
    }

    async fn call(&mut self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut req = self
            .http
            .request(method.parse().unwrap(), format!("{}{path}", self.base))
            .header("content-type", "application/json");
        if !self.csrf.is_empty() {
            req = req.header("x-csrf-token", &self.csrf);
        }
        if let Some(b) = body {
            req = req.body(b.to_string());
        }
        let resp = req.send().await.unwrap();
        let code = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let v: Value = serde_json::from_str(&text).unwrap_or(json!({"_raw": text}));
        if let Some(tok) = v.get("csrf").and_then(|t| t.as_str()) {
            if !tok.is_empty() {
                self.csrf = tok.to_string();
            }
        }
        (code, v)
    }
}

#[tokio::test]
async fn setup_login_and_csrf() {
    let ts = start().await;
    let mut c = TestClient::new(&ts.base);
    let (code, me) = c.call("GET", "/api/me", None).await;
    assert_eq!(code, 200);
    assert_eq!(me["needs_setup"], true);
    assert_eq!(c.call("GET", "/api/sites", None).await.0, 401);
    let (code, _) = c
        .call(
            "POST",
            "/api/setup",
            Some(json!({"username": "admin", "password": "short"})),
        )
        .await;
    assert_eq!(code, 400, "weak password accepted");
    let (code, out) = c
        .call(
            "POST",
            "/api/setup",
            Some(json!({"username": "admin", "password": "long enough password"})),
        )
        .await;
    assert_eq!(code, 200, "{out}");
    // Setup only works once.
    let mut other = TestClient::new(&ts.base);
    assert_eq!(
        other
            .call(
                "POST",
                "/api/setup",
                Some(json!({"username": "evil", "password": "long enough password"}))
            )
            .await
            .0,
        409
    );

    // Writes need the CSRF token.
    let tok = c.csrf.clone();
    c.csrf = String::new();
    assert_eq!(c.call("POST", "/api/sites", Some(json!({}))).await.0, 403);
    c.csrf = tok;

    let site = json!({"name": "app", "hosts": ["App.Example.com", "app.example.com"], "kind": "proxy", "enabled": true,
        "config": {"proxy": {"upstreams": ["localhost:8080", " "]}}});
    let (code, out) = c.call("POST", "/api/sites", Some(site.clone())).await;
    assert_eq!(code, 201, "{out}");
    assert_eq!(
        out["hosts"],
        json!(["app.example.com"]),
        "hosts not normalized"
    );
    // Same hostname on another enabled site is rejected.
    assert_eq!(c.call("POST", "/api/sites", Some(site)).await.0, 409);
    // Invalid input gets a helpful 400.
    let bad = json!({"hosts": ["app.example.com:8080"], "kind": "proxy", "config": {"proxy": {"upstreams": ["x:1"]}}});
    let (code, out) = c.call("POST", "/api/sites", Some(bad)).await;
    assert_eq!(code, 400);
    assert!(out["error"].as_str().unwrap().contains("port"), "{out}");

    // Login with wrong then right password.
    let mut l = TestClient::new(&ts.base);
    assert_eq!(
        l.call(
            "POST",
            "/api/login",
            Some(json!({"username": "admin", "password": "nope nope nope"}))
        )
        .await
        .0,
        401
    );
    let (code, out) = l
        .call(
            "POST",
            "/api/login",
            Some(json!({"username": "admin", "password": "long enough password"})),
        )
        .await;
    assert_eq!(code, 200, "{out}");
    assert!(out["csrf"].as_str().is_some_and(|s| !s.is_empty()));
    assert_eq!(l.call("POST", "/api/logout", None).await.0, 200);
    assert_eq!(l.call("GET", "/api/sites", None).await.0, 401);
}

#[tokio::test]
async fn roles_and_secret_masking() {
    let ts = start().await;
    let mut admin = TestClient::new(&ts.base);
    admin
        .call(
            "POST",
            "/api/setup",
            Some(json!({"username": "admin", "password": "long enough password"})),
        )
        .await;
    let (code, out) = admin
        .call(
            "POST",
            "/api/users",
            Some(json!({"username": "viewer", "password": "viewer password 1", "role": "viewer"})),
        )
        .await;
    assert_eq!(code, 200, "{out}");
    let mut v = TestClient::new(&ts.base);
    v.call(
        "POST",
        "/api/login",
        Some(json!({"username": "viewer", "password": "viewer password 1"})),
    )
    .await;
    assert_eq!(v.call("GET", "/api/sites", None).await.0, 200);
    assert_eq!(v.call("POST", "/api/sites", Some(json!({}))).await.0, 403);
    assert_eq!(v.call("GET", "/api/users", None).await.0, 403);

    // DNS credential secrets are stored but never returned.
    let (code, out) = admin
        .call(
            "POST",
            "/api/dns-credentials",
            Some(json!({"provider": "cloudflare", "fields": {"api_token": "super-secret-token"}})),
        )
        .await;
    assert_eq!(code, 200, "{out}");
    let (_, list) = admin.call("GET", "/api/dns-credentials", None).await;
    assert!(
        !list.to_string().contains("super-secret-token"),
        "secret leaked: {list}"
    );
    // Updating with an empty secret keeps the stored value.
    let id = out["id"].as_i64().unwrap();
    let (code, out) = admin
        .call(
            "PUT",
            &format!("/api/dns-credentials/{id}"),
            Some(json!({"provider": "cloudflare", "name": "cf", "fields": {"api_token": ""}})),
        )
        .await;
    assert_eq!(code, 200, "{out}");
    assert_eq!(out["set"]["api_token"], true);

    // The last admin can't be demoted.
    assert_eq!(
        admin
            .call(
                "PUT",
                "/api/users/1",
                Some(json!({"username": "admin", "role": "viewer"}))
            )
            .await
            .0,
        409
    );

    // API keys work without CSRF and respect roles.
    let (code, out) = admin
        .call(
            "POST",
            "/api/api-keys",
            Some(json!({"name": "ci", "role": "viewer"})),
        )
        .await;
    assert_eq!(code, 201, "{out}");
    let tok = out["token"].as_str().unwrap().to_string();
    let plain = crate::httpc::client();
    let r = plain
        .get(format!("{}/api/sites", ts.base))
        .bearer_auth(&tok)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let r = plain
        .post(format!("{}/api/sites", ts.base))
        .bearer_auth(&tok)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 403);

    // Settings round-trip keeps defaults and validates DDNS input.
    let (code, s) = admin.call("GET", "/api/settings", None).await;
    assert_eq!(code, 200);
    assert_eq!(s["ddns"]["check_interval"], "5m");
    let mut body = s.clone();
    body["ddns"]["enabled"] = json!(true);
    body["ddns"]["credential_id"] = json!(id);
    body["ddns"]["domains"] = json!([{"zone": "Example.com.", "names": ["@", " www "]}]);
    let (code, out) = admin.call("PUT", "/api/settings", Some(body)).await;
    assert_eq!(code, 200, "{out}");
    assert_eq!(out["ddns"]["domains"][0]["zone"], "example.com");
    assert_eq!(out["ddns"]["domains"][0]["names"], json!(["@", "www"]));
    assert_eq!(out["acme_eab_mac_set"], false);
    // The credential is now in use by DDNS.
    assert_eq!(
        admin
            .call("DELETE", &format!("/api/dns-credentials/{id}"), None)
            .await
            .0,
        409
    );
}

#[tokio::test]
async fn reach_explains_failures() {
    let ts = start().await;
    let mut c = TestClient::new(&ts.base);
    c.call(
        "POST",
        "/api/setup",
        Some(json!({"username": "admin", "password": "long enough password"})),
    )
    .await;
    // A listener that is up, a closed port, and a name that doesn't resolve.
    let up = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let up_addr = up.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        loop {
            let _ = up.accept().await;
        }
    });
    for (i, u) in [
        up_addr.as_str(),
        "127.0.0.1:1",
        "no-such-container.invalid:8096",
    ]
    .iter()
    .enumerate()
    {
        let site = json!({"name": i.to_string(), "hosts": [format!("s{i}.example.com")], "kind": "proxy", "enabled": true, "config": {"proxy": {"upstreams": [u]}}});
        let (code, out) = c.call("POST", "/api/sites", Some(site)).await;
        assert_eq!(code, 201, "{out}");
    }
    let (code, res) = c.call("GET", "/api/upstreams/reach", None).await;
    assert_eq!(code, 200, "{res}");
    let got: std::collections::HashMap<String, netscan::ReachResult> = res
        .as_array()
        .unwrap()
        .iter()
        .map(|r| serde_json::from_value::<netscan::ReachResult>(r.clone()).unwrap())
        .map(|r| (r.upstream.clone(), r))
        .collect();
    assert!(got[&up_addr].ok, "live upstream: {:?}", got[&up_addr]);
    let r = &got["127.0.0.1:1"];
    assert!(
        !r.ok && r.error.contains("nothing is listening"),
        "closed port: {r:?}"
    );
    let r = &got["no-such-container.invalid:8096"];
    assert!(
        !r.ok && r.error.contains("not found"),
        "unknown name: {r:?}"
    );
}

#[tokio::test]
async fn forward_proxy_site_validation() {
    let ts = start().await;
    let mut c = TestClient::new(&ts.base);
    c.call(
        "POST",
        "/api/setup",
        Some(json!({"username": "admin", "password": "long enough password"})),
    )
    .await;
    let bad = json!({"hosts": ["proxy.example.com"], "kind": "forward_proxy", "enabled": true,
        "config": {"forward_proxy": {"basic_auth": [{"username": "u", "password": ""}]}}});
    let (code, out) = c.call("POST", "/api/sites", Some(bad)).await;
    assert_eq!(code, 400, "{out}");
    let good = json!({"hosts": ["proxy.example.com"], "kind": "forward_proxy", "enabled": true,
        "config": {"forward_proxy": {"basic_auth": [{"username": "u", "password": "p"}], "acl": ["deny 10.0.0.0/8", "allow all"], "ports": [443, 0]}}});
    let (code, out) = c.call("POST", "/api/sites", Some(good)).await;
    assert_eq!(code, 201, "{out}");
    assert_eq!(out["config"]["forward_proxy"]["ports"], json!([443]));
    let (_, preview) = c.call("GET", "/api/config/preview", None).await;
    assert!(
        preview["caddyfile"]
            .as_str()
            .unwrap()
            .contains("forward_proxy {"),
        "{preview}"
    );
}

#[tokio::test]
async fn update_settings_and_report() {
    let ts = start().await;
    let mut admin = TestClient::new(&ts.base);
    admin
        .call(
            "POST",
            "/api/setup",
            Some(json!({"username": "admin", "password": "long enough password"})),
        )
        .await;
    // Defaults: checking on, installing off, nothing known yet.
    let (code, r) = admin.call("GET", "/api/updates", None).await;
    assert_eq!(code, 200, "{r}");
    assert_eq!(r["settings"]["check"], true);
    assert_eq!(r["settings"]["auto"], false);
    assert_eq!(r["settings"]["check_every_hours"], 6);
    assert_eq!(r["update_available"], false);
    assert_eq!(r["in_window"], true);
    assert_eq!(r["auto_blocked"], "automatic updates are off");
    let (code, s) = admin.call("GET", "/api/status", None).await;
    assert_eq!(code, 200);
    assert_eq!(s["update_available"], false);

    // Validation: a half window is refused, a bad time is refused.
    let (_, mut body) = admin.call("GET", "/api/settings", None).await;
    body["updates"]["window_start"] = json!("03:00");
    let (code, out) = admin.call("PUT", "/api/settings", Some(body.clone())).await;
    assert_eq!(code, 400, "{out}");
    body["updates"]["window_end"] = json!("5pm");
    let (code, _) = admin.call("PUT", "/api/settings", Some(body.clone())).await;
    assert_eq!(code, 400);

    // Automatic installs imply checking; the interval is kept sane.
    body["updates"] = json!({"check": false, "auto": true, "check_every_hours": 0,
        "window_start": " 03:00 ", "window_end": "05:00"});
    let (code, out) = admin.call("PUT", "/api/settings", Some(body)).await;
    assert_eq!(code, 200, "{out}");
    assert_eq!(out["updates"]["check"], true);
    assert_eq!(out["updates"]["check_every_hours"], 6);
    assert_eq!(out["updates"]["window_start"], "03:00");
    let (_, r) = admin.call("GET", "/api/updates", None).await;
    assert_eq!(r["settings"]["auto"], true);
    assert_eq!(r["auto_blocked"], "no update available");
    assert!(r["next_check"].is_string(), "{r}");
}

#[tokio::test]
async fn backup_restore_and_discard() {
    let ts = start().await;
    let mut admin = TestClient::new(&ts.base);
    admin
        .call(
            "POST",
            "/api/setup",
            Some(json!({"username": "admin", "password": "long enough password"})),
        )
        .await;
    let (code, site) = admin
        .call(
            "POST",
            "/api/sites",
            Some(
                json!({"name": "app", "hosts": ["app.example.com"], "kind": "proxy",
                "config": {"proxy": {"upstreams": ["127.0.0.1:3000"]}}}),
            ),
        )
        .await;
    assert_eq!(code, 201, "{site}");
    let site_id = site["id"].as_i64().unwrap();

    // Download: metadata in the clear, the rest sealed.
    let (code, env) = admin.call("GET", "/api/backup", None).await;
    assert_eq!(code, 200, "{env}");
    assert_eq!(env["format"], "caddymium-backup");
    assert_eq!(env["contents"]["sites"], 1);
    assert!(!env.to_string().contains("app.example.com"));

    // Change things, restore, and the site is back with its id.
    admin
        .call("DELETE", &format!("/api/sites/{site_id}"), None)
        .await;
    assert_eq!(
        admin
            .call("GET", "/api/sites", None)
            .await
            .1
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let (code, out) = admin
        .call("POST", "/api/backup/restore", Some(env.clone()))
        .await;
    assert_eq!(code, 200, "{out}");
    assert_eq!(out["restored"]["tables"]["sites"], 1);
    let (_, sites) = admin.call("GET", "/api/sites", None).await;
    assert_eq!(sites[0]["id"], site_id);
    assert_eq!(sites[0]["name"], "app");

    // Garbage and foreign archives are refused.
    let (code, out) = admin
        .call(
            "POST",
            "/api/backup/restore",
            Some(json!({"format": "nope"})),
        )
        .await;
    assert_eq!(code, 400, "{out}");
    let mut tampered = env.clone();
    tampered["data"] = json!("AAAA");
    assert_eq!(
        admin
            .call("POST", "/api/backup/restore", Some(tampered))
            .await
            .0,
        400
    );

    // Discard needs an applied version with a snapshot.
    let (code, out) = admin.call("POST", "/api/config/discard", None).await;
    assert_eq!(code, 409, "{out}");

    // Viewers can do none of this.
    let mut viewer = TestClient::new(&ts.base);
    admin
        .call(
            "POST",
            "/api/users",
            Some(json!({"username": "v", "password": "long enough password", "role": "viewer"})),
        )
        .await;
    viewer
        .call(
            "POST",
            "/api/login",
            Some(json!({"username": "v", "password": "long enough password"})),
        )
        .await;
    assert_eq!(viewer.call("GET", "/api/backup", None).await.0, 403);
    assert_eq!(
        viewer.call("POST", "/api/config/discard", None).await.0,
        403
    );
}

/// The whole sign-in flow without Caddy: redirect to the sign-in page,
/// password, first-time two-factor setup, the cookie for the parent domain,
/// and the verify answers Caddy's forward_auth gets.
#[tokio::test]
async fn sso_sign_in_flow() {
    let ts = start().await;
    let mut admin = TestClient::new(&ts.base);
    admin
        .call(
            "POST",
            "/api/setup",
            Some(json!({"username": "admin", "password": "long enough password"})),
        )
        .await;
    let (code, _) = admin
        .call(
            "POST",
            "/api/users",
            Some(json!({"username": "kid", "password": "member password 1", "role": "member"})),
        )
        .await;
    assert_eq!(code, 200);
    let (code, sec) = admin.call("GET", "/api/security", None).await;
    assert_eq!(code, 200, "{sec}");
    let mut body = json!({"security": sec["security"].clone()});
    body["security"]["sso"]["portal_host"] = json!("auth.example.com");
    body["security"]["defaults"]["login"] = json!("sso");
    let (code, out) = admin.call("PUT", "/api/security", Some(body)).await;
    assert_eq!(code, 200, "{out}");
    assert_eq!(out["sso"]["cookie_domain"], "example.com");

    // A member can't use the panel.
    let mut kid = TestClient::new(&ts.base);
    let (code, _) = kid
        .call(
            "POST",
            "/api/login",
            Some(json!({"username": "kid", "password": "member password 1"})),
        )
        .await;
    assert_eq!(code, 403);

    let http = crate::httpc::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let fwd = |req: reqwest::RequestBuilder| {
        req.header("x-forwarded-host", "app.example.com")
            .header("x-forwarded-proto", "https")
            .header("x-forwarded-uri", "/movies?x=1")
    };
    // Not signed in: sent to the sign-in page with the way back.
    let r = fwd(http.get(format!("{}/api/sso/verify?site=0", ts.base)))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 302);
    let loc = r.headers()["location"].to_str().unwrap().to_string();
    assert!(
        loc.starts_with("https://auth.example.com/sso/login?rd="),
        "{loc}"
    );
    assert!(loc.contains("app.example.com%2Fmovies"), "{loc}");

    let field = |html: &str, name: &str| -> String {
        let key = format!("name=\"{name}\" value=\"");
        let i = html
            .find(&key)
            .unwrap_or_else(|| panic!("no {name} in {html}"))
            + key.len();
        html[i..i + html[i..].find('"').unwrap()].to_string()
    };
    let set_cookie = |r: &reqwest::Response, name: &str| -> Option<String> {
        r.headers()
            .get_all("set-cookie")
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .find(|c| c.starts_with(&format!("{name}=")))
    };
    let rd = "https://app.example.com/movies?x=1";
    let r = http
        .get(format!(
            "{}/sso/login?rd={}",
            ts.base,
            url::form_urlencoded::byte_serialize(rd.as_bytes()).collect::<String>()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let csrf_c = set_cookie(&r, "caddymium_sso_csrf").unwrap();
    let csrf_c = csrf_c.split(';').next().unwrap().to_string();
    let html = r.text().await.unwrap();
    let csrf = field(&html, "csrf");

    // Wrong password, then right password: two-factor setup comes first.
    let post = |path: &str, form: Vec<(&str, String)>, cookie: &str| {
        let mut enc = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in &form {
            enc.append_pair(k, v);
        }
        http.post(format!("{}{path}", ts.base))
            .header("cookie", cookie.to_string())
            .header("content-type", "application/x-www-form-urlencoded")
            .body(enc.finish())
            .send()
    };
    let r = post(
        "/sso/login",
        vec![
            ("username", "kid".into()),
            ("password", "nope".into()),
            ("rd", rd.into()),
            ("csrf", csrf.clone()),
        ],
        &csrf_c,
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 401);
    let csrf_c = set_cookie(&r, "caddymium_sso_csrf")
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let csrf = field(&r.text().await.unwrap(), "csrf");
    let r = post(
        "/sso/login",
        vec![
            ("username", "kid".into()),
            ("password", "member password 1".into()),
            ("rd", rd.into()),
            ("csrf", csrf),
        ],
        &csrf_c,
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let csrf_c = set_cookie(&r, "caddymium_sso_csrf")
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let html = r.text().await.unwrap();
    assert!(html.contains("Set up two-factor"), "{html}");
    assert!(html.contains("<svg"), "QR code shown");
    let secret = {
        let i = html.find("<code>").unwrap() + 6;
        html[i..i + html[i..].find('<').unwrap()].to_string()
    };
    let code = crate::auth::totp_code(&secret, chrono::Utc::now().timestamp());
    let r = post(
        "/sso/enroll",
        vec![
            ("id", field(&html, "id")),
            ("code", code),
            ("rd", rd.into()),
            ("csrf", field(&html, "csrf")),
        ],
        &csrf_c,
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 303);
    assert_eq!(r.headers()["location"], rd);
    let sso = set_cookie(&r, "caddymium_sso").unwrap();
    assert!(sso.contains("Domain=example.com"), "{sso}");
    assert!(sso.contains("HttpOnly") && sso.contains("Secure"), "{sso}");
    let sso = sso.split(';').next().unwrap().to_string();

    // Signed in: Caddy gets the user.
    let r = fwd(http.get(format!("{}/api/sso/verify?site=0", ts.base)))
        .header("cookie", sso.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(r.headers()["remote-user"], "kid");

    // A site limited to other accounts refuses this one.
    let (code, site) = admin
        .call(
            "POST",
            "/api/sites",
            Some(json!({"name": "only-mom", "hosts": ["mom.example.com"], "kind": "proxy",
                "config": {"proxy": {"upstreams": ["127.0.0.1:3000"]}, "access": {"shields": {"sso_users": ["mom"]}}}})),
        )
        .await;
    assert_eq!(code, 201, "{site}");
    let r = fwd(http.get(format!("{}/api/sso/verify?site={}", ts.base, site["id"])))
        .header("cookie", sso.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 403);

    // Redirects only go back to sites under the cookie domain.
    let r = http
        .get(format!(
            "{}/sso/login?rd=https%3A%2F%2Fevil.com%2F",
            ts.base
        ))
        .header("cookie", sso.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200, "no redirect to a foreign site");

    // The admin sees the sign-in and can end it.
    let (_, list) = admin.call("GET", "/api/security/sessions", None).await;
    assert_eq!(list[0]["username"], "kid");
    assert_eq!(list[0]["mfa"], true);
    let (code, _) = admin
        .call("DELETE", "/api/security/sessions/all", None)
        .await;
    assert_eq!(code, 200);
    let r = fwd(http.get(format!("{}/api/sso/verify?site=0", ts.base)))
        .header("cookie", sso)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 302, "signed out everywhere");
}

#[tokio::test]
async fn security_suggestions_card() {
    let ts = start().await;
    let mut admin = TestClient::new(&ts.base);
    admin
        .call(
            "POST",
            "/api/setup",
            Some(json!({"username": "admin", "password": "long enough password"})),
        )
        .await;
    let site = json!({"name": "app", "hosts": ["app.example.com"], "kind": "proxy", "enabled": true,
        "config": {"proxy": {"upstreams": ["localhost:8080"]}}});
    let (code, out) = admin.call("POST", "/api/sites", Some(site)).await;
    assert_eq!(code, 201, "{out}");
    let id = out["id"].as_i64().unwrap();
    // What the next start does on an install that already has sites.
    crate::suggestions::settle(&ts.store).unwrap();

    let (code, sec) = admin.call("GET", "/api/security", None).await;
    assert_eq!(code, 200);
    assert_eq!(sec["security"]["defaults"]["bots"], json!(false));
    let ids: Vec<&str> = sec["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["bots", "privacy", "crowdsec"]);
    assert_eq!(sec["suggestions"][0]["sites"][0]["name"], json!("app"));

    let (code, out) = admin
        .call(
            "POST",
            "/api/security/suggestions/bots/apply",
            Some(json!({"site_id": id})),
        )
        .await;
    assert_eq!(code, 200, "{out}");
    assert_eq!(out["sites"][0]["effective"]["bots"], json!(true));
    let (code, out) = admin
        .call("POST", "/api/security/suggestions/privacy/apply", None)
        .await;
    assert_eq!(code, 200, "{out}");
    assert_eq!(out["security"]["defaults"]["privacy"], json!(true));
    let (code, out) = admin
        .call("POST", "/api/security/suggestions/crowdsec/dismiss", None)
        .await;
    assert_eq!(code, 200, "{out}");
    assert_eq!(out["suggestions"], json!([]));
    assert_eq!(
        out["pending_changes"],
        json!(true),
        "goes through Review & apply"
    );
    let (code, _) = admin
        .call("POST", "/api/security/suggestions/nope/apply", None)
        .await;
    assert_eq!(code, 400);
}

#[tokio::test]
async fn errors_say_what_to_do() {
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;
    let body = |e: ApiError| async move {
        let r = e.into_response();
        let b = r.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice::<Value>(&b).unwrap()["error"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let m = body(ApiError::internal("database is locked")).await;
    assert!(
        m.starts_with("Something went wrong: database is locked."),
        "{m}"
    );
    assert!(m.contains("docker logs caddymium"), "{m}");
    let m = body(ApiError::bad("host is required")).await;
    assert_eq!(m, "host is required", "client errors are already specific");
    let nf = ApiError::from(anyhow::Error::new(crate::store::NotFound));
    let m = body(nf).await;
    assert!(m.contains("Reload the page"), "{m}");
}
