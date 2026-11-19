//! The app's HTTP client against a running qamille-server (manual check of the wave 2 gate).
//! Start the server and create a user first, then:
//!   QAMILLE_LIVE_SERVER=http://127.0.0.1:8787 QAMILLE_LIVE_USER=mario \
//!   QAMILLE_LIVE_TEMPORARY=<temporary password> cargo test --jobs 2 --test server_account_live -- --ignored --nocapture
use qamille::{
    application::server_account::{Credentials, LoginOutcome, RemoteEndpoint, ServerAccount, ServerAddress},
    infrastructure::server_account::HttpServerAccount,
};
use std::time::Duration;

#[test]
#[ignore = "needs a running qamille-server and a fresh user"]
fn the_client_signs_in_lists_devices_and_signs_out_against_the_real_server() {
    let url = std::env::var("QAMILLE_LIVE_SERVER").expect("QAMILLE_LIVE_SERVER");
    let user = std::env::var("QAMILLE_LIVE_USER").expect("QAMILLE_LIVE_USER");
    let temporary = std::env::var("QAMILLE_LIVE_TEMPORARY").expect("QAMILLE_LIVE_TEMPORARY");
    let http = HttpServerAccount;
    let address = ServerAddress { url: url.clone(), ca_pem: None };
    let info = http.info(&address).unwrap();
    println!("info: {info:?}");
    assert_eq!(info.api, 1);
    let first = Credentials { identifier: &user, password: &temporary, device_name: "Prova 1" };
    assert!(matches!(http.login(&address, &first).unwrap(), LoginOutcome::PasswordChangeRequired), "EX-11: temporary first");
    let wrong = Credentials { identifier: &user, password: "sbagliata-123456", device_name: "Prova" };
    assert_eq!(http.login(&address, &wrong).err().unwrap().code, "invalid_credentials");
    let LoginOutcome::Signed(grant) = http.change_temporary_password(&address, &first, "una password nuova e lunga").unwrap() else { panic!("not signed") };
    assert!(grant.device_key.starts_with("spk_dev_"));
    let endpoint = RemoteEndpoint { url: url.clone(), device_key: grant.device_key.clone(), ca_pem: None, timeout: Duration::from_secs(10) };
    let devices = http.devices(&endpoint).unwrap();
    assert_eq!(devices.len(), 1);
    assert!(devices[0].current);
    // Up to the limit of three, then 409 with the list (EX-14).
    let again = |name: &'static str| Credentials { identifier: &user, password: "una password nuova e lunga", device_name: name };
    for name in ["Prova 2", "Prova 3"] {
        assert!(matches!(http.login(&address, &again(name)).unwrap(), LoginOutcome::Signed(_)));
    }
    let LoginOutcome::DeviceLimit(listed) = http.login(&address, &again("Prova 4")).unwrap() else { panic!("no device limit") };
    assert_eq!(listed.len(), 3);
    let old = listed.iter().find(|d| d.name == "Prova 2").unwrap();
    http.disable_device_with_password(&address, &user, "una password nuova e lunga", &old.id).unwrap();
    assert!(matches!(http.login(&address, &again("Prova 4")).unwrap(), LoginOutcome::Signed(_)));
    let limited = http.change_password(&endpoint, "sbagliata", "un'altra password lunga", true).err().unwrap();
    println!("wrong current password: {} / {}", limited.code, limited.message);
    http.change_password(&endpoint, "una password nuova e lunga", "un'altra password lunga", true).unwrap();
    assert_eq!(http.devices(&endpoint).unwrap().len(), 1, "the other devices were signed out");
    http.sign_out(&endpoint).unwrap();
    assert_eq!(http.devices(&endpoint).err().unwrap().code, "unauthorized", "the key no longer works");
}
