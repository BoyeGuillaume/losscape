//! ClickHouse connection and schema migrations.

use std::time::Duration;

use clickhouse::Client;
use hyper_util::client::legacy::Client as LegacyClient;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;

use crate::config::ClickHouseConfig;

/// (version, name, sql), applied in order and recorded in `schema_migrations`.
const MIGRATIONS: &[(u32, &str, &str)] = &[
    (
        1,
        "metrics",
        include_str!("../migrations/clickhouse/0001_metrics.sql"),
    ),
    (
        2,
        "console",
        include_str!("../migrations/clickhouse/0002_console.sql"),
    ),
];

/// Like the crate's own client (same keep-alive and pool timeout), but `https://` too, with
/// the certificate authority and client certificate of the settings.
fn client(config: &ClickHouseConfig) -> Result<Client, String> {
    let mut http = HttpConnector::new();
    http.set_keepalive(Some(Duration::from_secs(60)));
    http.enforce_http(false);
    let tls = config
        .tls
        .config()
        .map_err(|e| format!("ClickHouse TLS: {e}"))?;
    let connector = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls)
        .https_or_http()
        .enable_http1()
        .wrap_connector(http);
    // ClickHouse closes idle connections after 3 s.
    Ok(Client::with_http_client(
        LegacyClient::builder(TokioExecutor::new())
            .pool_idle_timeout(Duration::from_secs(2))
            .build(connector),
    ))
}

pub async fn connect_and_migrate(
    config: &ClickHouseConfig,
) -> Result<Client, Box<dyn std::error::Error + Send + Sync>> {
    if !config
        .database
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(format!("invalid ClickHouse database name {:?}", config.database).into());
    }
    let server = client(config)?
        .with_url(&config.url)
        .with_user(&config.user)
        .with_password(&config.password);
    server
        .query(&format!(
            "CREATE DATABASE IF NOT EXISTS `{}`",
            config.database
        ))
        .execute()
        .await?;

    let client = server.with_database(&config.database);
    client
        .query(
            "CREATE TABLE IF NOT EXISTS schema_migrations
             (version UInt32, name String, applied_at DateTime DEFAULT now())
             ENGINE = MergeTree ORDER BY version",
        )
        .execute()
        .await?;
    let applied: Vec<u32> = client
        .query("SELECT version FROM schema_migrations")
        .fetch_all()
        .await?;

    for &(version, name, sql) in MIGRATIONS {
        if applied.contains(&version) {
            continue;
        }
        client.query(sql).execute().await?;
        client
            .query("INSERT INTO schema_migrations (version, name) VALUES (?, ?)")
            .bind(version)
            .bind(name)
            .execute()
            .await?;
        tracing::info!(version, name, "applied ClickHouse migration");
    }
    Ok(client)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use hyper::service::service_fn;
    use rcgen::{CertificateParams, CertifiedIssuer, IsCa, KeyPair};
    use rustls::server::WebPkiClientVerifier;
    use rustls_pki_types::CertificateDer;
    use tokio::net::TcpListener;

    use super::*;
    use crate::tls::{self, CertFiles, ClientTls};

    /// A private CA, and from it a certificate for a server named localhost and one for a
    /// client: all written to a new folder.
    struct Pki {
        dir: std::path::PathBuf,
        ca: CertificateDer<'static>,
    }

    impl Pki {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("losscape-pki-{}", uuid::Uuid::now_v7()));
            std::fs::create_dir_all(&dir).unwrap();
            let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
            params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
            let ca = CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap();
            std::fs::write(dir.join("ca.pem"), ca.pem()).unwrap();
            for (name, sans) in [("server", vec!["localhost".to_owned()]), ("client", vec![])] {
                let key = KeyPair::generate().unwrap();
                let cert = CertificateParams::new(sans)
                    .unwrap()
                    .signed_by(&key, &ca)
                    .unwrap();
                std::fs::write(dir.join(format!("{name}.pem")), cert.pem()).unwrap();
                std::fs::write(dir.join(format!("{name}.key")), key.serialize_pem()).unwrap();
            }
            Self {
                ca: ca.der().clone(),
                dir,
            }
        }

        fn files(&self, name: &str) -> CertFiles {
            CertFiles {
                cert: self.dir.join(format!("{name}.pem")),
                key: self.dir.join(format!("{name}.key")),
            }
        }
    }

    impl Drop for Pki {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    /// Answers every request with 200, over HTTPS, to clients with a certificate from the CA.
    async fn mutual_tls_server(pki: &Pki) -> std::net::SocketAddr {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(pki.ca.clone()).unwrap();
        let verifier =
            WebPkiClientVerifier::builder_with_provider(Arc::new(roots), tls::provider())
                .build()
                .unwrap();
        let server = tls::load(&pki.files("server")).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(tls::provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_client_cert_verifier(verifier)
            .with_single_cert(server.cert, {
                let key = std::fs::read(pki.files("server").key).unwrap();
                rustls_pki_types::pem::PemObject::from_pem_slice(&key).unwrap()
            })
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let Ok(stream) = acceptor.accept(stream).await else {
                        return;
                    };
                    let ok = service_fn(|_| async {
                        Ok::<_, std::convert::Infallible>(hyper::Response::new(
                            axum::body::Body::empty(),
                        ))
                    });
                    hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                        .serve_connection(hyper_util::rt::TokioIo::new(stream), ok)
                        .await
                        .ok();
                });
            }
        });
        addr
    }

    fn config(port: u16, tls: ClientTls) -> ClickHouseConfig {
        ClickHouseConfig {
            url: format!("https://localhost:{port}"),
            user: "default".into(),
            password: String::new(),
            database: "losscape".into(),
            tls,
        }
    }

    #[tokio::test]
    async fn reaches_clickhouse_over_mutual_tls() {
        let pki = Pki::new();
        let port = mutual_tls_server(&pki).await.port();
        let ask = |tls: ClientTls| async move {
            client(&config(port, tls))
                .unwrap()
                .with_url(format!("https://localhost:{port}"))
                .query("SELECT 1")
                .execute()
                .await
        };
        let ca_cert = Some(pki.dir.join("ca.pem"));
        ask(ClientTls {
            ca_cert: ca_cert.clone(),
            identity: Some(pki.files("client")),
        })
        .await
        .unwrap();
        // Without a certificate: refused by the server.
        let refused = ask(ClientTls {
            ca_cert: ca_cert.clone(),
            identity: None,
        })
        .await;
        assert!(refused.is_err());
        // Without its CA: the server isn't trusted.
        let untrusted = ask(ClientTls {
            ca_cert: None,
            identity: Some(pki.files("client")),
        })
        .await;
        assert!(untrusted.is_err());
        // Its own key as a CA: not a certificate file.
        let bad = ClientTls {
            ca_cert: Some(pki.dir.join("client.key")),
            identity: None,
        };
        assert!(client(&config(port, bad)).is_err());
    }
}
