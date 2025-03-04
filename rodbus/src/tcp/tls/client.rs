use std::convert::TryFrom;
use std::io::{self, ErrorKind};
use std::path::Path;
use std::sync::Arc;

use tokio::net::TcpStream;
use tokio_rustls::{rustls, webpki};
use tracing::Instrument;

use crate::client::{Channel, ClientState, HostAddr, Listener, RetryStrategy};
use crate::common::phys::PhysLayer;
use crate::tcp::client::{TcpChannelTask, TcpTaskConnectionHandler};
use crate::tcp::tls::{load_certs, load_private_key, CertificateMode, MinTlsVersion, TlsError};

use crate::DecodeLevel;

/// TLS configuration
pub struct TlsClientConfig {
    dns_name: rustls::ServerName,
    config: Arc<rustls::ClientConfig>,
}

pub(crate) fn spawn_tls_channel(
    host: HostAddr,
    max_queued_requests: usize,
    connect_retry: Box<dyn RetryStrategy>,
    tls_config: TlsClientConfig,
    decode: DecodeLevel,
    listener: Box<dyn Listener<ClientState>>,
) -> Channel {
    let (handle, task) = create_tls_channel(
        host,
        max_queued_requests,
        connect_retry,
        tls_config,
        decode,
        listener,
    );
    tokio::spawn(task);
    handle
}

pub(crate) fn create_tls_channel(
    host: HostAddr,
    max_queued_requests: usize,
    connect_retry: Box<dyn RetryStrategy>,
    tls_config: TlsClientConfig,
    decode: DecodeLevel,
    listener: Box<dyn Listener<ClientState>>,
) -> (Channel, impl std::future::Future<Output = ()>) {
    let (tx, rx) = tokio::sync::mpsc::channel(max_queued_requests);
    let task = async move {
        TcpChannelTask::new(
            host.clone(),
            rx,
            TcpTaskConnectionHandler::Tls(tls_config),
            connect_retry,
            decode,
            listener,
        )
        .run()
        .instrument(tracing::info_span!("Modbus-Client-TCP", endpoint = ?host))
        .await;
    };
    (Channel { tx }, task)
}

impl TlsClientConfig {
    /// Create a TLS master config
    pub fn new(
        name: &str,
        peer_cert_path: &Path,
        local_cert_path: &Path,
        private_key_path: &Path,
        password: Option<&str>,
        min_tls_version: MinTlsVersion,
        certificate_mode: CertificateMode,
    ) -> Result<Self, TlsError> {
        let mut peer_certs = load_certs(peer_cert_path, false)?;
        let local_certs = load_certs(local_cert_path, true)?;
        let private_key = load_private_key(private_key_path, password)?;

        let builder = rustls::ClientConfig::builder()
            .with_safe_default_cipher_suites()
            .with_safe_default_kx_groups()
            .with_protocol_versions(min_tls_version.to_rustls())
            .map_err(|err| TlsError::BadConfig(err.to_string()))?;

        let config = match certificate_mode {
            CertificateMode::AuthorityBased => {
                // Build trust roots
                let mut root = Vec::with_capacity(peer_certs.len());
                for cert in &peer_certs {
                    let cert = OwnedTrustAnchor::try_from_cert_der(&cert.0).map_err(|err| {
                        TlsError::InvalidPeerCertificate(io::Error::new(
                            ErrorKind::InvalidData,
                            err.to_string(),
                        ))
                    })?;
                    root.push(cert);
                }

                builder
                    .with_custom_certificate_verifier(Arc::new(CommonNameServerCertVerifier::new(
                        root,
                        name.to_string(),
                    )))
                    .with_single_cert(local_certs, private_key)
            }
            CertificateMode::SelfSigned => {
                // Set the custom certificate verifier
                if let Some(peer_cert) = peer_certs.pop() {
                    if !peer_certs.is_empty() {
                        return Err(TlsError::InvalidPeerCertificate(io::Error::new(
                            ErrorKind::InvalidData,
                            "more than one peer certificate in self-signed mode",
                        )));
                    }

                    builder
                        .with_custom_certificate_verifier(Arc::new(
                            SelfSignedCertificateServerCertVerifier::new(peer_cert),
                        ))
                        .with_single_cert(local_certs, private_key)
                } else {
                    return Err(TlsError::InvalidPeerCertificate(io::Error::new(
                        ErrorKind::InvalidData,
                        "no peer certificate",
                    )));
                }
            }
        }
        .map_err(|err| {
            TlsError::InvalidLocalCertificate(io::Error::new(
                ErrorKind::InvalidData,
                err.to_string(),
            ))
        })?;

        let dns_name = rustls::ServerName::try_from(name).map_err(|_| TlsError::InvalidDnsName)?;

        Ok(Self {
            config: std::sync::Arc::new(config),
            dns_name,
        })
    }

    pub(crate) async fn handle_connection(
        &mut self,
        socket: TcpStream,
        endpoint: &HostAddr,
    ) -> Result<PhysLayer, String> {
        let connector = tokio_rustls::TlsConnector::from(self.config.clone());
        match connector.connect(self.dns_name.clone(), socket).await {
            Err(err) => Err(format!(
                "failed to establish TLS session with {}: {}",
                endpoint, err
            )),
            Ok(stream) => Ok(PhysLayer::new_tls(tokio_rustls::TlsStream::from(stream))),
        }
    }
}

struct CommonNameServerCertVerifier {
    roots: Vec<OwnedTrustAnchor>,
    server_name: String,
}

impl CommonNameServerCertVerifier {
    fn new(roots: Vec<OwnedTrustAnchor>, server_name: String) -> Self {
        Self { roots, server_name }
    }
}

