//! Device/operator authentication command (plan sections 2 and 7.1).
//!
//! Calls the Go backend's `POST /auth/login` with a device identifier and
//! operator PIN, and returns the issued JWT. This command only performs
//! the network round-trip; persisting the token locally (via
//! `@tauri-apps/plugin-store`) is the frontend's responsibility.

use serde::{Deserialize, Serialize};

const DEFAULT_BACKEND_URL: &str = "http://localhost:8080";

/// Backend base URL, overridable via the `PDV_BACKEND_URL` env var so dev
/// and future multi-store deployments don't require a rebuild.
fn backend_url() -> String {
    std::env::var("PDV_BACKEND_URL").unwrap_or_else(|_| DEFAULT_BACKEND_URL.to_string())
}

#[derive(Debug, Serialize)]
struct LoginRequest {
    device_id: String,
    operator_pin: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OperatorInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LoginResponse {
    pub token: String,
    pub expires_at: String,
    pub device_id: String,
    pub operator: OperatorInfo,
}

#[tauri::command]
pub async fn login(device_id: String, operator_pin: String) -> Result<LoginResponse, String> {
    let device_id = device_id.trim().to_string();
    let operator_pin = operator_pin.trim().to_string();

    if device_id.is_empty() {
        return Err("device_id is required".into());
    }
    if operator_pin.is_empty() {
        return Err("operator_pin is required".into());
    }

    let url = format!("{}/auth/login", backend_url());
    let client = reqwest::Client::new();

    let response = client
        .post(&url)
        .json(&LoginRequest {
            device_id,
            operator_pin,
        })
        .send()
        .await
        .map_err(|e| format!("failed to reach backend at {url}: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("login rejected ({status}): {body}"));
    }

    response
        .json::<LoginResponse>()
        .await
        .map_err(|e| format!("unexpected response from backend: {e}"))
}
