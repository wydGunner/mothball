//! A stand-in printer speaking TLS with a self-signed certificate: the first
//! connection pins it, a later certificate change is refused with a clear message.

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use vault_core::printers::Endpoint;

fn serve_once(cert: &'static [u8], key: &'static [u8]) -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![CertificateDer::from(cert.to_vec())], PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.to_vec())))
            .unwrap();
        let (sock, _) = l.accept().unwrap();
        let mut s = StreamOwned::new(ServerConnection::new(Arc::new(cfg)).unwrap(), sock);
        let mut b = [0u8; 4];
        if s.read_exact(&mut b).is_ok() && &b == b"ping" {
            s.write_all(b"pong").unwrap();
            s.flush().unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    });
    port
}

const CERT_A: &[u8] = include_bytes!("fixtures/cert-a.der");
const KEY_A: &[u8] = include_bytes!("fixtures/key-a.der");
const CERT_B: &[u8] = include_bytes!("fixtures/cert-b.der");
const KEY_B: &[u8] = include_bytes!("fixtures/key-b.der");

fn ep(port: u16, pin: Option<String>) -> Endpoint {
    Endpoint { host: "127.0.0.1".into(), port, serial: "TEST".into(), pinned_sha256: pin }
}

fn read_reply(s: &mut dyn vault_core::printers::Stream) -> Vec<u8> {
    let mut out = Vec::new();
    let mut b = [0u8; 16];
    let t = std::time::Instant::now();
    while out.len() < 4 && t.elapsed().as_secs() < 5 {
        match s.read(&mut b) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&b[..n]),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => panic!("{e}"),
        }
    }
    out
}

#[test]
fn pins_on_first_use_and_refuses_a_changed_certificate() {
    let want = vault_core::util::sha256_bytes(CERT_A);

    // first connection: nothing pinned yet, the certificate is reported
    let port = serve_once(CERT_A, KEY_A);
    let mut c = bambu_tls::connect(&ep(port, None)).expect("connect");
    assert_eq!(c.cert_sha256.as_deref(), Some(want.as_str()));
    c.stream.write_all(b"ping").unwrap();
    assert_eq!(read_reply(&mut *c.stream), b"pong");

    // same certificate again with the pin: fine
    let port = serve_once(CERT_A, KEY_A);
    assert!(bambu_tls::connect(&ep(port, Some(want.clone()))).is_ok());

    // a different certificate with the old pin: refused, with the explanation
    let port = serve_once(CERT_B, KEY_B);
    let err = bambu_tls::connect(&ep(port, Some(want))).err().expect("must refuse");
    assert_eq!(err, bambu_tls::CERT_CHANGED);
}

#[test]
fn unreachable_printer_is_explained() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    let err = bambu_tls::connect(&ep(port, None)).err().unwrap();
    assert!(err.starts_with("Cannot reach the printer at 127.0.0.1"), "{err}");
}
