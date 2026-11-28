//! End-to-end device authentication over an arbitrary (possibly relayed) byte stream.
//!
//! RFC 7250 carries the existing iroh Ed25519 public key as a standard DER SPKI.
//! Both endpoints prove possession in a fresh TLS 1.3 handshake. The acceptor
//! authenticates any valid device key; the caller must authorize the returned
//! identity separately, before processing business requests.

use std::{sync::Arc, time::Duration};

use iroh::{PublicKey, SecretKey};
use rustls::{
    CertificateError, DigitallySignedStruct, DistinguishedName, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{WebPkiSupportedAlgorithms, verify_tls13_signature_with_raw_key},
    pki_types::{CertificateDer, ServerName, SubjectPublicKeyInfoDer, UnixTime, alg_id},
    server::danger::{ClientCertVerified, ClientCertVerifier},
    sign::{CertifiedKey, Signer, SigningKey, public_key_to_spki},
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::{RemoteError, Result};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
// The pin, rather than a DNS name or the relay's identity, authenticates the server.
const SERVER_NAME: &str = "velune-tunnel.invalid";
const SIGNATURE_ALGORITHMS: WebPkiSupportedAlgorithms = WebPkiSupportedAlgorithms {
    all: &[&IrohEd25519],
    mapping: &[(SignatureScheme::ED25519, &[&IrohEd25519])],
};

/// Authenticate the destination key, independently of every forwarding device.
/// Dropping this future or reaching the handshake deadline drops the owned IO.
pub(crate) async fn connect<T>(
    key: &SecretKey,
    expected_peer: &str,
    io: T,
) -> Result<tokio_rustls::client::TlsStream<T>>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    let expected_peer = expected_peer
        .parse::<PublicKey>()
        .map_err(|_| RemoteError::Configuration)?;
    let connector = TlsConnector::from(Arc::new(client_config(key, expected_peer)?));
    let name = ServerName::try_from(SERVER_NAME).map_err(|_| RemoteError::Configuration)?;
    tokio::time::timeout(HANDSHAKE_TIMEOUT, connector.connect(name, io))
        .await
        .map_err(|_| RemoteError::Timeout)?
        .map_err(handshake_error)
}

/// Authenticate the caller and return its canonical iroh device ID from TLS.
/// This establishes identity only; it does not grant business authorization.
pub(crate) async fn accept<T>(
    key: &SecretKey,
    io: T,
) -> Result<(tokio_rustls::server::TlsStream<T>, String)>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    let acceptor = TlsAcceptor::from(Arc::new(server_config(key)?));
    let stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(io))
        .await
        .map_err(|_| RemoteError::Timeout)?
        .map_err(handshake_error)?;
    // Read only the identity retained by the completed, mandatory-auth handshake.
    let certificates = stream
        .get_ref()
        .1
        .peer_certificates()
        .ok_or(RemoteError::PeerMismatch)?;
    let [certificate] = certificates else {
        return Err(RemoteError::PeerMismatch);
    };
    let peer = public_key(certificate).map_err(|_| RemoteError::PeerMismatch)?;
    Ok((stream, peer.to_string()))
}

fn client_config(key: &SecretKey, expected_peer: PublicKey) -> Result<rustls::ClientConfig> {
    let mut config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .map_err(|_| RemoteError::Configuration)?
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(PinnedServer(expected_peer)))
    .with_client_cert_resolver(Arc::new(RawPublicKeyResolver::new(key)));
    config.enable_sni = false;
    config.enable_early_data = false;
    config.resumption = rustls::client::Resumption::disabled();
    Ok(config)
}

fn server_config(key: &SecretKey) -> Result<rustls::ServerConfig> {
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .map_err(|_| RemoteError::Configuration)?
    .with_client_cert_verifier(Arc::new(AuthenticatedClient))
    .with_cert_resolver(Arc::new(RawPublicKeyResolver::new(key)));
    config.max_early_data_size = 0;
    config.send_half_rtt_data = false;
    config.send_tls13_tickets = 0;
    config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    // The builder's default ticketer is disabled; do not install a ticket producer.
    Ok(config)
}

fn handshake_error(error: std::io::Error) -> RemoteError {
    if error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<rustls::Error>())
        .is_some()
    {
        RemoteError::PeerMismatch
    } else {
        RemoteError::Unavailable
    }
}

