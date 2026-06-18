//! CSP allowlist construction for the viewer webview.
//!
//! The packaged app loads over the `tauri://` protocol with the static CSP
//! from `tauri.conf.json`, whose `connect-src` allows only the default sync
//! server. To let projects sync against other servers, the registered and
//! explicitly-allowed sync servers are appended to `connect-src` at runtime
//! via `WebviewWindowBuilder::on_web_resource_request`.

use std::collections::HashMap;

use tauri::utils::config::{Csp, CspDirectiveSources};

/// Append each server in `extra_servers` to the `connect-src` directive of
/// `base_csp`, returning the augmented policy string. Sources already present
/// are not duplicated; every other directive is preserved unchanged.
pub fn augment_connect_src(base_csp: &str, extra_servers: &[String]) -> String {
    let mut directives: HashMap<String, CspDirectiveSources> =
        Csp::Policy(base_csp.to_string()).into();
    let connect = directives.entry("connect-src".to_string()).or_default();
    for server in extra_servers {
        if !connect.contains(server) {
            connect.push(server);
        }
    }
    Csp::from(directives).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str =
        "default-src 'self'; connect-src 'self' wss://sync.automerge.org; img-src 'self'";

    #[test]
    fn appends_extra_servers_to_connect_src() {
        let out = augment_connect_src(BASE, &["wss://my-server.example.com".into()]);
        assert!(out.contains("wss://sync.automerge.org"), "keeps default: {out}");
        assert!(out.contains("wss://my-server.example.com"), "adds extra: {out}");
    }

    #[test]
    fn preserves_other_directives() {
        let out = augment_connect_src(BASE, &["wss://my-server.example.com".into()]);
        assert!(out.contains("default-src 'self'"), "default-src kept: {out}");
        assert!(out.contains("img-src 'self'"), "img-src kept: {out}");
    }

    #[test]
    fn does_not_duplicate_existing_source() {
        let out = augment_connect_src(BASE, &["wss://sync.automerge.org".into()]);
        assert_eq!(
            out.matches("wss://sync.automerge.org").count(),
            1,
            "no duplicate default: {out}"
        );
    }

    #[test]
    fn empty_extra_servers_keeps_connect_src() {
        let out = augment_connect_src(BASE, &[]);
        assert!(out.contains("wss://sync.automerge.org"), "{out}");
        assert!(out.contains("connect-src"), "{out}");
    }
}
