//! The Breakless service: live Twitch channels with no advert breaks, as HLS playlists.
//!
//! The routes and their answers are those of the Python prototype
//! (`reference/python/server.py`, whose docstring is the contract). The binary in `main.rs` is a
//! thin wrapper; the tests build [`app::App`] and [`routes::router`] directly against a fake Twitch.

pub mod app;
pub mod channel;
pub mod chat;
pub mod config;
pub mod routes;
pub mod store;
pub mod twitch;
