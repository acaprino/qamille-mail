//! The mailboxes that follow a person from computer to computer (sync spec S2, S3): the item the
//! server keeps for each mailbox, its id and the merge by parts of two copies. No I/O. The
//! credential is a secret: it is never in `Debug`.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SCHEMA: u32 = 1;
pub const KIND: &str = "mail_account";
/// A time further ahead than this, against the local clock, counts as «now» (a computer with a
/// wrong clock must not win every conflict forever).
pub const MAX_SKEW_SECS: i64 = 600;
pub const CREDENTIAL_GOOGLE: &str = "google_refresh_token";
pub const CREDENTIAL_PASSWORD: &str = "password";

/// The settings of a mailbox that travel: not the local number, the index or the history.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SyncAccount {
    pub provider_id: String,
    pub name: String,
    pub email: String,
    pub color: [u8; 3],
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
    #[serde(default)]
    pub oauth_client_id: String,
    pub archive_folder: String,
    pub trash_folder: String,
}

/// What opens the mailbox: a refresh token of Google or an app password.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncCredential {
    pub kind: String,
    pub value: String,
    /// The secret of a personal Google client; never the one of the build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
}

impl std::fmt::Debug for SyncCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncCredential")
            .field("kind", &self.kind)
            .field("value", &"<hidden>")
            .field("client_secret", &self.client_secret.as_ref().map(|_| "<hidden>"))
            .finish()
    }
}

/// The content of one server item (`kind: "mail_account"`). Times are Unix seconds.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncItem {
    pub schema: u32,
    pub kind: String,
    pub updated_at: i64,
    pub credential_updated_at: i64,
    pub account: SyncAccount,
    pub credential: SyncCredential,
    /// The text of the signature of the mailbox, not the local reference to it.
    #[serde(default)]
    pub mail_signature: String,
}

impl std::fmt::Debug for SyncItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncItem")
            .field("updated_at", &self.updated_at)
            .field("credential_updated_at", &self.credential_updated_at)
            .field("account", &self.account)
            .field("credential", &self.credential)
            .finish_non_exhaustive()
    }
}

/// The id of the item of a mailbox: the same mailbox on two computers is one item.
pub fn item_id(provider_id: &str, email: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(provider_id.trim().as_bytes());
    hash.update([0u8]);
    hash.update(email.trim().to_lowercase().as_bytes());
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether `id` can be an item id (the server's rule: 64 lowercase hex digits).
pub fn valid_item_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A time further than `MAX_SKEW_SECS` ahead of `now` is `now`.
pub fn effective_time(at: i64, now: i64) -> i64 {
    if at > now + MAX_SKEW_SECS { now } else { at }
}

impl SyncItem {
    pub fn id(&self) -> String {
        item_id(&self.account.provider_id, &self.account.email)
    }

    /// An item from the server is data from outside: only a well-formed mailbox is applied.
    pub fn validate(&self) -> Result<(), &'static str> {
        let a = &self.account;
        if self.schema != SCHEMA || self.kind != KIND {
            return Err("unsupported item");
        }
        let text = |s: &str, max: usize| s.len() <= max && !s.chars().any(char::is_control);
        if !text(&a.provider_id, 40)
            || a.provider_id.trim().is_empty()
            || !text(&a.name, 200)
            || !text(&a.email, 254)
            || !a.email.contains('@')
            || a.email.contains(char::is_whitespace)
            || !text(&a.username, 254)
            || !text(&a.oauth_client_id, 300)
            || !text(&a.archive_folder, 200)
            || !text(&a.trash_folder, 200)
            || a.archive_folder.split(['/', '\\']).any(|part| part == "..")
            || a.trash_folder.split(['/', '\\']).any(|part| part == "..")
        {
            return Err("malformed account");
        }
        for host in [&a.imap_host, &a.smtp_host] {
            if host.is_empty() || host.len() > 253 || !host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-')) {
                return Err("malformed host");
            }
        }
        if a.imap_port == 0 || a.smtp_port == 0 {
            return Err("malformed port");
        }
        if !matches!(self.credential.kind.as_str(), CREDENTIAL_GOOGLE | CREDENTIAL_PASSWORD)
            || self.credential.value.trim().is_empty()
            || self.credential.value.len() > 4096
            || self.credential.client_secret.as_ref().is_some_and(|s| s.len() > 4096)
            || self.mail_signature.len() > 20_000
        {
            return Err("malformed credential");
        }
        // A Google token needs its client; a password never has one.
        if (self.credential.kind == CREDENTIAL_GOOGLE) == a.oauth_client_id.is_empty() {
            return Err("credential does not match the account");
        }
        Ok(())
    }
}

