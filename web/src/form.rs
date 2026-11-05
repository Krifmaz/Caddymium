//! Form fields bound to a JSON value in a signal, by a dotted path (`config.proxy.upstreams`): what the panel's
//! editors are made of.

use leptos::prelude::*;
use serde_json::{Map, Value};

/// The value at `path` (`a.b.c`, `list.0.name` for array items) in `v`, Null when missing.
pub fn get_path(v: &Value, path: &str) -> Value {
    let mut cur = v;
    for k in path.split('.') {
        let next = match (cur, k.parse::<usize>()) {
            (Value::Array(l), Ok(n)) => l.get(n),
            _ => cur.get(k),
        };
        match next {
            Some(x) => cur = x,
            None => return Value::Null,
        }
    }
    cur.clone()
}

/// Sets `path` in `v` to `x`, making the objects on the way.
pub fn set_path(v: &mut Value, path: &str, x: Value) {
    let mut cur = v;
    let keys: Vec<&str> = path.split('.').collect();
    for (n, k) in keys.iter().enumerate() {
        if let (Value::Array(l), Ok(at)) = (&*cur, k.parse::<usize>()) {
            if at < l.len() {
                let item = &mut cur.as_array_mut().expect("an array")[at];
                if n == keys.len() - 1 {
                    *item = x;
                    return;
                }
                cur = item;
                continue;
            }
        }
        if !cur.is_object() {
            *cur = Value::Object(Map::new());
        }
        let o = cur.as_object_mut().expect("an object");
        if n == keys.len() - 1 {
            o.insert(k.to_string(), x);
            return;
        }
        cur = o.entry(k.to_string()).or_insert_with(|| Value::Object(Map::new()));
    }
}

/// The string at `path`, "" when missing (numbers as text).
pub fn str_at(v: &Value, path: &str) -> String {
    match get_path(v, path) {
        Value::String(s) => s,
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// A text field.
#[component]
pub fn Text(
    state: RwSignal<Value>,
    #[prop(into)] path: String,
    #[prop(into)] label: String,
    #[prop(optional, into)] hint: String,
    #[prop(optional, into)] placeholder: String,
    #[prop(optional, into)] kind: Option<String>,
    #[prop(optional)] mono: bool,
    #[prop(optional)] disabled: bool,
) -> impl IntoView {
    let p = path.clone();
    let value = move || str_at(&state.get(), &p);
    let kind = kind.unwrap_or_else(|| "text".into());
    let number = kind == "number";
    let on_input = move |e: leptos::ev::Event| {
        let t = event_target_value(&e);
        let v = if number { t.parse::<i64>().map(Value::from).unwrap_or(Value::from(0)) } else { Value::String(t) };
        state.update(|s| set_path(s, &path, v));
    };
    view! {
        <label class="field">
            <span>{label}</span>
            <input class="input" class:mono=mono type=kind placeholder=placeholder prop:value=value on:input=on_input disabled=disabled
                autocomplete="off"/>
            {(!hint.is_empty()).then(|| view! { <small>{hint}</small> })}
        </label>
    }
}

/// A text area.
#[component]
pub fn Area(
    state: RwSignal<Value>,
    #[prop(into)] path: String,
    #[prop(into)] label: String,
    #[prop(optional, into)] hint: String,
    #[prop(optional, into)] placeholder: String,
    #[prop(optional)] rows: Option<u32>,
) -> impl IntoView {
    let p = path.clone();
    let value = move || str_at(&state.get(), &p);
    let on_input = move |e: leptos::ev::Event| {
        let t = event_target_value(&e);
        state.update(|s| set_path(s, &path, Value::String(t)));
    };
    view! {
        <label class="field">
            <span>{label}</span>
            <textarea class="input mono" rows=rows.unwrap_or(6) placeholder=placeholder prop:value=value on:input=on_input spellcheck="false"></textarea>
            {(!hint.is_empty()).then(|| view! { <small>{hint}</small> })}
        </label>
    }
}

/// A select of (value, label) pairs.
#[component]
pub fn Select(
    state: RwSignal<Value>,
    #[prop(into)] path: String,
    #[prop(into)] label: String,
    options: Vec<(String, String)>,
    #[prop(optional, into)] hint: String,
) -> impl IntoView {
    let p = path.clone();
    let on_change = move |e: leptos::ev::Event| {
        let t = event_target_value(&e);
        state.update(|s| set_path(s, &path, Value::String(t)));
    };
    let opts = options
        .into_iter()
        .map(|(v, l)| {
            let (vv, p) = (v.clone(), p.clone());
            view! { <option value=v selected=move || str_at(&state.get(), &p) == vv>{l}</option> }
        })
        .collect_view();
    view! {
        <label class="field">
            <span>{label}</span>
            <select class="input" on:change=on_change>{opts}</select>
            {(!hint.is_empty()).then(|| view! { <small>{hint}</small> })}
        </label>
    }
}

/// A switch for a bool.
#[component]
pub fn Check(
    state: RwSignal<Value>,
    #[prop(into)] path: String,
    #[prop(into)] label: String,
    #[prop(optional, into)] hint: String,
) -> impl IntoView {
    let p = path.clone();
    let on = move || get_path(&state.get(), &p).as_bool().unwrap_or(false);
    let on_change = move |e: leptos::ev::Event| {
        let c = event_target_checked(&e);
        state.update(|s| set_path(s, &path, Value::Bool(c)));
    };
    view! {
        <div class="field">
            <label class="switch">
                <input type="checkbox" prop:checked=on on:change=on_change/>
                <i></i>
                <span>{label}</span>
            </label>
            {(!hint.is_empty()).then(|| view! { <small>{hint}</small> })}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn paths_read_and_write() {
        let mut v = json!({"a": {"b": 1}});
        assert_eq!(get_path(&v, "a.b"), json!(1));
        assert_eq!(get_path(&v, "a.x.y"), Value::Null);
        set_path(&mut v, "a.c.d", json!("x"));
        assert_eq!(str_at(&v, "a.c.d"), "x");
        assert_eq!(str_at(&v, "a.b"), "1");
        set_path(&mut v, "top", json!(true));
        let mut l = json!({"list": [{"a": 1}, {"a": 2}]});
        set_path(&mut l, "list.1.a", json!(5));
        assert_eq!(get_path(&l, "list.1.a"), json!(5));
        assert_eq!(get_path(&l, "list.0.a"), json!(1));
        assert_eq!(v["top"], json!(true));
    }
}
