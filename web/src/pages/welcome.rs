//! First-run setup for an admin: certificates, then an existing config, then where to go next.

use crate::app::{self, i};
use crate::form::{Select, Text};
use crate::pages::import::Import;
use crate::ui::{toast_err, Alert, ErrorBox, Loading};
use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::{json, Value};

/// True while an admin has not finished first-run setup (`setup_complete: false` on `/api/me`).
pub fn due() -> bool {
    let me = app::me().get();
    me.get("user").is_some_and(|u| !u.is_null()) && app::is_admin() && me.get("setup_complete") == Some(&json!(false))
}

/// The settings to save once setup is done.
pub fn finished(settings: &Value) -> Value {
    let mut s = settings.clone();
    s["setup_complete"] = json!(true);
    s
}

pub const CAS: &[(&str, &str)] = &[
    ("letsencrypt", "Let's Encrypt (recommended)"),
    ("letsencrypt_staging", "Let's Encrypt staging (testing: untrusted certs, no rate limits)"),
    ("zerossl", "ZeroSSL"),
    ("google", "Google Trust Services"),
];

#[component]
pub fn Welcome() -> impl IntoView {
    let step = RwSignal::new(1u8);
    let settings = RwSignal::new(Value::Null);
    let imported = RwSignal::new(None::<Value>);
    let error = RwSignal::new(String::new());
    rt::spawn(async move {
        match api::get("/settings").await {
            Ok(v) => settings.set(v),
            Err(e) => toast_err(e),
        }
    });

    let save_basics = move |_| {
        error.set(String::new());
        let body = settings.get_untracked();
        rt::spawn(async move {
            match api::put("/settings", body).await {
                Ok(v) => {
                    settings.set(v);
                    step.set(2);
                }
                Err(e) => error.set(e.message),
            }
        });
    };
    let finish = move |to: &'static str| {
        let body = finished(&settings.get_untracked());
        rt::spawn(async move {
            match api::put("/settings", body).await {
                Ok(v) => {
                    settings.set(v);
                    let mut me = app::me().get_untracked();
                    me["setup_complete"] = json!(true);
                    app::set_me(me);
                    app::refresh_status();
                    app::navigate(to);
                }
                Err(e) => toast_err(e),
            }
        });
    };
    let on_done = Callback::new(move |d: Value| imported.set(Some(d)));
    let ca_options: Vec<(String, String)> = CAS.iter().map(|(v, l)| (v.to_string(), l.to_string())).collect();

    let body = move || {
        if settings.get().is_null() {
            return view! { <Loading/> }.into_any();
        }
        match step.get() {
            1 => view! {
                <div class="card stack">
                    <h2>"HTTPS certificates"</h2>
                    <p class="muted">"Caddy gets free certificates for your sites automatically. The certificate authority emails you only if something needs attention."</p>
                    <Text state=settings path="acme_email" label="Email for certificate notices" kind="email" placeholder="you@example.com"/>
                    <Select state=settings path="acme_ca" label="Certificate authority" options=ca_options.clone()/>
                    {move || (!error.get().is_empty()).then(|| view! { <ErrorBox message=error.get() title="Couldn't save the settings"/> })}
                    <div><button class="btn primary" on:click=save_basics>"Continue"</button></div>
                </div>
            }
            .into_any(),
            2 => view! {
                <div class="card stack">
                    <h2>"Bring in your existing config"</h2>
                    <p class="muted">"Caddymium looks for the config your Caddy is already running (or ran before). You can also paste a Caddyfile. Anything it can't turn into forms is kept for you to review."</p>
                </div>
                {move || match imported.get() {
                    Some(d) => view! { <Alert kind="ok">{format!("Imported {} site(s). You'll review the result before anything goes live.", i(&d, "sites"))}</Alert> }.into_any(),
                    None => view! { <Import embedded=true on_done=on_done/> }.into_any(),
                }}
                <div><button class="btn primary" on:click=move |_| step.set(3)>
                    {move || if imported.get().is_some() { "Continue" } else { "Skip, start fresh" }}</button></div>
            }
            .into_any(),
            _ => {
                let did = imported.get().is_some();
                view! {
                    <div class="card stack">
                        <h2>"You're ready"</h2>
                        <p>"How Caddymium works:"</p>
                        <ol class="steps-list">
                            <li><strong>"Edit"</strong>" sites with forms (or raw Caddyfile text when you need to)."</li>
                            <li><strong>"Review"</strong>": Caddymium generates the Caddyfile, Caddy validates it, and you see a diff."</li>
                            <li><strong>"Apply"</strong>": it goes live. If Caddy isn't healthy afterwards, the previous config is restored automatically."</li>
                        </ol>
                        <div class="row wrap">
                            {if did {
                                view! { <button class="btn primary" on:click=move |_| finish("/changes")>"Review the imported config"</button> }.into_any()
                            } else {
                                view! { <button class="btn primary" on:click=move |_| finish("/sites/new")>"Add my first site"</button> }.into_any()
                            }}
                            <button class="btn" on:click=move |_| finish("/sites")>"Go to dashboard"</button>
                        </div>
                    </div>
                }
                .into_any()
            }
        }
    };
    let dot = move |n: u8, label: &'static str| view! { <span class:on=move || step.get() == n>{label}</span> };

    view! {
        <div class="stack welcome">
            <div>
                <h1>"Let's set up Caddymium"</h1>
                <p class="muted">"Two quick steps. Everything can be changed later in Settings."</p>
            </div>
            <div class="row wizard-steps">
                {dot(1, "1. Certificates")}<span class="faint">"→"</span>
                {dot(2, "2. Existing config")}<span class="faint">"→"</span>
                {dot(3, "3. Done")}
            </div>
            {body}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::b;

    #[test]
    fn finishing_marks_setup_done() {
        let s = finished(&json!({"acme_email": "a@b.c", "setup_complete": false}));
        assert!(b(&s, "setup_complete"));
        assert_eq!(s["acme_email"], "a@b.c");
        assert_eq!(CAS[0].0, "letsencrypt");
    }
}
