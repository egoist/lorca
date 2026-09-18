//! Lorca's Device core, and, with the `runner` feature, the Runner: keys, the relay sync,
//! jobs and rooms, questions to other Runners, and the JSON API the apps speak. The `lorca`
//! binary adds the local websocket server and the command line; the phone links the core
//! alone through `lorca-mobile`.

pub mod api;
pub mod app;
pub mod config;
pub mod credentials;
pub mod crypto;
pub mod events;
pub mod files;
pub mod identity;
pub mod keys;
pub mod memory;
pub mod model;
pub mod pairing;
pub mod plugins;
#[cfg(feature = "runner")]
pub mod providers;
pub mod push;
pub mod relay;
pub mod requests;
pub mod routines;
pub mod runtime;
pub mod schedule;
pub mod sync;
#[cfg(feature = "runner")]
pub mod turns;
#[cfg(feature = "server")]
pub mod ws;
