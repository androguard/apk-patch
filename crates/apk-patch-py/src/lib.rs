//! Python bindings for apk-patch (in-memory decode / build).

use std::collections::BTreeMap;

use apk_patch_core::{
    build_project_bytes, decode_apk_bytes, inject_goauld_bytes, BuildOptions, DecodeOptions,
    InjectGoauldOptions,
};
use apk_patch_vfs::MemVfs;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

fn map_err(e: impl ToString) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// Decode an APK into an in-memory project tree.
///
/// Returns a dict with keys:
/// - ``files``: ``dict[str, bytes]`` (project paths → content)
/// - ``project_root``: ``str`` (usually ``"project"``)
/// - ``entry_count`` / ``dex_class_count``: ``int``
#[pyfunction]
#[pyo3(signature = (apk_bytes, apk_name="app.apk", *, no_src=false, no_res=false, all_src=false, only_manifest=false))]
fn decode(
    py: Python<'_>,
    apk_bytes: &[u8],
    apk_name: &str,
    no_src: bool,
    no_res: bool,
    all_src: bool,
    only_manifest: bool,
) -> PyResult<PyObject> {
    let opts = DecodeOptions {
        force: true,
        no_src,
        no_res,
        all_src,
        only_manifest,
        ..DecodeOptions::default()
    };
    let result = decode_apk_bytes(apk_bytes, apk_name, &opts).map_err(map_err)?;
    let dict = PyDict::new_bound(py);
    dict.set_item("project_root", &result.project_root)?;
    dict.set_item("entry_count", result.entry_count)?;
    dict.set_item("dex_class_count", result.dex_class_count)?;
    let files = PyDict::new_bound(py);
    for (path, data) in result.vfs.files() {
        files.set_item(path, PyBytes::new_bound(py, data))?;
    }
    dict.set_item("files", files)?;
    Ok(dict.into_py(py))
}

/// Build a project tree (path → bytes) into an APK.
///
/// ``files`` must use the same layout produced by :func:`decode`
/// (paths under ``project_root``, default ``"project"``).
#[pyfunction]
#[pyo3(signature = (files, project_root="project", *, sign=true))]
fn build(
    files: &Bound<'_, PyDict>,
    project_root: &str,
    sign: bool,
) -> PyResult<Vec<u8>> {
    let mut map: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (k, v) in files.iter() {
        let path: String = k.extract()?;
        let data: Vec<u8> = v.extract()?;
        map.insert(path, data);
    }
    let mut vfs = MemVfs::from_files(map);
    let mut opts = BuildOptions {
        force: true,
        skip_aapt2: true,
        use_aapt2: false,
        rebuild_resources: true,
        ..BuildOptions::default()
    };
    opts.sign.enabled = sign;
    let result = build_project_bytes(&mut vfs, project_root, &opts).map_err(map_err)?;
    Ok(result.apk_bytes)
}

/// Decode → inject goauld agent ``.so`` + loader provider → rebuild.
///
/// ``agent_so`` is the raw bytes of ``libgoauld_agent.so`` (arm64-v8a).
#[pyfunction]
#[pyo3(signature = (apk_bytes, agent_so, apk_name="app.apk", *, sign=true))]
fn inject_goauld(
    apk_bytes: &[u8],
    agent_so: &[u8],
    apk_name: &str,
    sign: bool,
) -> PyResult<Vec<u8>> {
    let mut opts = InjectGoauldOptions::default();
    opts.sign.enabled = sign;
    opts.force = true;
    inject_goauld_bytes(apk_bytes, agent_so, apk_name, &opts).map_err(map_err)
}

#[pymodule]
fn apk_patch(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(decode, m)?)?;
    m.add_function(wrap_pyfunction!(build, m)?)?;
    m.add_function(wrap_pyfunction!(inject_goauld, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
