//! Decode and build orchestration.

mod build;
mod decode;
mod inject_goauld;
mod manifest;
mod manifest_patch;

pub use build::{build_project, BuildOptions, BuildResult};
pub use decode::{decode_apk, DecodeOptions, DecodeResult};
pub use inject_goauld::{
    apply_goauld_inject, default_agent_so_path, inject_goauld, InjectGoauldOptions, GOAULD_LOADER_DEX,
};
pub use apk_patch_resources::ResResolveMode;
pub use manifest_patch::{
    insert_goauld_loader_provider, ManifestBuildPatch, patch_manifest_xml, GOAULD_LOADER_PROVIDER,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