impl rustls::client::ServerCertVerifier for CommonNameServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::Certificate,
        intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        now: std::time::SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        // Note: this code is taken from `WebPkiVerifier` in the `verifier` module of `rustls`

        // Verify trust chain using webpki
        let (cert, chain, trustroots) = prepare(end_entity, intermediates, &self.roots)?;
        let webpki_now =
            webpki::Time::try_from(now).map_err(|_| rustls::Error::FailedToGetCurrentTime)?;

        cert.verify_is_valid_tls_server_cert(
            SUPPORTED_SIG_ALGS,
            &webpki::TlsServerTrustAnchors(&trustroots),
            &chain,
            webpki_now,
        )
        .map_err(super::pki_error)
        .map(|_| cert)?;

        // Check DNS name (including in the Common Name)
        super::verify_dns_name(end_entity, &self.server_name)?;

        Ok(rustls::client::ServerCertVerified::assertion())
    }
}

type SignatureAlgorithms = &'static [&'static webpki::SignatureAlgorithm];

static SUPPORTED_SIG_ALGS: SignatureAlgorithms = &[
    &webpki::ECDSA_P256_SHA256,
    &webpki::ECDSA_P256_SHA384,
    &webpki::ECDSA_P384_SHA256,
    &webpki::ECDSA_P384_SHA384,
    &webpki::ED25519,
    &webpki::RSA_PSS_2048_8192_SHA256_LEGACY_KEY,
    &webpki::RSA_PSS_2048_8192_SHA384_LEGACY_KEY,
    &webpki::RSA_PSS_2048_8192_SHA512_LEGACY_KEY,
    &webpki::RSA_PKCS1_2048_8192_SHA256,
    &webpki::RSA_PKCS1_2048_8192_SHA384,
    &webpki::RSA_PKCS1_2048_8192_SHA512,
    &webpki::RSA_PKCS1_3072_8192_SHA384,
];

// TODO: if `rustls::OwnedTrustAnchor::to_trust_anchor` was public,
// we wouldn't need to duplicate this.
#[derive(Debug, Clone)]
struct OwnedTrustAnchor {
    subject: Vec<u8>,
    spki: Vec<u8>,
    name_constraints: Option<Vec<u8>>,
}

impl OwnedTrustAnchor {
    /// Get a `webpki::TrustAnchor` by borrowing the owned elements.
    fn to_trust_anchor(&self) -> webpki::TrustAnchor {
        webpki::TrustAnchor {
            subject: &self.subject,
            spki: &self.spki,
            name_constraints: self.name_constraints.as_deref(),
        }
    }

    fn try_from_cert_der(cert_der: &[u8]) -> Result<Self, webpki::Error> {
        let trust_anchor = webpki::TrustAnchor::try_from_cert_der(cert_der)?;

        Ok(Self {
            subject: trust_anchor.subject.to_owned(),
            spki: trust_anchor.spki.to_owned(),
            name_constraints: trust_anchor.name_constraints.map(|x| x.to_owned()),
        })
    }
}

type CertChainAndRoots<'a, 'b> = (
    webpki::EndEntityCert<'a>,
    Vec<&'a [u8]>,
    Vec<webpki::TrustAnchor<'b>>,
);

fn prepare<'a, 'b>(
    end_entity: &'a rustls::Certificate,
    intermediates: &'a [rustls::Certificate],
    roots: &'b [OwnedTrustAnchor],
) -> Result<CertChainAndRoots<'a, 'b>, rustls::Error> {
    // EE cert must appear first.
    let cert = webpki::EndEntityCert::try_from(end_entity.0.as_ref()).map_err(super::pki_error)?;

    let intermediates: Vec<&'a [u8]> = intermediates.iter().map(|cert| cert.0.as_ref()).collect();

    let trustroots: Vec<webpki::TrustAnchor> = roots
        .iter()
        .map(OwnedTrustAnchor::to_trust_anchor)
        .collect();

    Ok((cert, intermediates, trustroots))
}

struct SelfSignedCertificateServerCertVerifier {
    cert: rustls::Certificate,
}

impl SelfSignedCertificateServerCertVerifier {
    fn new(cert: rustls::Certificate) -> Self {
        Self { cert }
    }
}

impl rustls::client::ServerCertVerifier for SelfSignedCertificateServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::Certificate,
        intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        now: std::time::SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        // Check that no intermediate certificates are present
        if !intermediates.is_empty() {
            return Err(rustls::Error::General(format!(
                "client sent {} intermediate certificates, expected none",
                intermediates.len()
            )));
        }

        // Check that presented certificate matches byte-for-byte the expected certificate
        if end_entity != &self.cert {
            return Err(rustls::Error::InvalidCertificateData(
                "client certificate doesn't match the expected self-signed certificate".to_string(),
            ));
        }

        // Check that the certificate is still valid
        let parsed_cert = rx509::x509::Certificate::parse(&end_entity.0).map_err(|err| {
            rustls::Error::InvalidCertificateData(format!(
                "unable to parse cert with rasn: {:?}",
                err
            ))
        })?;

        let now = now
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| rustls::Error::FailedToGetCurrentTime)?;
        let now = rx509::der::UtcTime::from_seconds_since_epoch(now.as_secs());

        if !parsed_cert.tbs_certificate.value.validity.is_valid(now) {
            return Err(rustls::Error::InvalidCertificateData(
                "self-signed certificate is currently not valid".to_string(),
            ));
        }

        // We do not validate DNS name. Providing the exact same certificate is sufficient.

        Ok(rustls::client::ServerCertVerified::assertion())
    }
}
