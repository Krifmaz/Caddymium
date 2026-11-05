use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

/// What a Caddy binary does once started.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Acts {
    Serves,
    /// Runs but never answers anything (a hung process).
    Hangs,
    /// Exits at once.
    Dies,
}

/// A pretend Caddy process and its supervisor: killing or stopping it
/// starts the binary `bin` points at.
struct World {
    bin: String,
    acts: HashMap<String, Acts>,
    now: Acts,
    generation: u64,
    kills: u32,
    starts: Vec<String>,
}

impl World {
    fn new(bin: &str, acts: &[(&str, Acts)], now: Acts) -> Arc<Mutex<World>> {
        Arc::new(Mutex::new(World {
            bin: bin.into(),
            acts: acts.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            now,
            generation: 1,
            kills: 0,
            starts: Vec::new(),
        }))
    }
    fn restart(&mut self) {
        self.generation += 1;
        self.now = self.acts[&self.bin];
        self.starts.push(self.bin.clone());
    }
}

struct Api(Arc<Mutex<World>>);
struct Ctl(Arc<Mutex<World>>);

impl Api {
    fn now(&self) -> Acts {
        self.0.lock().unwrap().now
    }
}

impl CaddyApi for Api {
    fn ping<'a>(&'a self) -> BoxFut<'a, Result<()>> {
        Box::pin(async move {
            match self.now() {
                Acts::Serves => Ok(()),
                Acts::Hangs => std::future::pending().await,
                Acts::Dies => Err(anyhow!("connection refused")),
            }
        })
    }
    fn config<'a>(&'a self) -> BoxFut<'a, Result<Vec<u8>>> {
        Box::pin(async move {
            self.ping().await?;
            Ok(b"{}".to_vec())
        })
    }
    fn stop<'a>(&'a self) -> BoxFut<'a, Result<()>> {
        Box::pin(async move {
            match self.now() {
                Acts::Serves => {
                    self.0.lock().unwrap().restart();
                    Ok(())
                }
                Acts::Hangs => std::future::pending().await,
                Acts::Dies => Err(anyhow!("connection refused")),
            }
        })
    }
}

impl Control for Ctl {
    fn generation(&self) -> Option<u64> {
        Some(self.0.lock().unwrap().generation)
    }
    fn kill(&self) -> Result<()> {
        let mut w = self.0.lock().unwrap();
        w.kills += 1;
        w.restart();
        Ok(())
    }
}

fn restarter(w: &Arc<Mutex<World>>) -> Restarter {
    Restarter::new(Arc::new(Api(w.clone())), Arc::new(Ctl(w.clone())), vec![])
}

fn to(w: &Arc<Mutex<World>>, bin: &str) -> Fallback {
    let (w, bin) = (w.clone(), bin.to_string());
    Fallback {
        label: format!("the {bin} build"),
        apply: Box::new(move || {
            w.lock().unwrap().bin = bin.clone();
            Ok(())
        }),
    }
}

fn logger() -> (Arc<Mutex<Vec<String>>>, impl Fn(&str) + Send + Sync) {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let l = lines.clone();
    (lines, move |s: &str| l.lock().unwrap().push(s.to_string()))
}

fn ok_check() -> BoxFut<'static, Result<()>> {
    Box::pin(async { Ok(()) })
}

#[tokio::test(start_paused = true)]
async fn a_hung_caddy_is_killed_and_the_new_one_serves() {
    let w = World::new("new", &[("new", Acts::Serves)], Acts::Hangs);
    let r = restarter(&w);
    let (lines, log) = logger();
    let out = r.restart(&log, &ok_check, &[]).await;
    assert_eq!(out, Outcome::Serving);
    assert_eq!(w.lock().unwrap().kills, 1);
    let lines = lines.lock().unwrap().join("\n");
    assert!(lines.contains("did not answer the stop request"), "{lines}");
}

