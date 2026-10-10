//! MyBot 2.0's computer: the Docker desktop each conversation runs on, browser
//! control over CDP, security boundaries, the saved-login fill, and the native
//! live view (VNC).

pub mod boundary;
pub mod browser;
pub mod cdp;
pub mod container;
pub mod loginfill;
pub mod rfb;
