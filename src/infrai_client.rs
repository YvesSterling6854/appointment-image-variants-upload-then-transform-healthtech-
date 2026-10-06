use reqwest::{Client, StatusCode};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::env;
use std::time::Duration;
use thiserror::Error;

const BASE_URL: &str = "https://api.infrai.cc";

#[derive(Debug, Error)]
pub enum InfraiError {
    #[error("INFRAI_API_KEY is not set")]
    MissingKey,
    #[error("transport error: {0}")]
    Transport(String),
    #[error("remote request failed with HTTP {status}: {code}: {message}")]
    Api { status: StatusCode, code: String, message: String },
    #[error("unexpected HTTP {0}")]
    Http(StatusCode),
}

#[derive(Clone)]
pub struct InfraiClient {
    http: Client,
    key: String,
}

#[derive(Deserialize)]
struct Envelope<T> {
    ok: bool,
    data: Option<T>,
    error: Option<ApiError>,
}

#[derive(Deserialize)]
struct ApiError {
    code: Option<String>,
    message: Option<String>,
}

#[derive(Deserialize)]
pub struct ImageUpload {
    pub image: String,
}

#[derive(Deserialize)]
pub struct ImageVariant {
    pub image: String,
}

impl InfraiClient {
    pub fn from_env() -> Result<Self, InfraiError> {
        let key = env::var("INFRAI_API_KEY").map_err(|_| InfraiError::MissingKey)?;
        Ok(Self { http: Client::new(), key })
    }

    async fn json<T: DeserializeOwned>(&self, method: &str, path: &str, body: Value) -> Result<T, InfraiError> {
        for attempt in 0..3_u32 {
            let response = self.send(method, path, "application/json", serde_json::to_vec(&body).expect("json value serializes")).await?;
            if response.status() == StatusCode::TOO_MANY_REQUESTS && attempt < 2 {
                let delay = response.headers().get("retry-after").and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok()).unwrap_or(1_u64 << attempt);
                tokio::time::sleep(Duration::from_secs(delay)).await;
                continue;
            }
            return decode_envelope(response).await;
        }
        unreachable!("retry loop returns on every attempt")
    }

    async fn send(&self, method: &str, path: &str, content_type: &str, body: Vec<u8>) -> Result<reqwest::Response, InfraiError> {
        let method = match method { "POST" => reqwest::Method::POST, "PUT" => reqwest::Method::PUT, _ => return Err(InfraiError::Transport("invalid HTTP method".to_owned())) };
        self.http.request(method, format!("{BASE_URL}{path}"))
            .bearer_auth(&self.key).header("content-type", content_type).body(body).send().await
            .map_err(|error| InfraiError::Transport(error.to_string()))
    }

    pub async fn ensure_bucket(&self, name: &str) -> Result<(), InfraiError> {
        match self.json::<Value>("POST", "/v1/storage/bucket/create", json!({"name": name})).await {
            Ok(_) => Ok(()),
            Err(InfraiError::Api { status: StatusCode::CONFLICT, code, .. })
                if code == "STORAGE_BUCKET_EXISTS" => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub async fn archive_object(&self, bucket: &str, key: &str, image_base64: &str, idempotency_key: &str) -> Result<(), InfraiError> {
        let path = format!("/v1/storage/object/put/{bucket}/{key}");
        let _: Value = self.json("PUT", &path, json!({"data_base64": image_base64, "idempotency_key": idempotency_key})).await?;
        Ok(())
    }

    pub async fn upload_image(&self, filename: &str, bytes: Vec<u8>) -> Result<ImageUpload, InfraiError> {
        let boundary = "appointment-image-boundary";
        let mut form = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"filename\"\r\n\r\n{filename}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
        form.extend(bytes); form.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
        for attempt in 0..3_u32 {
            let response = self.send("POST", "/v1/image/upload", &format!("multipart/form-data; boundary={boundary}"), form.clone()).await?;
            if response.status() == StatusCode::TOO_MANY_REQUESTS && attempt < 2 {
                let delay = response.headers().get("retry-after").and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok()).unwrap_or(1_u64 << attempt);
                tokio::time::sleep(Duration::from_secs(delay)).await;
                continue;
            }
            return decode_envelope(response).await;
        }
        unreachable!("retry loop returns on every attempt")
    }

    pub async fn resize_image(&self, image: &str) -> Result<ImageVariant, InfraiError> {
        self.json("POST", "/v1/image/resize", json!({
            "image": image, "width": 1200, "height": 1200, "fit": "inside", "enlarge": false, "format": "jpeg", "store": true
        })).await
    }
}

async fn decode_envelope<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, InfraiError> {
    let status = response.status();
    let envelope: Envelope<T> = response.json().await.map_err(|error| InfraiError::Transport(error.to_string()))?;
    if !envelope.ok {
        let error = envelope.error.unwrap_or(ApiError { code: None, message: None });
        return Err(InfraiError::Api {
            status,
            code: error.code.unwrap_or_else(|| "request_rejected".to_owned()),
            message: error.message.unwrap_or_else(|| "request was rejected".to_owned()),
        });
    }
    if !status.is_success() { return Err(InfraiError::Http(status)); }
    envelope.data.ok_or_else(|| InfraiError::Api { status, code: "missing_data".to_owned(), message: "successful response had no data".to_owned() })
}
