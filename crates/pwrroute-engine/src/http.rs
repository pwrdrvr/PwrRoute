//! The real [`Network`]: one HTTP request with ureq over rustls, verified by
//! the OS's certificate store, no redirects followed (the engine follows
//! them), no cookies, and the proxy from the environment.

use std::{sync::Arc, time::Duration};

use anyhow::Result;
use ureq::{
    Agent,
    tls::{RootCerts, TlsConfig, TlsProvider},
};

use crate::shortlinks::{Method, Network, Reply};

/// Expands short links over the network. Cheap to clone; safe to share
/// between threads.
#[derive(Clone)]
pub struct HttpNetwork {
    agent: Agent,
}

impl std::fmt::Debug for HttpNetwork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpNetwork").finish_non_exhaustive()
    }
}

impl Default for HttpNetwork {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpNetwork {
    pub fn new() -> Self {
        let tls = TlsConfig::builder()
            .provider(TlsProvider::Rustls)
            .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        let agent = Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .save_redirect_history(false)
            .user_agent(concat!("PwrRoute/", env!("CARGO_PKG_VERSION")))
            .tls_config(tls)
            .build()
            .new_agent();
        Self { agent }
    }
}

impl Network for HttpNetwork {
    fn request(&self, method: Method, url: &str, timeout: Duration) -> Result<Reply> {
        let response = match method {
            Method::Head => self
                .agent
                .head(url)
                .config()
                .timeout_global(Some(timeout))
                .build()
                .call()?,
            // The body is never read; dropping the response closes it.
            Method::Get => self
                .agent
                .get(url)
                .config()
                .timeout_global(Some(timeout))
                .build()
                .call()?,
        };
        let location = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        Ok(Reply {
            status: response.status().as_u16(),
            location,
        })
    }
}
