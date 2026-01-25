//! REST API handlers for the web UI.

use crate::state::{FaultRecord, IoSnapshot, MetricsSnapshot, SharedState, StateSnapshot};
use axum::{
    extract::Extension,
    http::StatusCode,
    response::{IntoResponse, Json},
};
use serde::Serialize;
use std::sync::Arc;

/// Health check response.
#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

/// Health check endpoint.
///
/// GET /health
pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Get full state snapshot.
///
/// GET /api/state
pub async fn get_state(
    Extension(state): Extension<Arc<SharedState>>,
) -> Result<Json<StateSnapshot>, StatusCode> {
    let snapshot = state.snapshot();
    Ok(Json(snapshot))
}

/// Get cycle metrics.
///
/// GET /api/metrics
pub async fn get_metrics(
    Extension(state): Extension<Arc<SharedState>>,
) -> Result<Json<MetricsSnapshot>, StatusCode> {
    let metrics = state
        .metrics
        .read()
        .map(|m| m.clone())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(metrics))
}

/// Get I/O state.
///
/// GET /api/io
pub async fn get_io_state(
    Extension(state): Extension<Arc<SharedState>>,
) -> Result<Json<IoSnapshot>, StatusCode> {
    let io = state
        .io
        .read()
        .map(|i| i.clone())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(io))
}

/// Get recent faults.
///
/// GET /api/faults
pub async fn get_faults(
    Extension(state): Extension<Arc<SharedState>>,
) -> Result<Json<Vec<FaultRecord>>, StatusCode> {
    let faults = state
        .faults
        .read()
        .map(|f| f.clone())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(faults))
}

/// API error response.
#[derive(Serialize)]
pub struct ApiError {
    pub error: String,
    pub code: u16,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let status = StatusCode::from_u16(self.code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(self)).into_response()
    }
}
