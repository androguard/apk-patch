//! Text AndroidManifest.xml patches for build-time options.

use apk_patch_meta::ApkToolMeta;

/// Build-time manifest mutations (SDK / version / debug / network security).
#[derive(Debug, Clone, Default)]
pub struct ManifestBuildPatch {
    pub debuggable: bool,
    pub net_sec_conf: bool,
    /// Attribute value for networkSecurityConfig (default `@xml/network_security_config`).
    pub net_sec_conf_ref: String,
}

impl ManifestBuildPatch {
    pub fn new() -> Self {
        Self {
            debuggable: false,
            net_sec_conf: false,
            net_sec_conf_ref: "@xml/network_security_config".into(),
        }
    }
}

/// Apply `apktool.yml` sdk/version info and CLI patches to text manifest XML.
pub fn patch_manifest_xml(xml: &str, meta: &ApkToolMeta, patch: &ManifestBuildPatch) -> String {
    let mut out = xml.to_string();

    if let Some(code) = meta.versionInfo.versionCode {
        out = set_manifest_attr(&out, "android:versionCode", &code.to_string());
    }
    if let Some(ref name) = meta.versionInfo.versionName {
        out = set_manifest_attr(&out, "android:versionName", name);
    }

    if meta.sdkInfo.minSdkVersion.is_some()
        || meta.sdkInfo.targetSdkVersion.is_some()
        || meta.sdkInfo.maxSdkVersion.is_some()
    {
        out = ensure_uses_sdk(
            &out,
            meta.sdkInfo.minSdkVersion.as_deref(),
            meta.sdkInfo.targetSdkVersion.as_deref(),
            meta.sdkInfo.maxSdkVersion.as_deref(),
        );
    }

    if patch.debuggable {
        out = set_application_attr(&out, "android:debuggable", "true");
    }

    if patch.net_sec_conf {
        out = set_application_attr(
            &out,
            "android:networkSecurityConfig",
            &patch.net_sec_conf_ref,
        );
    }

    out
}

/// Permissive network security config XML (cleartext + trust user CAs).
pub fn network_security_config_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="utf-8"?>
<network-security-config>
    <base-config cleartextTrafficPermitted="true">
        <trust-anchors>
            <certificates src="system" />
            <certificates src="user" />
        </trust-anchors>
    </base-config>
</network-security-config>
"#
}

fn set_manifest_attr(xml: &str, attr: &str, value: &str) -> String {
    set_tag_attr(xml, "manifest", attr, value)
}

fn set_application_attr(xml: &str, attr: &str, value: &str) -> String {
    set_tag_attr(xml, "application", attr, value)
}

/// Set or insert `attr="value"` on the first opening tag named `tag`.
fn set_tag_attr(xml: &str, tag: &str, attr: &str, value: &str) -> String {
    let open = format!("<{tag}");
    let Some(start) = xml.find(&open) else {
        return xml.to_string();
    };
    let after = start + open.len();
    let rest = &xml[after..];
    let end_rel = rest.find('>').unwrap_or(rest.len());
    let tag_body = &rest[..end_rel];
    let self_closing = tag_body.trim_end().ends_with('/');
    let body = if self_closing {
        tag_body.trim_end().trim_end_matches('/').trim_end()
    } else {
        tag_body
    };

    let new_body = if let Some(pos) = find_attr(body, attr) {
        replace_attr_value(body, pos, attr, value)
    } else {
        format!("{body} {attr}=\"{value}\"")
    };

    let mut out = String::with_capacity(xml.len() + value.len() + 16);
    out.push_str(&xml[..after]);
    out.push_str(&new_body);
    if self_closing {
        out.push_str(" /");
    }
    out.push_str(&xml[after + end_rel..]);
    out
}

fn find_attr(body: &str, attr: &str) -> Option<usize> {
    let needle = format!("{attr}=");
    body.find(&needle)
}

fn replace_attr_value(body: &str, pos: usize, attr: &str, value: &str) -> String {
    let after_eq = pos + attr.len() + 1;
    let bytes = body.as_bytes();
    if after_eq >= bytes.len() {
        return format!("{body}{attr}=\"{value}\"");
    }
    let quote = bytes[after_eq] as char;
    if quote != '"' && quote != '\'' {
        return format!("{} {}=\"{}\"", &body[..pos], attr, value);
    }
    let value_start = after_eq + 1;
    let Some(rel) = body[value_start..].find(quote) else {
        return body.to_string();
    };
    let value_end = value_start + rel;
    format!("{}{}{}", &body[..value_start], value, &body[value_end..])
}

/// Well-known ContentProvider class used by goauld inject.
pub const GOAULD_LOADER_PROVIDER: &str = "goauld.inject.LoaderProvider";

/// Extract the `package` attribute from the root `<manifest>` tag.
pub fn manifest_package(xml: &str) -> Option<String> {
    let open = "<manifest";
    let start = xml.find(open)?;
    let after = start + open.len();
    let rest = &xml[after..];
    let end_rel = rest.find('>')?;
    let body = &rest[..end_rel];
    let needle = "package=";
    let pos = body.find(needle)?;
    let after_eq = pos + needle.len();
    let bytes = body.as_bytes();
    if after_eq >= bytes.len() {
        return None;
    }
    let quote = bytes[after_eq] as char;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let value_start = after_eq + 1;
    let rel = body[value_start..].find(quote)?;
    Some(body[value_start..value_start + rel].to_string())
}