fn invalid_key() -> rustls::Error {
    rustls::Error::InvalidCertificate(CertificateError::BadEncoding)
}

fn spki(key: &PublicKey) -> SubjectPublicKeyInfoDer<'static> {
    public_key_to_spki(&alg_id::ED25519, key.as_bytes())
}

fn public_key(certificate: &CertificateDer<'_>) -> std::result::Result<PublicKey, rustls::Error> {
    let bytes = certificate.as_ref();
    let raw = bytes.last_chunk::<32>().ok_or_else(invalid_key)?;
    let key = PublicKey::from_bytes(raw).map_err(|_| invalid_key())?;
    // Re-encoding with rustls validates the entire standard Ed25519 SPKI, including
    // algorithm, lengths and padding. No unchecked fixed-offset DER decoding.
    if spki(&key).as_ref() != bytes {
        return Err(invalid_key());
    }
    Ok(key)
}

fn check_certificate(
    certificate: &CertificateDer<'_>,
    intermediates: &[CertificateDer<'_>],
) -> std::result::Result<PublicKey, rustls::Error> {
    if !intermediates.is_empty() {
        return Err(invalid_key());
    }
    public_key(certificate)
}

fn verify_signature(
    message: &[u8],
    certificate: &CertificateDer<'_>,
    signature: &DigitallySignedStruct,
) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
    verify_tls13_signature_with_raw_key(
        message,
        &SubjectPublicKeyInfoDer::from(certificate.as_ref()),
        signature,
        &SIGNATURE_ALGORITHMS,
    )
}

fn reject_tls12() -> rustls::Error {
    rustls::Error::PeerIncompatible(rustls::PeerIncompatible::Tls12NotOffered)
}

#[derive(Debug)]
struct PinnedServer(PublicKey);

impl ServerCertVerifier for PinnedServer {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if check_certificate(end_entity, intermediates)? != self.0 {
            return Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(reject_tls12())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        SIGNATURE_ALGORITHMS.supported_schemes()
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

#[derive(Debug)]
struct AuthenticatedClient;

impl ClientCertVerifier for AuthenticatedClient {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        check_certificate(end_entity, intermediates)?;
        // Proof of possession follows in verify_tls13_signature, before accept returns.
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(reject_tls12())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        SIGNATURE_ALGORITHMS.supported_schemes()
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

#[derive(Debug)]
struct RawPublicKeyResolver(Arc<CertifiedKey>);

impl RawPublicKeyResolver {
    fn new(key: &SecretKey) -> Self {
        Self(Arc::new(CertifiedKey::new(
            vec![CertificateDer::from(spki(&key.public()).as_ref().to_vec())],
            Arc::new(IrohSigningKey(key.clone())),
        )))
    }
}

impl rustls::client::ResolvesClientCert for RawPublicKeyResolver {
    fn resolve(
        &self,
        _root_hint_subjects: &[&[u8]],
        sigschemes: &[SignatureScheme],
    ) -> Option<Arc<CertifiedKey>> {
        sigschemes
            .contains(&SignatureScheme::ED25519)
            .then(|| self.0.clone())
    }

    fn has_certs(&self) -> bool {
        true
    }

    fn only_raw_public_keys(&self) -> bool {
        true
    }
}

impl rustls::server::ResolvesServerCert for RawPublicKeyResolver {
    fn resolve(&self, hello: rustls::server::ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        hello
            .signature_schemes()
            .contains(&SignatureScheme::ED25519)
            .then(|| self.0.clone())
    }

    fn only_raw_public_keys(&self) -> bool {
        true
    }
}

// iroh's SecretKey Debug implementation redacts its contents.
#[derive(Debug, Clone)]
struct IrohSigningKey(SecretKey);

impl SigningKey for IrohSigningKey {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
        offered
            .contains(&SignatureScheme::ED25519)
            .then(|| Box::new(self.clone()) as Box<dyn Signer>)
    }

    fn algorithm(&self) -> rustls::SignatureAlgorithm {
        rustls::SignatureAlgorithm::ED25519
    }

