//! Decode and build orchestration.

mod build;
mod build_vfs;
mod bytes_api;
mod decode;
mod decode_vfs;
mod inject_goauld;
mod manifest;
mod manifest_patch;

pub use build::{build_project, BuildOptions, BuildResult};
pub use build_vfs::build_project_vfs;
pub use bytes_api::{
    build_project_bytes, decode_apk_bytes, inject_goauld_bytes, BytesBuildResult, BytesDecodeResult,
};
pub use decode::{decode_apk, DecodeOptions, DecodeResult};
pub use decode_vfs::decode_apk_vfs;
pub use inject_goauld::{
    apply_goauld_inject, apply_goauld_inject_vfs, default_agent_so_path, inject_goauld,
    InjectGoauldOptions, GOAULD_LOADER_DEX,
};
pub use apk_patch_resources::ResResolveMode;
pub use apk_patch_vfs::{export_tree, import_tree, MemVfs, Vfs};
#[cfg(feature = "native-fs")]
pub use apk_patch_vfs::StdFs;
pub use manifest_patch::{
    insert_goauld_loader_provider, ManifestBuildPatch, patch_manifest_xml, GOAULD_LOADER_PROVIDER,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
