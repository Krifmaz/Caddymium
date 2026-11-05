//! `#/account`: the signed-in account's password, two-factor sign-in and this browser's theme.

use crate::app::{self, b, me, s};
use crate::ui::{ErrorBox, PageHead};
use kit_leptos::toasts::toast;
use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::{json, Value};

fn value(r: NodeRef<leptos::html::Input>) -> String {
    r.get().map(|i| i.value()).unwrap_or_default()
}

fn clear(rs: &[NodeRef<leptos::html::Input>]) {
    for r in rs {
        if let Some(i) = r.get() {
            i.set_value("");
        }
    }
}

/// Sets `user.totp_enabled` in the kept `/api/me`.
fn set_totp(on: bool) {
    let mut m = me().get_untracked();
    if let Some(u) = m.get_mut("user").and_then(Value::as_object_mut) {
        u.insert("totp_enabled".into(), Value::Bool(on));
    }
    app::set_me(m);
}

/// The password card.
#[component]
fn Password() -> impl IntoView {
    let (current, next, next2) = (NodeRef::new(), NodeRef::new(), NodeRef::new());
    let error = RwSignal::new(None::<String>);
    let submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        error.set(None);
        if value(next) != value(next2) {
            error.set(Some("The new passwords don't match. Type the same password twice.".into()));
            return;
        }
        let body = json!({ "current": value(current), "new": value(next) });
        rt::spawn(async move {
            match api::post("/me/password", body).await {
                Ok(_) => {
                    clear(&[current, next, next2]);
                    toast("Password changed. Your other sessions were signed out.", "ok");
                }
                Err(e) => error.set(Some(e.message)),
            }
        });
    };
    view! {
        <form class="card stack" on:submit=submit>
            <h2>"Change password"</h2>
            <label class="field"><span>"Current password"</span><input class="input" type="password" node_ref=current autocomplete="current-password" required/></label>
            <label class="field"><span>"New password"</span><input class="input" type="password" node_ref=next autocomplete="new-password" minlength="10" required/></label>
            <label class="field"><span>"Repeat new password"</span><input class="input" type="password" node_ref=next2 autocomplete="new-password" required/></label>
            {move || error.get().map(|e| view! { <ErrorBox message=e/> })}
            <div><button class="btn primary" type="submit">"Change password"</button></div>
        </form>
    }
}

/// The two-factor card: set up (QR code and key, then a code), or turn off with the password.
#[component]
fn TwoFactor() -> impl IntoView {
    let m = me();
    let on = Memo::new(move |_| m.get().get("user").is_some_and(|u| b(u, "totp_enabled")));
    let enroll = RwSignal::new(None::<Value>);
    let error = RwSignal::new(None::<String>);
    let (code, password) = (NodeRef::new(), NodeRef::new());
    let begin = move |_| {
        error.set(None);
        rt::spawn(async move {
            match api::post_empty("/me/totp/begin").await {
                Ok(v) => enroll.set(Some(v)),
                Err(e) => error.set(Some(e.message)),
            }
        });
    };
    let enable = move |_| {
        error.set(None);
        let c = value(code);
        rt::spawn(async move {
            match api::post("/me/totp/enable", json!({ "code": c })).await {
                Ok(_) => {
                    enroll.set(None);
                    set_totp(true);
                    toast("Two-factor sign-in is on", "ok");
                }
                Err(e) => error.set(Some(e.message)),
            }
        });
    };
    let disable = move |_| {
        error.set(None);
        let p = value(password);
        rt::spawn(async move {
            match api::post("/me/totp/disable", json!({ "password": p })).await {
                Ok(_) => {
                    set_totp(false);
                    toast("Two-factor sign-in is off", "ok");
                }
                Err(e) => error.set(Some(e.message)),
            }
        });
    };
    let body = move || {
        if on.get() {
            view! {
                <p class="muted" style="margin:0">"Signing in needs a code from your authenticator app. Enter your password to turn it off."</p>
                <div class="row wrap">
                    <input class="input" type="password" node_ref=password placeholder="Password" style="max-width:240px"/>
                    <button class="btn danger" on:click=disable>"Turn off"</button>
                </div>
            }
            .into_any()
        } else if let Some(e) = enroll.get() {
            view! {
                <p style="margin:0">"Scan this with an authenticator app (Aegis, 2FAS, Google Authenticator, 1Password…), then enter the 6-digit code."</p>
                <div class="qr" inner_html=s(&e, "qr_svg")></div>
                <p class="muted small" style="margin:0">"Can't scan? Enter this key by hand: " <code class="mono">{s(&e, "secret")}</code></p>
                <div class="row wrap">
                    <input class="input mono" node_ref=code inputmode="numeric" placeholder="123456" style="max-width:160px"/>
                    <button class="btn primary" on:click=enable>"Turn on"</button>
                </div>
            }
            .into_any()
        } else {
            view! {
                <p class="muted" style="margin:0">"Protect your account with a code from your phone in addition to your password."</p>
                <div><button class="btn" on:click=begin>"Set up two-factor"</button></div>
            }
            .into_any()
        }
    };
    view! {
        <div class="card stack">
            <h2 class="row">"Two-factor sign-in" {move || on.get().then(|| view! { <span class="badge ok">"on"</span> })}</h2>
            {body}
            {move || error.get().map(|e| view! { <ErrorBox message=e/> })}
        </div>
    }
}

/// The theme card: the kit's themes for this browser.
#[component]
fn Theme() -> impl IntoView {
    let chosen = RwSignal::new(app::theme());
    let buttons = app::THEMES
        .iter()
        .map(|(id, label)| {
            let on = move || chosen.get() == *id;
            view! {
                <button class:on=on on:click=move |_| { app::set_theme(id); chosen.set(id.to_string()); }>{*label}</button>
            }
        })
        .collect_view();
    view! {
        <div class="card stack">
            <h2>"Look"</h2>
            <p class="muted" style="margin:0">"The theme this browser shows. System follows your device's light or dark mode."</p>
            <div class="seg">{buttons}</div>
        </div>
    }
}

/// `#/account`.
#[component]
pub fn Account() -> impl IntoView {
    let m = me().get_untracked();
    let u = m.get("user").cloned().unwrap_or(Value::Null);
    let sub = format!("{} · {} · Caddymium {}", s(&u, "username"), s(&u, "role"), s(&m, "version"));
    view! {
        <PageHead title="Your account" text=sub/>
        <div class="stack mt" style="max-width:560px">
            <Password/>
            <TwoFactor/>
            <Theme/>
        </div>
    }
}
