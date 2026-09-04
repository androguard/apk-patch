//! Signing options and helpers for apk-patch builds.

use apkparser::{align_and_sign, sign_apk, KeystoreMaterial, SignError, SignOptions};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Sign(#[from] SignError),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Build-time signing configuration.
#[derive(Debug, Clone)]
pub struct BuildSignConfig {
    pub enabled: bool,
    pub v1: bool,
    pub v2: bool,
    pub v3: bool,
    pub keystore: Option<KeystoreMaterial>,
}

impl Default for BuildSignConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            // v1 PKCS#7 writer is still incomplete; v2+v3 are verified by apksigner
            // and suffice for Android 7+ installs.
            v1: false,
            v2: true,
            v3: true,
            keystore: None,
        }
    }
}

impl BuildSignConfig {
    pub fn to_sign_options(&self) -> SignOptions {
        SignOptions {
            v1: self.v1,
            v2: self.v2,
            v3: self.v3,
            keystore: self.keystore.clone().unwrap_or_else(|| {
                #[cfg(target_arch = "wasm32")]
                {
                    KeystoreMaterial::debug_ephemeral()
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    KeystoreMaterial::debug()
                }
            }),
        }
    }
}

pub fn sign_build_output(apk: &[u8], config: &BuildSignConfig) -> Result<Vec<u8>> {
    if !config.enabled {
        return Ok(apk.to_vec());
    }
    align_and_sign(apk, &config.to_sign_options()).map_err(Error::from)
}

pub fn sign_only(apk: &[u8], config: &BuildSignConfig) -> Result<Vec<u8>> {
    if !config.enabled {
        return Ok(apk.to_vec());
    }
    sign_apk(apk, &config.to_sign_options()).map_err(Error::from)
}
