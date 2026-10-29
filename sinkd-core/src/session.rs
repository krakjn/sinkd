use std::path::Path;

pub enum Role<'a> {
    Server { bind: &'a str, port: u16 },
    Client { host: &'a str, port: u16 },
}

fn esc(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\").replace('"', "\\\"")
}

pub async fn open_session(
    ca_pem: &Path,
    cert_pem: &Path,
    key_pem: &Path,
    role: Role<'_>,
) -> Result<zenoh::Session, String> {
    let ca = esc(ca_pem);
    let cert = esc(cert_pem);
    let key = esc(key_pem);
    let (mode, listen, connect, tls_identity) = match role {
        Role::Server { bind, port } => (
            "router".to_string(),
            format!("[\"tls/{bind}:{port}\"]"),
            "[]".to_string(),
            format!("listen_private_key: \"{key}\", listen_certificate: \"{cert}\","),
        ),
        Role::Client { host, port } => (
            "client".to_string(),
            "[]".to_string(),
            format!("[\"tls/{host}:{port}\"]"),
            format!("connect_private_key: \"{key}\", connect_certificate: \"{cert}\","),
        ),
    };
    let json = format!(
        r#"{{
            mode: "{mode}",
            listen: {{ endpoints: {listen} }},
            connect: {{ endpoints: {connect} }},
            scouting: {{ multicast: {{ enabled: false }} }},
            transport: {{
                shared_memory: {{ enabled: false }},
                link: {{
                    protocols: ["tls"],
                    tls: {{
                        enable_mtls: true,
                        verify_name_on_connect: false,
                        root_ca_certificate: "{ca}",
                        {tls_identity}
                    }}
                }}
            }}
        }}"#
    );
    let config = zenoh::Config::from_json5(&json).map_err(|e| format!("zenoh config: {e}"))?;
    zenoh::open(config).await.map_err(|e| e.to_string())
}
