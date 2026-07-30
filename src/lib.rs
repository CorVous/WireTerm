//! `WireTerm`'s maintained host foundation.
//!
//! Rendering ends at [`frame::PanelFrame`]. The serial-owning
//! [`host::HostBridge`] deliberately accepts only that display-ready type so
//! image imports, future Liquid/SVG playlist items, and other producers share
//! one transport path without being reprocessed.

pub mod app;
pub mod frame;
pub mod host;
pub mod raster;
pub mod transport;
