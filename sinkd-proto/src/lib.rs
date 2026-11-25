//! Control messages carried on Zenoh. File bytes never go through here.

use serde::{Deserialize, Serialize};

pub const VERSION: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub version: u8,
    pub client_id: String,
    pub secret: [u8; 32],
    pub body: Body,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Body {
    Push {
        anchor_id: String,
        path: String,
        user: String,
        share: bool,
        basis: u64,
        #[serde(default)]
        ignore: Vec<String>,
    },
    Pull {
        anchor_id: String,
        path: String,
        user: String,
        share: bool,
        delete: bool,
        update_only: bool,
        #[serde(default)]
        ignore: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Granted {
        data_port: u16,
        token: [u8; 16],
        head: u64,
        client_sends: bool,
        delete: bool,
        update_only: bool,
    },
    Behind {
        head: u64,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadEvent {
    pub anchor_key: String,
    pub head: u64,
    pub writer: String,
}

#[must_use]
pub fn rpc_key(server_id: &str) -> String {
    format!("sinkd/v1/{server_id}/rpc")
}

#[must_use]
pub fn event_key(server_id: &str, anchor_key: &str) -> String {
    format!("sinkd/v1/{server_id}/events/{anchor_key}")
}

#[must_use]
pub fn events_selector(server_id: &str) -> String {
    format!("sinkd/v1/{server_id}/events/**")
}

#[must_use]
pub fn up_key(server_id: &str) -> String {
    format!("sinkd/v1/{server_id}/up")
}

pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    postcard::to_allocvec(value).map_err(|e| e.to_string())
}

pub fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    postcard::from_bytes(bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let req = Request {
            version: VERSION,
            client_id: "abc".to_string(),
            secret: [7; 32],
            body: Body::Push {
                anchor_id: "ff".to_string(),
                path: "/data".to_string(),
                user: "tony".to_string(),
                share: false,
                basis: 3,
                ignore: vec!["skip/".to_string()],
            },
        };
        let bytes = encode(&req).expect("encode");
        let back: Request = decode(&bytes).expect("decode");
        assert_eq!(back, req);
    }

    #[test]
    fn keys_are_stable() {
        assert_eq!(rpc_key("s"), "sinkd/v1/s/rpc");
        assert_eq!(event_key("s", "tony/a"), "sinkd/v1/s/events/tony/a");
        assert_eq!(up_key("s"), "sinkd/v1/s/up");
    }
}
