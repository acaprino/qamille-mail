//! La forma HTTP degli errori: l'errore aperto delle estensioni.

use axum::http::StatusCode;
use axum::response::IntoResponse;
use qamille_server::ServerError;
use serde_json::{Value, json};

#[tokio::test]
async fn an_extension_error_carries_its_extra_fields_inside_the_error_object() {
    let error = ServerError::Extension {
        status: StatusCode::IM_A_TEAPOT,
        code: "teapot",
        message: "Sono una teiera.".to_owned(),
        retry_after_s: Some(7),
        extra: Some(json!({"plan": "pro"})),
    };
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
    assert_eq!(response.headers()["retry-after"], "7");
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body,
        json!({"error": {"code": "teapot", "message": "Sono una teiera.", "retry_after_s": 7, "plan": "pro"}})
    );
}
