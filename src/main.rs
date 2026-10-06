mod appointment_images;
mod infrai_client;

use appointment_images::{archive_key, patient_safe_notification, IntakeRequest, IntakeResult};
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use infrai_client::{InfraiClient, InfraiError};
use std::{net::SocketAddr, sync::Arc};

const BUCKET: &str = "appointment-images";

#[derive(Clone)]
struct AppState { infrai: Arc<InfraiClient> }

#[tokio::main]
async fn main() -> Result<(), InfraiError> {
    let infrai = Arc::new(InfraiClient::from_env()?);
    infrai.ensure_bucket(BUCKET).await?;
    let app = Router::new().route("/appointments/image", post(intake_image)).with_state(AppState { infrai });
    let address: SocketAddr = "127.0.0.1:3000".parse().expect("valid listen address");
    let listener = tokio::net::TcpListener::bind(address).await.map_err(|error| InfraiError::Transport(error.to_string()))?;
    axum::serve(listener, app).await.map_err(|error| InfraiError::Transport(error.to_string()))
}

async fn intake_image(State(state): State<AppState>, Json(request): Json<IntakeRequest>) -> Result<Json<IntakeResult>, (StatusCode, String)> {
    let bytes = decode_base64(&request.image_base64).ok_or((StatusCode::BAD_REQUEST, "image_base64 must be valid base64".to_owned()))?;
    let key = archive_key(&request.appointment_id, &request.filename);
    let idempotency_key = format!("{}:{}", request.appointment_id, request.filename);
    state.infrai.archive_object(BUCKET, &key, &request.image_base64, &idempotency_key).await.map_err(map_error)?;
    let uploaded = state.infrai.upload_image(&request.filename, bytes).await.map_err(map_error)?;
    let variant = state.infrai.resize_image(&uploaded.image).await.map_err(map_error)?;
    Ok(Json(IntakeResult { appointment_id: request.appointment_id.clone(), archive_key: key, variant: variant.image, notification: patient_safe_notification(&request.appointment_id) }))
}

fn map_error(error: InfraiError) -> (StatusCode, String) {
    match error { InfraiError::Api { status, message, .. } if status.is_client_error() => (status, message), other => (StatusCode::BAD_GATEWAY, other.to_string()) }
}

fn decode_base64(value: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = Vec::new(); let mut block = [0_u8; 4]; let mut used = 0;
    for byte in value.bytes().filter(|b| !b.is_ascii_whitespace()) {
        block[used] = if byte == b'=' { 64 } else { ALPHABET.iter().position(|x| *x == byte)? as u8 }; used += 1;
        if used == 4 { output.push((block[0] << 2) | (block[1] >> 4)); if block[2] != 64 { output.push((block[1] << 4) | (block[2] >> 2)); } if block[3] != 64 { output.push((block[2] << 6) | block[3]); } used = 0; }
    }
    (used == 0).then_some(output)
}
