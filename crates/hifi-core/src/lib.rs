//! Hi-Fi core domain model — tabs, groups, spaces, split trees, settings,
//! persistence and the IPC wire protocol shared by the app and the `hifi` CLI.
//!
//! This crate has no UI and no webview dependencies; it is portable to macOS,
//! Linux and Windows unchanged.

pub mod ipc;
pub mod models;
pub mod paths;
pub mod persistence;
pub mod schemes;

pub use ipc::{IpcRequest, IpcResponse};
pub use models::*;
pub use paths::HifiPaths;
pub use persistence::StateFile;
pub use schemes::{RoutedUrl, route};
