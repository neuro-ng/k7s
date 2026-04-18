//! k7s library crate.
//!
//! Exposes all public modules so benchmarks, integration tests, and external
//! tooling can `use k7s::…` without going through the binary entry point.
//!
//! The binary (`src/main.rs`) keeps its own `mod` declarations for the
//! application loop; this file mirrors them for the library target.

#![allow(dead_code, unused_imports)]

pub mod ai;
pub mod bench;
pub mod metrics;
pub mod client;
pub mod config;
pub mod dao;
pub mod error;
pub mod exec;
pub mod health;
pub mod history;
pub mod model;
pub mod portforward;
pub mod render;
pub mod sanitizer;
pub mod ui;
pub mod util;
pub mod view;
pub mod vul;
pub mod watch;
