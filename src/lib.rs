#![warn(clippy::unwrap_used)]

pub mod action;
pub mod app;
pub mod cli;
pub mod clipboard;
pub mod cmd;
pub mod config;
pub mod config_watcher;
pub mod core;
pub(crate) mod inbox;
pub mod interactive;
pub mod keybinds;
pub(crate) mod nl;
pub(crate) mod note;
pub mod recurrence;
pub mod sample;
pub(crate) mod search;
pub(crate) mod serve;
pub mod theme;
pub(crate) mod threshold;
pub mod todo;
pub mod ui;
pub mod update;
pub(crate) mod xdg;
