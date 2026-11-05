//! Signing in: the first admin account (setup), username and password, then the two-factor code when the
//! account has one.

use crate::app::{self, b, me};
use crate::ui::ErrorBox;
use crate::APP;
use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::json;

/// What the form asks for.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Step {
    /// no account yet: create the first admin
    Setup,
    /// username and password
    Login,
    /// the authenticator's code
    TwoFactor,
}

impl Step {
    /// The step for a `/api/me` answer.
    pub fn of(m: &serde_json::Value) -> Step {
        if b(m, "pending_2fa") {
            Step::TwoFactor
        } else if b(m, "needs_setup") {
            Step::Setup
        } else {
            Step::Login
        }
    }
}

fn value(r: NodeRef<leptos::html::Input>) -> String {
    r.get().map(|i| i.value()).unwrap_or_default()
}

/// The sign-in page.
#[component]
pub fn SignIn() -> impl IntoView {
    let step = RwSignal::new(Step::of(&me().get_untracked()));
    let token_required = b(&me().get_untracked(), "setup_token_required");
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let (username, password, password2, token, code) = (NodeRef::new(), NodeRef::new(), NodeRef::new(), NodeRef::new(), NodeRef::new());
    let submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        error.set(None);
        let st = step.get_untracked();
        if st == Step::Setup && value(password) != value(password2) {
            error.set(Some("The passwords don't match. Type the same password twice.".into()));
            return;
        }
        busy.set(true);
        let (u, p, t, c) = (value(username), value(password), value(token), value(code));
        rt::spawn(async move {
            let r = match st {
                Step::TwoFactor => api::post("/login/totp", json!({ "code": c })).await,
                Step::Setup => api::post("/setup", json!({ "username": u, "password": p, "token": t })).await,
                Step::Login => api::post("/login", json!({ "username": u, "password": p })).await,
            };
            busy.set(false);
            match r {
                Ok(res) => {
                    let mut m = me().get_untracked();
                    if let (Some(o), Some(n)) = (m.as_object_mut(), res.as_object()) {
                        for (k, v) in n {
                            o.insert(k.clone(), v.clone());
                        }
                    }
                    app::set_me(m);
                    if b(&res, "pending_2fa") {
                        step.set(Step::TwoFactor);
                    } else {
                        rt::reload();
                    }
                }
                Err(ex) => error.set(Some(ex.message)),
            }
        });
    };
    let head = move || match step.get() {
        Step::TwoFactor => ("Two-factor code", "Enter the 6-digit code from your authenticator app."),
        Step::Setup => ("Create your admin account", "This account manages your Caddy server. You can add more people later."),
        Step::Login => ("Welcome back", "Sign in to manage your Caddy server."),
    };
    let fields = move || {
        match step.get() {
        Step::TwoFactor => view! {
            <label class="field"><span>"Code"</span>
                <input class="input mono" node_ref=code inputmode="numeric" autocomplete="one-time-code" placeholder="123 456" required autofocus/>
            </label>
        }
        .into_any(),
        Step::Setup => view! {
            {token_required.then(|| view! {
                <label class="field"><span>"Setup token"</span>
                    <input class="input" type="password" node_ref=token required/>
                    <small>"The value of CADDYMIUM_SETUP_TOKEN on your server."</small>
                </label>
            })}
            <label class="field"><span>"Username"</span><input class="input" node_ref=username autocomplete="username" required autofocus/></label>
            <label class="field"><span>"Password"</span>
                <input class="input" type="password" node_ref=password autocomplete="new-password" minlength="10" required/>
                <small>"At least 10 characters. A few random words work well."</small>
            </label>
            <label class="field"><span>"Repeat password"</span><input class="input" type="password" node_ref=password2 autocomplete="new-password" required/></label>
        }
        .into_any(),
        Step::Login => view! {
            <label class="field"><span>"Username"</span><input class="input" node_ref=username autocomplete="username" required autofocus/></label>
            <label class="field"><span>"Password"</span><input class="input" type="password" node_ref=password autocomplete="current-password" required/></label>
        }
        .into_any(),
    }
    };
    let label = move || match (busy.get(), step.get()) {
        (true, _) => "Please wait…",
        (_, Step::TwoFactor) => "Verify",
        (_, Step::Setup) => "Create account",
        (_, Step::Login) => "Sign in",
    };
    view! {
        <div class="auth-wrap">
            <div class="auth-card">
                <div class="brand"><img src="favicon.svg" alt=""/>{APP}</div>
                <h2 style="text-align:center;margin-top:8px">{move || head().0}</h2>
                <p class="muted" style="text-align:center;margin:6px 0 0">{move || head().1}</p>
                <form autocomplete="on" on:submit=submit>
                    {fields}
                    {move || error.get().map(|e| view! { <ErrorBox message=e/> })}
                    <button class="btn primary" type="submit" style="padding:12px" disabled=move || busy.get()>{label}</button>
                </form>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::Step;
    use serde_json::json;

    #[test]
    fn the_step_follows_the_server() {
        assert_eq!(Step::of(&json!({"needs_setup": true})), Step::Setup);
        assert_eq!(Step::of(&json!({"needs_setup": false})), Step::Login);
        assert_eq!(Step::of(&json!({"pending_2fa": true, "needs_setup": false})), Step::TwoFactor);
    }
}