/// Insert `child_xml` before `</application>` if not already present.
///
/// Idempotent when `marker` (e.g. `android:name="goauld.inject.LoaderProvider"`) is already in `xml`.
pub fn insert_application_child(xml: &str, child_xml: &str, marker: &str) -> String {
    if xml.contains(marker) {
        return xml.to_string();
    }
    let close = "</application>";
    let Some(pos) = xml.find(close) else {
        return xml.to_string();
    };
    let indent = "        ";
    let mut out = String::with_capacity(xml.len() + child_xml.len() + indent.len() + 2);
    out.push_str(&xml[..pos]);
    if !xml[..pos].ends_with('\n') {
        out.push('\n');
    }
    out.push_str(indent);
    out.push_str(child_xml.trim());
    out.push('\n');
    out.push_str(&xml[pos..]);
    out
}

/// Insert the goauld LoaderProvider into `<application>` (idempotent).
pub fn insert_goauld_loader_provider(xml: &str) -> String {
    let package = manifest_package(xml).unwrap_or_else(|| "app".into());
    let authorities = format!("{package}.goauld.loader");
    let marker = format!("android:name=\"{GOAULD_LOADER_PROVIDER}\"");
    let child = format!(
        r#"<provider android:name="{GOAULD_LOADER_PROVIDER}" android:authorities="{authorities}" android:exported="false" android:initOrder="2147483647" />"#
    );
    insert_application_child(xml, &child, &marker)
}

fn ensure_uses_sdk(
    xml: &str,
    min: Option<&str>,
    target: Option<&str>,
    max: Option<&str>,
) -> String {
    if xml.contains("<uses-sdk") {
        let mut out = xml.to_string();
        if let Some(v) = min {
            out = set_tag_attr(&out, "uses-sdk", "android:minSdkVersion", v);
        }
        if let Some(v) = target {
            out = set_tag_attr(&out, "uses-sdk", "android:targetSdkVersion", v);
        }
        if let Some(v) = max {
            out = set_tag_attr(&out, "uses-sdk", "android:maxSdkVersion", v);
        }
        return out;
    }

    let mut attrs = String::new();
    if let Some(v) = min {
        attrs.push_str(&format!(" android:minSdkVersion=\"{v}\""));
    }
    if let Some(v) = target {
        attrs.push_str(&format!(" android:targetSdkVersion=\"{v}\""));
    }
    if let Some(v) = max {
        attrs.push_str(&format!(" android:maxSdkVersion=\"{v}\""));
    }
    let insert = format!("    <uses-sdk{attrs} />\n");

    // Insert after <manifest ...>
    if let Some(pos) = xml.find("<manifest") {
        if let Some(end) = xml[pos..].find('>') {
            let insert_at = pos + end + 1;
            let mut out = String::new();
            out.push_str(&xml[..insert_at]);
            out.push('\n');
            out.push_str(&insert);
            out.push_str(&xml[insert_at..]);
            return out;
        }
    }
    xml.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use apk_patch_meta::{ApkToolMeta, SdkInfo, VersionInfo};

    #[test]
    fn patches_version_and_debuggable() {
        let xml = r#"<?xml version="1.0"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="com.example">
    <application android:label="App">
    </application>
</manifest>
"#;
        let mut meta = ApkToolMeta::new("app.apk");
        meta.versionInfo = VersionInfo {
            versionCode: Some(42),
            versionName: Some("2.0".into()),
        };
        meta.sdkInfo = SdkInfo {
            minSdkVersion: Some("24".into()),
            targetSdkVersion: Some("34".into()),
            maxSdkVersion: None,
        };
        let mut patch = ManifestBuildPatch::new();
        patch.debuggable = true;
        let out = patch_manifest_xml(xml, &meta, &patch);
        assert!(out.contains("android:versionCode=\"42\""));
        assert!(out.contains("android:versionName=\"2.0\""));
        assert!(out.contains("android:minSdkVersion=\"24\""));
        assert!(out.contains("android:debuggable=\"true\""));
    }

    #[test]
    fn injects_net_sec_conf() {
        let xml = r#"<manifest package="p"><application></application></manifest>"#;
        let meta = ApkToolMeta::new("a.apk");
        let mut patch = ManifestBuildPatch::new();
        patch.net_sec_conf = true;
        let out = patch_manifest_xml(xml, &meta, &patch);
        assert!(out.contains("android:networkSecurityConfig=\"@xml/network_security_config\""));
    }

    #[test]
    fn parses_manifest_package() {
        let xml = r#"<?xml version="1.0"?><manifest package="com.example.app" android:versionCode="1">"#;
        assert_eq!(manifest_package(xml).as_deref(), Some("com.example.app"));
    }

    #[test]
    fn inserts_goauld_provider_idempotent() {
        let xml = r#"<?xml version="1.0"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="com.example">
    <application android:label="App">
        <activity android:name=".Main"/>
    </application>
</manifest>
"#;
        let once = insert_goauld_loader_provider(xml);
        assert!(once.contains("android:name=\"goauld.inject.LoaderProvider\""));
        assert!(once.contains("android:authorities=\"com.example.goauld.loader\""));
        assert!(once.contains("android:exported=\"false\""));
        assert!(once.contains("<activity android:name=\".Main\"/>"));
        let twice = insert_goauld_loader_provider(&once);
        assert_eq!(once.matches("goauld.inject.LoaderProvider").count(), 1);
        assert_eq!(twice, once);
    }
}
