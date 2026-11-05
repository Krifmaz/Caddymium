//! Talks to Caddy through its stable interfaces only: the CLI (adapt,
//! validate, version, list-modules, build-info) and the admin API (/load,
//! /config, /reverse_proxy/upstreams, /metrics, /stop).

pub mod admin;
pub mod control;
pub mod runner;

pub use admin::Admin;
#[allow(unused_imports)]
pub use admin::{ApiError, Upstream};
#[allow(unused_imports)]
pub use runner::CaddyError;
pub use runner::{AdaptResult, Message, Runner};