/// The merge by parts of two copies of one mailbox (S3): the credential, with the client it was
/// issued for, from the copy whose `credential_updated_at` is newer; the rest from the copy whose
/// `updated_at` is newer. A tie goes to `remote`. The result's times are the newest of both.
pub fn merge(local: &SyncItem, remote: &SyncItem, now: i64) -> SyncItem {
    merge_changed(local, remote, now, true, true)
}

/// The merge when it is known which parts of `local` changed on this computer since the server
/// last had them (`account_changed`: the settings and the signature; `credential_changed`: the
/// credential with its client). A part that did not change is not compared by time at all: it is
/// the remote copy's, whole, whatever the two clocks say (a computer whose clock is behind would
/// otherwise never be followed). Only a part changed on both sides is decided by the times.
pub fn merge_changed(local: &SyncItem, remote: &SyncItem, now: i64, account_changed: bool, credential_changed: bool) -> SyncItem {
    let (local_cred, remote_cred) = (effective_time(local.credential_updated_at, now), effective_time(remote.credential_updated_at, now));
    let (local_up, remote_up) = (effective_time(local.updated_at, now), effective_time(remote.updated_at, now));
    let cred = if credential_changed && local_cred > remote_cred { local } else { remote };
    let rest = if account_changed && local_up > remote_up { local } else { remote };
    let mut account = rest.account.clone();
    account.oauth_client_id = cred.account.oauth_client_id.clone();
    SyncItem {
        schema: SCHEMA,
        kind: KIND.into(),
        updated_at: if account_changed { local_up.max(remote_up) } else { remote_up },
        credential_updated_at: if credential_changed { local_cred.max(remote_cred) } else { remote_cred },
        account,
        credential: cred.credential.clone(),
        mail_signature: rest.mail_signature.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gmail(email: &str) -> SyncItem {
        SyncItem {
            schema: SCHEMA,
            kind: KIND.into(),
            updated_at: 100,
            credential_updated_at: 100,
            account: SyncAccount {
                provider_id: "gmail".into(),
                name: "Giulia".into(),
                email: email.into(),
                color: [1, 2, 3],
                imap_host: "imap.gmail.com".into(),
                imap_port: 993,
                smtp_host: "smtp.gmail.com".into(),
                smtp_port: 465,
                username: email.into(),
                oauth_client_id: "client.apps.googleusercontent.com".into(),
                archive_folder: "[Gmail]/All Mail".into(),
                trash_folder: "[Gmail]/Trash".into(),
            },
            credential: SyncCredential { kind: CREDENTIAL_GOOGLE.into(), value: "1//refresh-secret".into(), client_secret: Some("client-secret".into()) },
            mail_signature: "Giulia".into(),
        }
    }

    #[test]
    fn the_id_depends_on_provider_and_lowercase_address_only() {
        let id = item_id("gmail", "Giulia@Gmail.com");
        assert_eq!(id, item_id("gmail", "giulia@gmail.com"));
        assert!(valid_item_id(&id), "{id}");
        assert_ne!(id, item_id("imap", "giulia@gmail.com"));
        assert_ne!(id, item_id("gmail", "altra@gmail.com"));
        assert!(!valid_item_id(&id.to_uppercase()) && !valid_item_id("../x") && !valid_item_id(&id[1..]));
    }

    #[test]
    fn an_item_survives_json_and_validates() {
        let item = gmail("giulia@gmail.com");
        let back: SyncItem = serde_json::from_str(&serde_json::to_string(&item).unwrap()).unwrap();
        assert_eq!(back, item);
        assert_eq!(back.validate(), Ok(()));
        assert_eq!(item.id(), item_id("gmail", "giulia@gmail.com"));
        let mut password = gmail("a@b.it");
        password.credential = SyncCredential { kind: CREDENTIAL_PASSWORD.into(), value: "app-password".into(), client_secret: None };
        password.account.oauth_client_id.clear();
        assert_eq!(password.validate(), Ok(()));
        assert!(!serde_json::to_string(&password).unwrap().contains("client_secret"));
    }

    #[test]
    fn an_item_reads_what_the_server_may_add() {
        let mut value = serde_json::to_value(gmail("a@b.it")).unwrap();
        value["future"] = serde_json::json!(1);
        value["account"]["future"] = serde_json::json!(true);
        assert!(serde_json::from_value::<SyncItem>(value).is_ok());
    }

    #[test]
    fn a_hostile_item_is_refused() {
        let ok = gmail("a@b.it");
        let cases: Vec<Box<dyn Fn(&mut SyncItem)>> = vec![
            Box::new(|i| i.schema = 2),
            Box::new(|i| i.kind = "other".into()),
            Box::new(|i| i.account.imap_host = "a b".into()),
            Box::new(|i| i.account.smtp_host = "host/evil".into()),
            Box::new(|i| i.account.imap_port = 0),
            Box::new(|i| i.account.email = "no-at".into()),
            Box::new(|i| i.account.name = "due\nrighe".into()),
            Box::new(|i| i.account.archive_folder = "../x".into()),
            Box::new(|i| i.credential.value = "  ".into()),
            Box::new(|i| i.credential.kind = "other".into()),
            Box::new(|i| i.account.oauth_client_id.clear()),
            Box::new(|i| i.account.provider_id.clear()),
        ];
        for (n, mutate) in cases.iter().enumerate() {
            let mut item = ok.clone();
            mutate(&mut item);
            assert!(item.validate().is_err(), "case {n}");
        }
    }

    #[test]
    fn the_debug_form_hides_the_credential() {
        let item = gmail("a@b.it");
        for text in [format!("{item:?}"), format!("{item:#?}"), format!("{:?}", item.credential)] {
            assert!(!text.contains("refresh-secret") && !text.contains("client-secret"), "{text}");
        }
        assert!(format!("{item:?}").contains("a@b.it"));
    }

    #[test]
    fn merge_takes_the_credential_from_the_newest_credential_time_and_the_rest_from_the_newest_update() {
        let mut local = gmail("a@b.it");
        local.updated_at = 500; // the signature was edited here
        local.mail_signature = "Nuova firma".into();
        local.credential_updated_at = 100;
        let mut remote = gmail("a@b.it");
        remote.updated_at = 300;
        remote.credential_updated_at = 400; // the token was renewed elsewhere
        remote.credential.value = "1//new-token".into();
        remote.account.oauth_client_id = "other.apps.googleusercontent.com".into();
        let merged = merge(&local, &remote, 1000);
        assert_eq!(merged.mail_signature, "Nuova firma");
        assert_eq!(merged.credential.value, "1//new-token");
        assert_eq!(merged.account.oauth_client_id, "other.apps.googleusercontent.com", "the client follows its token");
        assert_eq!((merged.updated_at, merged.credential_updated_at), (500, 400));
        assert_eq!(merged.validate(), Ok(()));
    }

    #[test]
    fn a_part_not_changed_here_is_the_remote_one_whole_whatever_the_clocks_say() {
        let mut local = gmail("a@b.it");
        local.updated_at = 900; // remembered from the server: not changed here
        local.credential_updated_at = 900;
        let mut remote = gmail("a@b.it");
        remote.updated_at = 300; // written by a computer whose clock is behind
        remote.credential_updated_at = 300;
        remote.mail_signature = "Dall'altro".into();
        remote.credential.value = "1//new-token".into();
        let adopted = merge_changed(&local, &remote, 1000, false, false);
        assert_eq!((adopted.mail_signature.as_str(), adopted.credential.value.as_str()), ("Dall'altro", "1//new-token"));
        assert_eq!((adopted.updated_at, adopted.credential_updated_at), (300, 300));
        // Only the credential changed here: the settings still come from the server.
        let mut mixed_local = local.clone();
        mixed_local.credential.value = "1//mine".into();
        let mixed = merge_changed(&mixed_local, &remote, 1000, false, true);
        assert_eq!((mixed.mail_signature.as_str(), mixed.credential.value.as_str()), ("Dall'altro", "1//mine"));
        assert_eq!((mixed.updated_at, mixed.credential_updated_at), (300, 900));
        assert_eq!(merge(&local, &remote, 1000).mail_signature, "Giulia", "both changed: the times decide");
    }

    #[test]
    fn merge_ties_go_to_the_remote_item_and_the_merge_is_stable() {
        let mut local = gmail("a@b.it");
        local.mail_signature = "locale".into();
        let mut remote = gmail("a@b.it");
        remote.mail_signature = "remota".into();
        let merged = merge(&local, &remote, 1000);
        assert_eq!(merged.mail_signature, "remota");
        assert_eq!(merge(&merged, &remote, 1000), merged);
        assert_eq!(merge(&local, &merged, 1000), merged);
    }

    #[test]
    fn an_instant_far_in_the_future_counts_as_now() {
        assert_eq!(effective_time(1000 + 11 * 60, 1000), 1000);
        assert_eq!(effective_time(1000 + 9 * 60, 1000), 1000 + 9 * 60);
        let mut local = gmail("a@b.it");
        local.updated_at = 10_000_000; // a clock gone mad
        local.mail_signature = "pazza".into();
        let mut remote = gmail("a@b.it");
        remote.updated_at = 1000;
        remote.mail_signature = "giusta".into();
        // «now» (1000) ties with the remote copy, and a tie goes to the remote.
        assert_eq!(merge(&local, &remote, 1000).mail_signature, "giusta");
    }
}
