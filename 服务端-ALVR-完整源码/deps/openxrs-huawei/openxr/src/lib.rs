//! Compatibility shim for the private `openxrs-huawei` fork.
//!
//! The ALVR Android client depends on this crate through `path = "../../deps/openxrs-huawei/openxr"`.
//! The private fork is not public, so this shim re-exports the upstream `openxr` 0.18 API and maps
//! the Huawei feature names to the standard OpenXR loader.

pub use openxr_upstream::*;

/// The fork exposes `openxr::sys`; upstream re-exports individual items. Re-export the whole
/// `openxr-sys` crate so client code using `openxr::sys::*` compiles unchanged.
pub mod sys {
    pub use openxr_sys::*;
}
