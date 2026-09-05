//! Decode and build orchestration.

mod build;
mod build_vfs;
mod bytes_api;
mod container;
mod decode;
mod decode_vfs;
mod inject_goauld;
mod manifest;
mod manifest_patch;

pub use build::{BuildOptions, BuildResult};
#[cfg(feature = "native-fs")]
pub use build::build_project;
pub use build_vfs::build_project_vfs;
pub use bytes_api::{
    build_project_bytes, decode_apk_bytes, inject_goauld_bytes, BytesBuildResult, BytesDecodeResult,
};
pub use container::{looks_like_xapk, pack_split_container, SplitPackage};
pub use decode::{DecodeOptions, DecodeResult};
#[cfg(feature = "native-fs")]
pub use decode::decode_apk;
pub use decode_vfs::decode_apk_vfs;
pub use inject_goauld::{
    apply_goauld_inject_vfs, default_agent_so_path, InjectGoauldOptions, GOAULD_LOADER_DEX,
};
#[cfg(feature = "native-fs")]
pub use inject_goauld::{apply_goauld_inject, inject_goauld};
pub use apk_patch_resources::ResResolveMode;
pub use apk_patch_vfs::{export_tree, import_tree, MemVfs, Vfs};
#[cfg(feature = "native-fs")]
pub use apk_patch_vfs::StdFs;
pub use apk_patch_meta::{PackageFormat, SplitContainerMeta, CONTAINER_DIR};
pub use manifest_patch::{
    insert_goauld_loader_provider, ManifestBuildPatch, patch_manifest_xml, GOAULD_LOADER_PROVIDER,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