#[tokio::test(start_paused = true)]
async fn a_build_that_hangs_is_retried_once_then_rolled_back() {
    let w = World::new(
        "new",
        &[("new", Acts::Hangs), ("previous", Acts::Serves)],
        Acts::Serves,
    );
    let r = restarter(&w);
    let (lines, log) = logger();
    let started = tokio::time::Instant::now();
    let out = r.restart(&log, &ok_check, &[to(&w, "previous")]).await;
    match out {
        Outcome::RolledBack { to, why } => {
            assert_eq!(to, "the previous build");
            assert!(why.contains("not serving 30s after the restart"), "{why}");
        }
        o => panic!("{o:?}"),
    }
    assert_eq!(
        w.lock().unwrap().starts,
        vec!["new", "new", "previous"],
        "restart, one retry, then the rollback"
    );
    assert!(started.elapsed() < Duration::from_secs(120));
    let lines = lines.lock().unwrap().join("\n");
    assert!(lines.contains("restarting once more"), "{lines}");
    assert!(
        lines.contains("Caddy is serving again on the previous build"),
        "{lines}"
    );
}

#[tokio::test(start_paused = true)]
async fn the_check_can_reject_the_new_caddy() {
    let w = World::new(
        "new",
        &[("new", Acts::Serves), ("previous", Acts::Serves)],
        Acts::Serves,
    );
    let r = restarter(&w);
    let (_, log) = logger();
    fn wrong() -> BoxFut<'static, Result<()>> {
        Box::pin(async { Err(anyhow!("Caddy reports v2.10.0 instead of v2.11.0")) })
    }
    let out = r.restart(&log, &wrong, &[to(&w, "previous")]).await;
    assert!(
        matches!(&out, Outcome::RolledBack { why, .. } if why.contains("instead of")),
        "{out:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn fallbacks_are_tried_in_order_and_down_is_reported() {
    let w = World::new(
        "new",
        &[
            ("new", Acts::Dies),
            ("previous", Acts::Dies),
            ("image", Acts::Serves),
        ],
        Acts::Serves,
    );
    let r = restarter(&w);
    let (_, log) = logger();
    let out = r
        .restart(&log, &ok_check, &[to(&w, "previous"), to(&w, "image")])
        .await;
    assert!(
        matches!(&out, Outcome::RolledBack { to, .. } if to == "the image build"),
        "{out:?}"
    );

    let w = World::new("new", &[("new", Acts::Dies)], Acts::Serves);
    let out = restarter(&w).restart(&log, &ok_check, &[]).await;
    assert!(matches!(out, Outcome::Down { .. }), "{out:?}");
}

#[test]
fn listen_addresses() {
    assert_eq!(listen_port(":443"), Some(443));
    assert_eq!(listen_port("0.0.0.0:80"), Some(80));
    assert_eq!(listen_port("tcp/:8080"), Some(8080));
    assert_eq!(listen_port("tcp4/127.0.0.1:81"), Some(81));
    assert_eq!(listen_port(":8000-8010"), Some(8000));
    assert_eq!(listen_port("udp/:443"), None);
    assert_eq!(listen_port("unix//run/caddy.sock"), None);
    assert_eq!(listen_port("[::1]:444"), Some(444));

    let cfg = br#"{"apps":{"http":{"servers":{"srv0":{"listen":[":443"]},"srv1":{"listen":[":80","udp/:443"]}}}}}"#;
    assert_eq!(http_ports(cfg), vec![80, 443]);
    assert!(http_ports(b"{}").is_empty());

    let w = World::new("x", &[], Acts::Serves);
    let mut r = restarter(&w);
    r.probe_addrs = vec!["127.0.0.1:80".into()];
    assert_eq!(r.listeners(cfg), vec!["127.0.0.1:80", "127.0.0.1:443"]);
    assert!(
        r.listeners(b"{}").is_empty(),
        "nothing to dial without HTTP servers"
    );
}
