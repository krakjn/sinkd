use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use rcgen::{BasicConstraints, Certificate, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};

use crate::paths::{hex_decode, hex_encode, random_bytes};
use crate::atomic_write;

pub struct ServerIdentity {
    pub id: String,
    pub ca_pem: PathBuf,
    pub cert_pem: PathBuf,
    pub key_pem: PathBuf,
}

pub struct ClientBundle {
    pub id: String,
    pub server_id: String,
    pub secret: [u8; 32],
    pub ca_pem: PathBuf,
    pub cert_pem: PathBuf,
    pub key_pem: PathBuf,
}

fn write_mode(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(mode)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    Ok(())
}

fn sign(name: &str, issuer: &Certificate, issuer_key: &KeyPair) -> Result<(String, String), String> {
    let key = KeyPair::generate().map_err(|e| e.to_string())?;
    let params = CertificateParams::new(vec![name.to_string()]).map_err(|e| e.to_string())?;
    let cert = params
        .signed_by(&key, issuer, issuer_key)
        .map_err(|e| e.to_string())?;
    Ok((cert.pem(), key.serialize_pem()))
}

pub fn ensure_server_identity(state: &Path) -> Result<ServerIdentity, String> {
    fs::create_dir_all(state).map_err(|e| format!("{}: {e}", state.display()))?;
    let id_path = state.join("id");
    let ca_path = state.join("ca.pem");
    let ca_key_path = state.join("ca.key");
    let cert_path = state.join("cert.pem");
    let key_path = state.join("key.pem");
    if id_path.exists() && ca_path.exists() && cert_path.exists() && key_path.exists() {
        let id = fs::read_to_string(&id_path).map_err(|e| e.to_string())?;
        return Ok(ServerIdentity {
            id: id.trim().to_string(),
            ca_pem: ca_path,
            cert_pem: cert_path,
            key_pem: key_path,
        });
    }
    let id = hex_encode(&random_bytes::<16>()?);
    let ca_key = KeyPair::generate().map_err(|e| e.to_string())?;
    let ca_key_pem = ca_key.serialize_pem();
    let mut ca_params = CertificateParams::new(vec!["sinkd-ca".to_string()]).map_err(|e| e.to_string())?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages.push(KeyUsagePurpose::KeyCertSign);
    let ca_cert = ca_params.self_signed(&ca_key).map_err(|e| e.to_string())?;
    let (cert_pem, key_pem) = sign("sinkd-server", &ca_cert, &ca_key)?;
    write_mode(&id_path, format!("{id}\n").as_bytes(), 0o644)?;
    write_mode(&ca_path, ca_cert.pem().as_bytes(), 0o644)?;
    write_mode(&ca_key_path, ca_key_pem.as_bytes(), 0o600)?;
    write_mode(&cert_path, cert_pem.as_bytes(), 0o644)?;
    write_mode(&key_path, key_pem.as_bytes(), 0o600)?;
    Ok(ServerIdentity {
        id,
        ca_pem: ca_path,
        cert_pem: cert_path,
        key_pem: key_path,
    })
}

pub fn issue_client(state: &Path, client_id: &str) -> Result<(String, String, String, [u8; 32]), String> {
    let ca_pem = fs::read_to_string(state.join("ca.pem")).map_err(|e| e.to_string())?;
    let ca_key_pem = fs::read_to_string(state.join("ca.key")).map_err(|e| e.to_string())?;
    let ca_key = KeyPair::from_pem(&ca_key_pem).map_err(|e| e.to_string())?;
    let ca_params = CertificateParams::from_ca_cert_pem(&ca_pem).map_err(|e| e.to_string())?;
    let ca_cert = ca_params.self_signed(&ca_key).map_err(|e| e.to_string())?;
    let (cert_pem, key_pem) = sign(client_id, &ca_cert, &ca_key)?;
    let secret = random_bytes::<32>()?;
    save_peer(state, client_id, secret)?;
    Ok((ca_pem, cert_pem, key_pem, secret))
}

pub fn peers_path(state: &Path) -> PathBuf {
    state.join("peers")
}

pub fn load_peers(state: &Path) -> Result<HashMap<String, [u8; 32]>, String> {
    let path = peers_path(state);
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(HashMap::new());
    };
    let mut peers = HashMap::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(id) = parts.next() else { continue };
        let Some(secret) = parts.next() else { continue };
        let bytes = hex_decode(secret)?;
        let secret: [u8; 32] = bytes.try_into().map_err(|_| "peer secret must be 32 bytes".to_string())?;
        peers.insert(id.to_string(), secret);
    }
    Ok(peers)
}

pub fn save_peer(state: &Path, client_id: &str, secret: [u8; 32]) -> Result<(), String> {
    let mut peers = load_peers(state)?;
    peers.insert(client_id.to_string(), secret);
    let mut text = String::new();
    for (id, secret) in &peers {
        text.push_str(id);
        text.push(' ');
        text.push_str(&hex_encode(secret));
        text.push('\n');
    }
    write_mode(&peers_path(state), text.as_bytes(), 0o600)
}

pub fn ensure_client_id(state: &Path) -> Result<String, String> {
    fs::create_dir_all(state).map_err(|e| e.to_string())?;
    let path = state.join("id");
    if let Ok(text) = fs::read_to_string(&path) {
        let id = text.trim();
        if !id.is_empty() {
            return Ok(id.to_string());
        }
    }
    let id = hex_encode(&random_bytes::<16>()?);
    write_mode(&path, format!("{id}\n").as_bytes(), 0o644)?;
    Ok(id)
}

pub fn save_client_bundle(
    state: &Path,
    server_id: &str,
    server_host: &str,
    ca_pem: &str,
    cert_pem: &str,
    key_pem: &str,
    secret: &[u8; 32],
) -> Result<(), String> {
    write_mode(&state.join("ca.pem"), ca_pem.as_bytes(), 0o644)?;
    write_mode(&state.join("cert.pem"), cert_pem.as_bytes(), 0o644)?;
    write_mode(&state.join("key.pem"), key_pem.as_bytes(), 0o600)?;
    write_mode(&state.join("secret"), secret, 0o600)?;
    atomic_write(&state.join("server_id"), format!("{server_id}\n").as_bytes())?;
    atomic_write(&state.join("server_host"), format!("{server_host}\n").as_bytes())?;
    Ok(())
}

pub fn load_client_bundle(state: &Path) -> Result<ClientBundle, String> {
    let id = fs::read_to_string(state.join("id")).map_err(|e| format!("client id: {e}"))?;
    let server_id = fs::read_to_string(state.join("server_id")).map_err(|_| {
        "not paired yet; run `sinkd pair <host>`".to_string()
    })?;
    let secret_bytes = fs::read(state.join("secret")).map_err(|e| format!("secret: {e}"))?;
    let secret: [u8; 32] = secret_bytes
        .try_into()
        .map_err(|_| "secret must be 32 bytes".to_string())?;
    Ok(ClientBundle {
        id: id.trim().to_string(),
        server_id: server_id.trim().to_string(),
        secret,
        ca_pem: state.join("ca.pem"),
        cert_pem: state.join("cert.pem"),
        key_pem: state.join("key.pem"),
    })
}

pub fn secrets_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in a.iter().zip(b) {
        diff |= left ^ right;
    }
    diff == 0
}
