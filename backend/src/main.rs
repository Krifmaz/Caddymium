//! Caddymium: a web UI and API that owns a structured config, generates a
//! Caddyfile from it and applies it to Caddy through the admin API with
//! validation and automatic rollback.

mod apply;
mod auth;
mod backup;
mod caddy;
mod caddyfile;
mod config;
mod crowdsec;
mod ddns;
mod diff;
mod generator;
mod httpc;
mod importer;
mod kit;
mod model;
mod notify;
mod plugins;
mod recovery;
mod registry;
mod scan;
mod secrets;
mod server;
mod settings_form;
mod shields;
mod store;
mod suggestions;
mod supervisor;
mod updates;
mod watchdog;
mod web;

use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::registry::Capabilities;

/// Set at build time by the Dockerfile (CADDYMIUM_VERSION); "dev" otherwise.
const VERSION: &str = match option_env!("CADDYMIUM_VERSION") {
    Some(v) => v,
    None => "dev",
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a.trim_start_matches('-') == f);
    if flag("version") {
        println!("{VERSION}");
        return;
    }
    let cfg = config::Config::load();
    if flag("healthcheck") {
        std::process::exit(probe(&cfg.listen));
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
    httpc::init_tls();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    if let Err(e) = rt.block_on(run(cfg)) {
        tracing::error!("fatal: {e:#}");
        std::process::exit(1);
    }
}

