//! TLS for the printer connection.
//!
//! Bambu printers present a certificate issued by Bambu's own authority for the
//! printer's serial number, reached by IP address, so ordinary web PKI checks
//! don't apply. Instead the certificate is pinned on first use: its SHA-256 is
//! stored with the printer, and any later connection presenting a different
//! certificate is refused (someone else on the network could be pretending to
//! be the printer to collect the access code). The handshake signature is still
//! verified against the presented certificate, so the peer must hold its key.

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme, StreamOwned};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use vault_core::printers::{Connected, Connector, Endpoint};

pub const CERT_CHANGED: &str = "The printer presented a different security certificate than before. If its firmware was just updated, choose \"Trust new certificate\" in Settings → Printers; otherwise something else on the network may be pretending to be the printer.";

#[derive(Debug)]
struct Pinned {
    expected: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, _intermediates: &[CertificateDer<'_>], _name: &ServerName<'_>, _ocsp: &[u8], _now: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        let sha = vault_core::util::sha256_bytes(end_entity.as_ref());
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(sha.clone());
        match &self.expected {
            Some(want) if !want.eq_ignore_ascii_case(&sha) => Err(rustls::Error::General("certificate changed".into())),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }
    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algs)
    }
    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algs)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

/// Opens a TLS connection with the certificate pinned (or pinned now, on first use).
pub fn connect(e: &Endpoint) -> Result<Connected, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let seen = Arc::new(Mutex::new(None));
    let verifier = Pinned { expected: e.pinned_sha256.clone(), seen: seen.clone(), algs: provider.signature_verification_algorithms };
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|err| format!("TLS setup failed: {err}"))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    let name = ServerName::try_from(e.host.clone()).map_err(|_| format!("\"{}\" is not a valid printer address.", e.host))?;
    let mut conn = ClientConnection::new(Arc::new(config), name).map_err(|err| format!("TLS setup failed: {err}"))?;

    let addr = format!("{}:{}", e.host, e.port);
    let sock = addr
        .parse::<std::net::SocketAddr>()
        .ok()
        .map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(8)))
        .unwrap_or_else(|| TcpStream::connect(&addr))
        .map_err(|err| format!("Cannot reach the printer at {}: {err}. Check the IP address and that the printer is on and on the same network.", e.host))?;
    sock.set_read_timeout(Some(Duration::from_secs(10))).ok();
    sock.set_write_timeout(Some(Duration::from_secs(10))).ok();
    let mut sock = sock;
    while conn.is_handshaking() {
        if let Err(err) = conn.complete_io(&mut sock) {
            let changed = matches!((&e.pinned_sha256, seen.lock().unwrap_or_else(|x| x.into_inner()).as_ref()), (Some(a), Some(b)) if !a.eq_ignore_ascii_case(b));
            return Err(if changed { CERT_CHANGED.to_string() } else { format!("Secure connection to the printer failed: {err}") });
        }
    }
    // short reads let the worker send keep-alives and notice shutdown
    sock.set_read_timeout(Some(Duration::from_millis(500))).ok();
    let cert = seen.lock().unwrap_or_else(|x| x.into_inner()).clone();
    Ok(Connected { stream: Box::new(StreamOwned::new(conn, sock)), cert_sha256: cert })
}

pub fn connector() -> Connector {
    Arc::new(connect)
}
