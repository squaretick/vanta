//! `vanta-test` — hermetic test fixtures: build artifacts in memory, hash them,
//! and serve them over a tiny local HTTP server so the install pipeline can be
//! exercised end-to-end without the network (`docs/28-testing.md`).
#![forbid(unsafe_code)]

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

/// Build a `.tar.gz` in memory containing `files` under a top-level `top/` dir
/// (so providers with `strip = 1` lay them out correctly). Each file is mode 0755.
pub fn make_targz(top: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    for (path, data) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, format!("{top}/{path}"), *data)
            .expect("append fixture file");
    }
    builder
        .into_inner()
        .expect("finish tar")
        .finish()
        .expect("finish gzip")
}

/// Lowercase-hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Produce a minisign keypair (from `seed`) and a legacy (`Ed`) `.minisig` over
/// `data`, returning `(public_key_text, signature_text)` in minisign file format.
/// For tests of the signature-verification path.
pub fn minisign_sign(seed: [u8; 32], data: &[u8]) -> (String, String) {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use ed25519_dalek::{Signer, SigningKey};

    let key_id = [1u8, 2, 3, 4, 5, 6, 7, 8];
    let sk = SigningKey::from_bytes(&seed);
    let pk = sk.verifying_key().to_bytes();
    let sig = sk.sign(data).to_bytes();

    let mut pk_raw = Vec::new();
    pk_raw.extend_from_slice(b"Ed");
    pk_raw.extend_from_slice(&key_id);
    pk_raw.extend_from_slice(&pk);
    let pubkey = format!(
        "untrusted comment: vanta-test\n{}",
        STANDARD.encode(&pk_raw)
    );

    let mut sig_raw = Vec::new();
    sig_raw.extend_from_slice(b"Ed");
    sig_raw.extend_from_slice(&key_id);
    sig_raw.extend_from_slice(&sig);
    let sig_file = format!(
        "untrusted comment: vanta-test sig\n{}\ntrusted comment: t\n{}",
        STANDARD.encode(&sig_raw),
        STANDARD.encode([0u8; 64])
    );
    (pubkey, sig_file)
}

/// Serve a fixed set of `path -> bytes` over HTTP on a random localhost port,
/// returning the port. The server runs on a detached thread for the test's life.
pub fn serve(files: HashMap<String, Vec<u8>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let port = listener.local_addr().expect("addr").port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
            match files.get(&path) {
                Some(body) => {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(body);
                }
                None => {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                }
            }
        }
    });
    port
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targz_and_hash_are_stable() {
        let a = make_targz("t-1.0.0", &[("bin/t", b"hello")]);
        let b = make_targz("t-1.0.0", &[("bin/t", b"hello")]);
        // gzip embeds no timestamp by default here → identical bytes → identical hash.
        assert_eq!(sha256_hex(&a), sha256_hex(&b));
        assert_eq!(sha256_hex(b"").len(), 64);
    }

    #[test]
    fn server_serves_registered_paths() {
        let mut files = HashMap::new();
        files.insert("/x".to_string(), b"payload".to_vec());
        let port = serve(files);
        // crude client
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(b"GET /x HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut resp = String::new();
        s.read_to_string(&mut resp).unwrap();
        assert!(resp.contains("200 OK"));
        assert!(resp.ends_with("payload"));
    }
}