async fn run(cfg: config::Config) -> Result<()> {
    std::fs::create_dir_all(&cfg.data_dir).context("data dir")?;
    let (key, key_src) = secrets::load_key(&cfg.secret_key, &cfg.secret_key_file, &cfg.data_dir)?;
    let secret_box = Arc::new(secrets::SecretBox::new(&key)?);
    if key_src == "generated" {
        tracing::warn!(
            "using a generated secret key stored in the data dir; set CADDYMIUM_SECRET_KEY to keep backups restorable on another host"
        );
    }
    let store = Arc::new(
        store::Store::open(&config::database_path(&cfg.data_dir), secret_box)
            .context("open database")?,
    );

    initial_admin(&cfg, &store)?;
    // Before anything is generated: new recommended protections never switch
    // on for existing sites through an update.
    if let Err(e) = suggestions::settle(&store) {
        tracing::warn!(err = %e, "could not check the Security defaults");
    }

    let runner = Arc::new(caddy::Runner::new(&cfg.caddy_bin));
    let admin = Arc::new(caddy::Admin::new(&cfg.caddy_admin));
    let notifier = Arc::new(notify::Notifier::new(store.clone()));
    let caps = Arc::new(RwLock::new(Capabilities::default()));

    let env = generator::Env {
        admin_listen: cfg.admin_listen.clone(),
        admin_origins: cfg.admin_origins.clone(),
        log_dir: cfg.log_dir.clone(),
        panel_upstream: cfg.panel_upstream.clone(),
        ..Default::default()
    };
    let n = notifier.clone();
    let pipeline = Arc::new(apply::Pipeline::new(apply::PipelineParts {
        store: store.clone(),
        validator: runner.clone(),
        admin: admin.clone(),
        health: Arc::new(apply::DefaultHealth {
            admin: admin.clone(),
            probe_addrs: cfg.probe_addrs.clone(),
            settle: Duration::from_millis(500),
        }),
        secrets: Some(Arc::new(secrets::Materializer::new(
            cfg.secrets_dir.clone(),
        ))),
        env,
        persist_path: cfg.caddyfile_path.clone(),
        caps: caps.clone(),
        notify: Some(Arc::new(move |event: &str, msg: &str| {
            n.send(event, "Caddymium: config not applied", msg)
        })),
    }));
    // How a Caddy that stopped answering is killed: the supervisor owns the
    // process in the all-in-one image; the caddy container's entrypoint
    // watches the shared bin volume in the two-container layout.
    let process = Arc::new(caddy::control::ProcessHandle::default());
    let control: Arc<dyn caddy::control::Control> = if cfg.manage_caddy {
        process.clone()
    } else {
        Arc::new(caddy::control::FileControl {
            bin_dir: cfg
                .caddy_bin
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default(),
        })
    };
    let n = notifier.clone();
    let plugins = Arc::new(
        plugins::Manager::new(
            store.clone(),
            admin.clone(),
            caps.clone(),
            pipeline.clone(),
            cfg.caddy_bin.clone(),
            cfg.caddy_arch.clone(),
            cfg.github_token.clone(),
        )
        .with_recovery(
            control,
            cfg.probe_addrs.clone(),
            Some(Arc::new(move |event: &str, title: &str, msg: &str| {
                n.send(event, title, msg)
            })),
        ),
    );

    // Built-in CrowdSec lives in the data volume; it needs Caddy in the same
    // container (all-in-one), since its API listens on loopback.
    let crowdsec = Arc::new(crowdsec::Manager::new(
        store.clone(),
        cfg.data_dir.join("crowdsec"),
        cfg.log_dir.clone(),
        cfg.github_token.clone(),
        cfg.manage_caddy,
    ));
    let state = server::AppState::new(server::AppParts {
        store: store.clone(),
        crowdsec: crowdsec.clone(),
        log_dir: cfg.log_dir.clone(),
        pipeline,
        runner: runner.clone(),
        admin,
        notifier,
        caps: caps.clone(),
        plugins,
        version: VERSION.into(),
        setup_token: cfg.setup_token.clone(),
        trust_xff: cfg.trust_proxy_headers,
        secure_cookie: cfg.base_url.starts_with("https://"),
        dev_no_auth: cfg.dev_no_auth,
        imports: server::ImportPaths {
            autosave_backup: supervisor::autosave_backup(&cfg.caddy_config_dir),
            autosave: cfg.caddy_config_dir.join("caddy").join("autosave.json"),
            caddyfile: cfg.import_caddyfile.clone(),
        },
        kit: kit::Kit::open(&cfg.data_dir, VERSION)?,
    });
    if cfg.dev_no_auth {
        tracing::warn!(
            "CADDYMIUM_DEV_NO_AUTH is set: the panel has NO authentication. Never use this in production."
        );
    }

    // One signal for everything: the HTTP server and the Caddy supervisor.
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        shutdown_signal().await;
        let _ = stop_tx.send(true);
    });

    // All-in-one mode: start Caddy ourselves and keep it running.
    if cfg.manage_caddy {
        let managed = supervisor::Managed {
            image_bin: cfg.caddy_image_bin.clone(),
            caddy_bin: cfg.caddy_bin.clone(),
            caddyfile: cfg.caddyfile_path.clone(),
            admin_listen: cfg.admin_listen.clone(),
            admin_origins: cfg.admin_origins.clone(),
            secrets_dir: cfg.secrets_dir.clone(),
            log_dir: std::path::PathBuf::from(&cfg.log_dir),
            data_dir: cfg.caddy_data_dir.clone(),
            config_dir: cfg.caddy_config_dir.clone(),
        };
        managed.prepare().context("prepare caddy directories")?;
        bootstrap_managed_config(&managed, &state).await;
        tokio::spawn(managed.run(state.admin.clone(), process.clone(), stop_rx.clone()));
    }

    // Restarts Caddy when it stops serving for a minute.
    if cfg.watchdog {
        let wd = Arc::new(watchdog::Watchdog::new(
            store.clone(),
            state.plugins.clone(),
            caps.clone(),
        ));
        tokio::spawn(wd.run(stop_rx.clone()));
    }

    // Update checker and automatic Caddy updates (settings decide what it does).
    tokio::spawn(state.updates.clone().run(stop_rx.clone()));
    // Built-in CrowdSec (runs only when turned on) and its daily chores.
    tokio::spawn(crowdsec.clone().run(stop_rx.clone()));
    tokio::spawn(crowdsec.clone().chores_loop(stop_rx.clone()));
    // AI crawler list refresh.
    tokio::spawn(shields::refresh_loop(
        store.clone(),
        state.pipeline.clone(),
        stop_rx.clone(),
    ));

    // Background jobs: Caddy capability detection and session cleanup.
    {
        let (runner, caps, store) = (runner.clone(), caps.clone(), store.clone());
        tokio::spawn(async move {
            let detect = || async {
                match runner.capabilities().await {
                    Ok(c) => {
                        let changed = caps
                            .read()
                            .map(|cur| {
                                cur.version_s != c.version_s || cur.modules.len() != c.modules.len()
                            })
                            .unwrap_or(true);
                        if let Ok(mut w) = caps.write() {
                            *w = c.clone();
                        }
                        if changed {
                            tracing::info!(
                                version = c.version_s,
                                modules = c.modules.len(),
                                "detected caddy"
                            );
                        }
                        true
                    }
                    Err(_) => false,
                }
            };
            while !detect().await {
                tokio::time::sleep(Duration::from_secs(15)).await;
            }
            loop {
                tokio::time::sleep(Duration::from_secs(600)).await;
                detect().await;
                store.prune_sessions();
            }
        });
    }

    let addr = listen_addr(&cfg.listen);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("listen on {addr}"))?;
    tracing::info!(addr = %addr, version = VERSION, data = %cfg.data_dir.display(), ui = web::available(), "Caddymium listening");
    let app = server::router(state);
    let mut stop = stop_rx;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        while !*stop.borrow() {
            if stop.changed().await.is_err() {
                break;
            }
        }
    })
    .await
    .context("http server")?;
    // Give the supervisor a moment to stop Caddy gracefully.
    tokio::time::sleep(Duration::from_millis(500)).await;
    tracing::info!("shut down");
    Ok(())
}

