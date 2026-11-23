//
//  Copyright (c) 2026 Viva Republica, Inc.
//

//! Protocol v1 framing and handshake: a 4 byte big endian length, then a JSON message.

use serde::Deserialize;
use serde_json::{json, Value};
use std::io;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTOCOL_VERSION: i64 = 1;
/// Guards against a malformed length prefix allocating unbounded memory.
pub const MAXIMUM_MESSAGE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub protocol_version: i64,
    #[serde(rename = "appBundleID")]
    pub app_bundle_id: String,
    pub app_name: String,
    #[serde(default)]
    pub app_version: String,
    pub device_name: String,
    #[serde(default)]
    pub os_version: String,
    /// Base64 PNG, absent when the app has no icon to read.
    pub app_icon: Option<String>,
}

pub fn frame(message: &[u8]) -> io::Result<Vec<u8>> {
    if message.len() > MAXIMUM_MESSAGE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "message exceeds the frame limit"));
    }
    let mut bytes = (message.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(message);
    Ok(bytes)
}

pub async fn read_frame(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<Vec<u8>> {
    let mut header = [0u8; 4];
    reader.read_exact(&mut header).await?;
    let length = u32::from_be_bytes(header) as usize;
    if length > MAXIMUM_MESSAGE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "message exceeds the frame limit"));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).await?;
    Ok(body)
}

pub fn envelope(kind: &str, payload: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({ "type": kind, "payload": payload })).expect("JSON values always encode")
}

/// Reads the app's hello and answers it. A different protocol version is refused, never negotiated.
pub async fn handshake(stream: &mut (impl AsyncRead + AsyncWrite + Unpin)) -> io::Result<Hello> {
    let exchange = async {
        let hello: Hello = serde_json::from_slice(&read_frame(stream).await?)?;
        let rejection = if hello.protocol_version != PROTOCOL_VERSION {
            Some("unsupportedProtocolVersion")
        } else if hello.app_bundle_id.is_empty() {
            Some("missingAppIdentity")
        } else {
            None
        };
        let mut ack = json!({ "accepted": rejection.is_none(), "hostProtocolVersion": PROTOCOL_VERSION });
        if let Some(rejection) = rejection {
            ack["rejection"] = json!(rejection);
        }
        stream.write_all(&frame(&serde_json::to_vec(&ack)?)?).await?;
        match rejection {
            None => Ok(hello),
            Some(reason) => Err(io::Error::new(io::ErrorKind::InvalidData, format!("{}: {reason}", hello.app_bundle_id))),
        }
    };
    tokio::time::timeout(Duration::from_secs(10), exchange)
        .await
        .unwrap_or_else(|_| Err(io::Error::new(io::ErrorKind::TimedOut, "handshake timed out")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trips_a_frame_and_refuses_oversized_lengths() {
        let bytes = frame(b"{\"a\":1}").unwrap();
        assert_eq!(&bytes[..4], &[0, 0, 0, 7]);
        assert_eq!(read_frame(&mut bytes.as_slice()).await.unwrap(), b"{\"a\":1}");

        let huge = ((MAXIMUM_MESSAGE_BYTES + 1) as u32).to_be_bytes();
        assert!(read_frame(&mut huge.as_slice()).await.is_err());
    }

    #[tokio::test]
    async fn accepts_v1_and_rejects_other_versions() {
        for (version, accepted) in [(1, true), (2, false)] {
            let (mut app, mut host) = tokio::io::duplex(4096);
            let hello = json!({ "protocolVersion": version, "appBundleID": "a.b", "appName": "A", "deviceName": "P", "sdkVersion": "1" });
            app.write_all(&frame(&serde_json::to_vec(&hello).unwrap()).unwrap()).await.unwrap();
            assert_eq!(handshake(&mut host).await.is_ok(), accepted);
            let ack: Value = serde_json::from_slice(&read_frame(&mut app).await.unwrap()).unwrap();
            assert_eq!(ack["accepted"], json!(accepted));
        }
    }
}
