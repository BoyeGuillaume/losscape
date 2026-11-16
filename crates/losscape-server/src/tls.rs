//! TLS: serving HTTPS with a certificate of your own (`LOSSCAPE_TLS_CERT`, `LOSSCAPE_TLS_KEY`),
//! and the client side of encrypted connections to ClickHouse. All with ring, whatever other
//! providers the dependencies bring in.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, ServerConfig, SignatureScheme};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use tokio_rustls::TlsAcceptor;

/// How often the certificate files are checked for a renewed certificate.
const RELOAD_EVERY: Duration = Duration::from_secs(10);

pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// A certificate (with its chain) and its private key, both PEM files.
#[derive(Clone, Debug, PartialEq)]
pub struct CertFiles {
    pub cert: PathBuf,
    pub key: PathBuf,
}

fn read_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    let certs = CertificateDer::pem_file_iter(path)
        .and_then(|certs| certs.collect::<Result<Vec<_>, _>>())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if certs.is_empty() {
        return Err(format!("{}: no certificate in it (PEM)", path.display()));
    }
    Ok(certs)
}

fn read_key(path: &Path) -> Result<PrivateKeyDer<'static>, String> {
    PrivateKeyDer::from_pem_file(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Reads the files, checking that the key goes with the certificate.
pub fn load(files: &CertFiles) -> Result<CertifiedKey, String> {
    let certs = read_certs(&files.cert)?;
    let key = read_key(&files.key)?;
    CertifiedKey::from_der(certs, key, &provider())
        .map_err(|e| format!("{} and {}: {e}", files.cert.display(), files.key.display()))
}

/// The first certificate in the file: the server's own.
pub fn leaf(path: &Path) -> Result<CertificateDer<'static>, String> {
    Ok(read_certs(path)?.swap_remove(0))
}

/// The certificate being served, replaced when its files change.
#[derive(Debug)]
struct Reloading(RwLock<Arc<CertifiedKey>>);

impl ResolvesServerCert for Reloading {
    fn resolve(&self, _: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.read().expect("not poisoned").clone())
    }
}

fn stamp(files: &CertFiles) -> Option<[(SystemTime, u64); 2]> {
    let of = |path: &Path| {
        std::fs::metadata(path)
            .and_then(|m| Ok((m.modified()?, m.len())))
            .ok()
    };
    Some([of(&files.cert)?, of(&files.key)?])
}

/// Accepts TLS connections with the certificate in `files`, read again when they change (a
/// renewal needs no restart). Fails if they can't be read now.
pub fn acceptor(files: CertFiles) -> Result<TlsAcceptor, String> {
    let current = Arc::new(Reloading(RwLock::new(Arc::new(load(&files)?))));
    let mut config = ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_cert_resolver(current.clone());
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    let mut seen = stamp(&files);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(RELOAD_EVERY).await;
            let now = stamp(&files);
            if now.is_none() || now == seen {
                continue;
            }
            // Half-written files fail to load: tried again next time, the old one kept.
            match load(&files) {
                Ok(key) => {
                    *current.0.write().expect("not poisoned") = Arc::new(key);
                    seen = now;
                    tracing::info!(cert = %files.cert.display(), "TLS certificate reloaded");
                }
                Err(err) => tracing::warn!(%err, "TLS certificate changed, but can't be used yet"),
            }
        }
    });
    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// To reach this server itself (`healthcheck`): only its own certificate is accepted, by its
/// bytes, whatever name it is for and whoever signed it.
pub fn pinned_client(cert: CertificateDer<'static>) -> Result<ClientConfig, String> {
    let provider = provider();
    Ok(ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned { cert, provider }))
        .with_no_client_auth())
}

#[derive(Debug)]
struct Pinned {
    cert: CertificateDer<'static>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if end_entity.as_ref() == self.cert.as_ref() {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "not the certificate in LOSSCAPE_TLS_CERT".into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// How the server checks who it talks to, and proves who it is, on outgoing connections.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClientTls {
    /// Certificate authorities to trust, instead of the public ones: a private CA.
    pub ca_cert: Option<PathBuf>,
    /// A client certificate, for servers that ask for one (mutual TLS).
    pub identity: Option<CertFiles>,
}

impl ClientTls {
    /// A client configuration, checking the server's certificate always.
    pub fn config(&self) -> Result<ClientConfig, String> {
        let mut roots = RootCertStore::empty();
        match &self.ca_cert {
            Some(path) => {
                for cert in read_certs(path)? {
                    roots
                        .add(cert)
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                }
            }
            None => roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
        }
        let builder = ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(roots);
        match &self.identity {
            Some(files) => builder
                .with_client_auth_cert(read_certs(&files.cert)?, read_key(&files.key)?)
                .map_err(|e| format!("{} and {}: {e}", files.cert.display(), files.key.display())),
            None => Ok(builder.with_no_client_auth()),
        }
    }
}

/// What a plain HTTP request to the HTTPS port gets: told where to go, rather than a closed
/// connection to puzzle over.
pub const PLAIN_HTTP_ANSWER: &[u8] = b"HTTP/1.1 400 Bad Request\r\n\
Content-Type: text/plain; charset=utf-8\r\nContent-Length: 47\r\nConnection: close\r\n\r\n\
This server only speaks HTTPS: use https://...\n";

/// Whether a connection's first byte starts a TLS handshake.
pub fn is_handshake(first: u8) -> bool {
    first == 0x16
}
