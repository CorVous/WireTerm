//! `WireTerm`'s maintained host foundation.
//!
//! Rendering ends at [`frame::PanelFrame`]. The serial-owning
//! [`host::HostBridge`] deliberately accepts only that display-ready type so
//! image imports, Lua/SVG Playlist Items, and other producers share
//! one transport path without being reprocessed.

pub mod app;
pub mod extension;
pub mod frame;
pub mod host;
pub mod playback;
pub mod playlist;
pub mod raster;
pub mod secrets;
pub mod transport;