/// Before the first managed start of Caddy: if the persisted Caddyfile is
/// missing or was written for the two-container layout (another admin
/// address), regenerate it from the model with the binary that is about to
/// run, so sites come up immediately instead of waiting for an apply.
async fn bootstrap_managed_config(m: &supervisor::Managed, state: &server::AppState) {
    let current = std::fs::read_to_string(&m.caddyfile).unwrap_or_default();
    if current.contains(&m.admin_listen) && current.trim() != m.bootstrap_caddyfile().trim() {
        return; // already ours
    }
    if state.store.sites().map(|s| s.is_empty()).unwrap_or(true) && state.store.last_good().is_err()
    {
        return; // fresh install: the minimal config is right
    }
    let bin = m.choose().await;
    let runner = caddy::Runner::new(&bin);
    match runner.capabilities().await {
        Ok(c) => {
            if let Ok(mut w) = state.caps.write() {
                *w = c;
            }
        }
        Err(e) => {
            tracing::warn!(err = %e, "cannot inspect the Caddy binary; starting with the minimal config");
            return;
        }
    }
    match state.pipeline.plan_with(&runner).await {
        Ok(plan) if plan.valid => {
            match std::fs::write(&m.caddyfile, &plan.caddyfile) {
                Ok(()) => {
                    tracing::info!("regenerated the Caddyfile from the saved sites for this layout")
                }
                Err(e) => tracing::warn!(err = %e, "cannot write the regenerated Caddyfile"),
            }
            for w in plan.warnings {
                tracing::warn!("{w}");
            }
        }
        Ok(plan) => {
            let errs: Vec<String> = plan.errors.iter().map(|e| e.text.clone()).collect();
            tracing::warn!(
                stage = plan.stage,
                "saved config does not validate; starting with the minimal config: {}",
                errs.join("; ")
            );
        }
        Err(e) => {
            tracing::warn!(err = %e, "cannot regenerate the config; starting with the minimal config")
        }
    }
}

/// Turns ":8080" (Go style) into "0.0.0.0:8080".
fn listen_addr(l: &str) -> String {
    if let Some(port) = l.strip_prefix(':') {
        return format!("0.0.0.0:{port}");
    }
    l.to_string()
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
    tracing::info!("shutting down");
}

/// Creates an admin from CADDYMIUM_INITIAL_ADMIN_USER/PASSWORD on an empty
/// database (for unattended installs). Otherwise the UI's first-run setup
/// creates it.
fn initial_admin(cfg: &config::Config, st: &store::Store) -> Result<()> {
    if st.user_count() > 0 || cfg.initial_admin_user.is_empty() || cfg.initial_admin_pass.is_empty()
    {
        return Ok(());
    }
    auth::password_policy(&cfg.initial_admin_pass)
        .map_err(|e| anyhow::anyhow!("CADDYMIUM_INITIAL_ADMIN_PASSWORD: {e}"))?;
    let h = auth::hash_password(&cfg.initial_admin_pass)?;
    st.save_user(store::User {
        username: cfg.initial_admin_user.clone(),
        password_hash: h,
        role: auth::ROLE_ADMIN.into(),
        ..Default::default()
    })?;
    tracing::info!(
        user = cfg.initial_admin_user,
        "created initial admin from environment"
    );
    Ok(())
}

/// Probes the local panel (for Docker HEALTHCHECK).
fn probe(listen: &str) -> i32 {
    let mut addr = listen.to_string();
    if addr.starts_with(':') {
        addr = format!("127.0.0.1{addr}");
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let c = crate::httpc::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .expect("http client");
        match c.get(format!("http://{addr}/api/health")).send().await {
            Ok(r) if r.status().is_success() => 0,
            Ok(r) => {
                eprintln!("health: HTTP {}", r.status());
                1
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        }
    })
}
