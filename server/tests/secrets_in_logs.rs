//! Nessun segreto nei `Debug` delle richieste del core.

use qamille_server::http::{
    ChangePasswordRequest, DeleteAccountRequest, DisableDeviceRequest, LoginRequest,
    TemporaryPasswordRequest,
};
use serde_json::json;

#[test]
fn debug_of_password_requests_hides_the_password() {
    let login: LoginRequest = serde_json::from_value(json!({
        "identifier": "mario", "password": "segreto-uno-1234", "device_name": "PC"
    }))
    .unwrap();
    let temporary: TemporaryPasswordRequest = serde_json::from_value(json!({
        "identifier": "mario", "temporary_password": "segreto-due-1234",
        "new_password": "segreto-tre-1234", "device_name": "PC"
    }))
    .unwrap();
    let disable: DisableDeviceRequest = serde_json::from_value(json!({
        "identifier": "mario", "password": "segreto-quattro-1234", "device_id": "d"
    }))
    .unwrap();
    let change: ChangePasswordRequest = serde_json::from_value(json!({
        "current_password": "segreto-cinque-1234", "new_password": "segreto-sei-1234"
    }))
    .unwrap();
    assert!(change.sign_out_other_devices, "spuntato di base");
    let delete: DeleteAccountRequest =
        serde_json::from_value(json!({ "password": "segreto-sette-1234" })).unwrap();

    let printed = format!("{login:?} {temporary:?} {disable:?} {change:?} {delete:?} {login:#?}");
    assert!(!printed.contains("segreto"), "{printed}");
    assert!(printed.contains("LoginRequest"));
}