    fn public_key(&self) -> Option<SubjectPublicKeyInfoDer<'_>> {
        Some(spki(&self.0.public()))
    }
}

impl Signer for IrohSigningKey {
    fn sign(&self, message: &[u8]) -> std::result::Result<Vec<u8>, rustls::Error> {
        Ok(self.0.sign(message).to_bytes().to_vec())
    }

    fn scheme(&self) -> SignatureScheme {
        SignatureScheme::ED25519
    }
}

/// The same rustls/webpki adapter as iroh: delegate Ed25519 to the device key API.
#[derive(Debug)]
struct IrohEd25519;

impl rustls::pki_types::SignatureVerificationAlgorithm for IrohEd25519 {
    fn verify_signature(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> std::result::Result<(), rustls::pki_types::InvalidSignature> {
        use rustls::pki_types::InvalidSignature;
        let key = PublicKey::try_from(public_key).map_err(|_| InvalidSignature)?;
        let signature = iroh::Signature::try_from(signature).map_err(|_| InvalidSignature)?;
        key.verify(message, &signature)
            .map_err(|_| InvalidSignature)
    }

    fn public_key_alg_id(&self) -> rustls::pki_types::AlgorithmIdentifier {
        alg_id::ED25519
    }

    fn signature_alg_id(&self) -> rustls::pki_types::AlgorithmIdentifier {
        alg_id::ED25519
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::future::join;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

    fn run(test: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(async {
                tokio::time::timeout(HANDSHAKE_TIMEOUT + Duration::from_secs(5), test)
                    .await
                    .expect("test and all owned IO must finish within the deadline");
            });
    }

    fn key(byte: u8) -> SecretKey {
        SecretKey::from_bytes(&[byte; 32])
    }

    #[test]
    fn identities_survive_a_byte_only_relay() {
        run(async {
            let caller = key(1);
            let target = key(3);
            let target_id = target.public().to_string();
            let (a, mut b_in) = duplex(256);
            let (mut b_out, c) = duplex(256);
            let endpoints = async {
                let client = async {
                    let mut tls = connect(&caller, &target_id, a).await.unwrap();
                    let session = tls.get_ref().1;
                    assert_eq!(
                        session.protocol_version(),
                        Some(rustls::ProtocolVersion::TLSv1_3)
                    );
                    assert_eq!(session.handshake_kind(), Some(rustls::HandshakeKind::Full));
                    assert_eq!(
                        public_key(&session.peer_certificates().unwrap()[0]).unwrap(),
                        target.public()
                    );
                    // A self-reported identity in the body cannot change the TLS caller.
                    tls.write_all(b"claimed caller: B").await.unwrap();
                    tls.flush().await.unwrap();
                    let mut response = [0; 2];
                    tls.read_exact(&mut response).await.unwrap();
                    assert_eq!(&response, b"ok");
                    tls.shutdown().await.unwrap();
                    let mut remaining = Vec::new();
                    tls.read_to_end(&mut remaining).await.unwrap();
                    assert!(remaining.is_empty());
                };
                let server = async {
                    let (mut tls, caller_id) = accept(&target, c).await.unwrap();
                    assert_eq!(caller_id, caller.public().to_string());
                    assert_eq!(
                        tls.get_ref().1.protocol_version(),
                        Some(rustls::ProtocolVersion::TLSv1_3)
                    );
                    let mut request = [0; 17];
                    tls.read_exact(&mut request).await.unwrap();
                    assert_eq!(&request, b"claimed caller: B");
                    tls.write_all(b"ok").await.unwrap();
                    tls.shutdown().await.unwrap();
                    let mut remaining = Vec::new();
                    tls.read_to_end(&mut remaining).await.unwrap();
                    assert!(remaining.is_empty());
                };
                join(client, server).await;
            };
            let relay = async {
                let (forwarded, returned) = tokio::io::copy_bidirectional(&mut b_in, &mut b_out)
                    .await
                    .unwrap();
                assert!(forwarded > 0 && returned > 0);
            };
            join(endpoints, relay).await;
        });
    }

    #[test]
    fn wrong_target_pin_is_rejected() {
        run(async {
            let (a, c) = duplex(4096);
            let caller = key(1);
            let target = key(3);
            let wrong_target = key(4).public().to_string();
            let (client, server) =
                join(connect(&caller, &wrong_target, a), accept(&target, c)).await;
            assert!(matches!(client, Err(RemoteError::PeerMismatch)));
            assert!(matches!(
                server,
                Err(RemoteError::PeerMismatch | RemoteError::Unavailable)
            ));
        });
    }

    // An attacker can copy a public SPKI, but cannot sign its CertificateVerify.
    fn forged_resolver(claimed: &SecretKey, signer: &SecretKey) -> Arc<RawPublicKeyResolver> {
        Arc::new(RawPublicKeyResolver(Arc::new(CertifiedKey::new(
            vec![CertificateDer::from(
                spki(&claimed.public()).as_ref().to_vec(),
            )],
            Arc::new(IrohSigningKey(signer.clone())),
        ))))
    }

    #[test]
    fn forged_server_signature_is_rejected_even_with_the_right_pin() {
        run(async {
            let caller = key(1);
            let target = key(3);
            let mut config = server_config(&target).unwrap();
            config.cert_resolver = forged_resolver(&target, &key(4));
            let acceptor = TlsAcceptor::from(Arc::new(config));
            let (a, c) = duplex(4096);
            let target_id = target.public().to_string();
            let (client, server) = join(connect(&caller, &target_id, a), acceptor.accept(c)).await;
            assert!(matches!(client, Err(RemoteError::PeerMismatch)));
            assert!(server.is_err());
        });
    }

    #[test]
    fn forged_client_signature_is_rejected() {
        run(async {
            let caller = key(1);
            let target = key(3);
            let mut config = client_config(&caller, target.public()).unwrap();
            config.client_auth_cert_resolver = forged_resolver(&caller, &key(4));
            let connector = TlsConnector::from(Arc::new(config));
            let (a, c) = duplex(4096);
            let client = async {
                // TLS 1.3 clients can finish locally before the server rejects client auth.
                if let Ok(mut tls) = connector
                    .connect(ServerName::try_from(SERVER_NAME).unwrap(), a)
                    .await
                {
                    let mut byte = [0];
                    let result = tls.read(&mut byte).await;
                    assert!(result.is_err() || matches!(result, Ok(0)));
                }
            };
            let (_, server) = join(client, accept(&target, c)).await;
            assert!(matches!(server, Err(RemoteError::PeerMismatch)));
        });
    }

    #[derive(Debug)]
    struct NoClientIdentity;

    impl rustls::client::ResolvesClientCert for NoClientIdentity {
        fn resolve(&self, _: &[&[u8]], _: &[SignatureScheme]) -> Option<Arc<CertifiedKey>> {
            None
        }
        fn has_certs(&self) -> bool {
            false
        }
        fn only_raw_public_keys(&self) -> bool {
            true
        }
    }

    #[test]
    fn unauthenticated_client_is_rejected() {
        run(async {
            let caller = key(1);
            let target = key(3);
            let mut config = client_config(&caller, target.public()).unwrap();
            config.client_auth_cert_resolver = Arc::new(NoClientIdentity);
            let connector = TlsConnector::from(Arc::new(config));
            let (a, c) = duplex(4096);
            let client = async {
                if let Ok(mut tls) = connector
                    .connect(ServerName::try_from(SERVER_NAME).unwrap(), a)
                    .await
                {
                    let mut byte = [0];
                    let result = tls.read(&mut byte).await;
                    assert!(result.is_err() || matches!(result, Ok(0)));
                }
            };
            let (_, server) = join(client, accept(&target, c)).await;
            assert!(matches!(server, Err(RemoteError::PeerMismatch)));
        });
    }

    #[test]
    fn malformed_and_truncated_handshakes_are_rejected() {
        run(async {
            for data in [
                &b"not a TLS handshake"[..],
                &b"\x16\x03\x03\x00\x10\x01"[..],
            ] {
                let target = key(3);
                let (mut attacker, c) = duplex(4096);
                let write = async {
                    attacker.write_all(data).await.unwrap();
                    attacker.shutdown().await.unwrap();
                };
                let (_, result) = join(write, accept(&target, c)).await;
                assert!(matches!(
                    result,
                    Err(RemoteError::PeerMismatch | RemoteError::Unavailable)
                ));
            }
        });
    }

    #[test]
    fn only_canonical_ed25519_spki_is_accepted() {
        // RFC 8410 Ed25519 SubjectPublicKeyInfo with the RFC 8032 test-1 key.
        // Neither the expected DER nor the public key is made by our encoder.
        let bytes = [
            0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64,
            0x07, 0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68,
            0xf7, 0x07, 0x51, 0x1a,
        ];
        let peer = PublicKey::from_bytes(&bytes).unwrap();
        let mut encoded = vec![
            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
        ];
        encoded.extend_from_slice(&bytes);
        assert_eq!(spki(&peer).as_ref(), encoded);
        assert_eq!(
            public_key(&CertificateDer::from(encoded.clone())).unwrap(),
            peer
        );
        let mut wrong_algorithm = encoded.clone();
        wrong_algorithm[8] ^= 1;
        let mut trailing = encoded.clone();
        trailing.push(0);
        for bytes in [
            vec![],
            peer.as_bytes().to_vec(),
            wrong_algorithm,
            trailing,
            encoded[1..].to_vec(),
        ] {
            assert!(public_key(&CertificateDer::from(bytes)).is_err());
        }
    }

    #[test]
    fn reused_configs_still_require_full_handshakes() {
        run(async {
            let caller = key(1);
            let target = key(3);
            let connector =
                TlsConnector::from(Arc::new(client_config(&caller, target.public()).unwrap()));
            let server = server_config(&target).unwrap();
            assert!(!server.ticketer.enabled());
            assert!(!server.session_storage.can_cache());
            let acceptor = TlsAcceptor::from(Arc::new(server));
            for _ in 0..2 {
                let (a, c) = duplex(4096);
                let (client, server) = join(
                    connector.connect(ServerName::try_from(SERVER_NAME).unwrap(), a),
                    acceptor.accept(c),
                )
                .await;
                let mut client = client.unwrap();
                let mut server = server.unwrap();
                assert_eq!(
                    client.get_ref().1.handshake_kind(),
                    Some(rustls::HandshakeKind::Full)
                );
                assert_eq!(
                    server.get_ref().1.handshake_kind(),
                    Some(rustls::HandshakeKind::Full)
                );
                assert!(!client.get_ref().1.is_early_data_accepted());
                assert!(server.get_mut().1.early_data().is_none());
                // Process all post-handshake traffic, including any erroneously issued tickets.
                let (client_close, server_close) = join(client.shutdown(), server.shutdown()).await;
                client_close.unwrap();
                server_close.unwrap();
                let mut data = Vec::new();
                client.read_to_end(&mut data).await.unwrap();
                assert!(data.is_empty());
            }
        });
    }

    #[test]
    fn stalled_handshakes_time_out_and_release_the_io() {
        run(async {
            let caller = key(1);
            let target = key(3);
            let target_id = target.public().to_string();
            // Even ClientHello cannot drain when the destination stops reading.
            let (a, mut silent_server) = duplex(1);
            let (mut silent_client, c) = duplex(4096);
            let (client, server) = join(connect(&caller, &target_id, a), accept(&target, c)).await;
            assert!(matches!(client, Err(RemoteError::Timeout)));
            assert!(matches!(server, Err(RemoteError::Timeout)));
            let mut data = Vec::new();
            silent_server.read_to_end(&mut data).await.unwrap();
            assert!(!data.is_empty()); // ClientHello was sent, but no server replied.
            data.clear();
            silent_client.read_to_end(&mut data).await.unwrap();
            assert!(data.is_empty());
        });
    }

    #[test]
    fn cancelled_handshakes_release_the_io() {
        run(async {
            let caller = key(1);
            let target = key(3);
            let target_id = target.public().to_string();
            let (a, mut silent_server) = duplex(4096);
            let mut handshake = Box::pin(connect(&caller, &target_id, a));
            std::future::poll_fn(|cx| {
                assert!(std::future::Future::poll(handshake.as_mut(), cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            drop(handshake);
            let mut data = Vec::new();
            silent_server.read_to_end(&mut data).await.unwrap();
            assert!(!data.is_empty());

            let (mut silent_client, c) = duplex(4096);
            let mut handshake = Box::pin(accept(&target, c));
            std::future::poll_fn(|cx| {
                assert!(std::future::Future::poll(handshake.as_mut(), cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            drop(handshake);
            data.clear();
            silent_client.read_to_end(&mut data).await.unwrap();
            assert!(data.is_empty());
        });
    }
}
