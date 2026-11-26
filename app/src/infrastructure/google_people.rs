//! Google People API adapter for contacts. No filesystem or UI; credentials come through a token source.
use crate::application::{
    AppError, AppResult,
    contact_ports::{ContactGateway, RemoteContact, RemotePage},
    ports::{GOOGLE_SECRET, SecretStore},
};
use crate::domain::{contacts::*, model::Account};
use serde_json::{Map, Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::{Duration, Instant},
};

pub const READ_SCOPE: &str = "https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/contacts.readonly https://www.googleapis.com/auth/contacts.other.readonly";
pub const WRITE_SCOPE: &str = "https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/contacts https://www.googleapis.com/auth/contacts.other.readonly";

const API: &str = "https://people.googleapis.com/v1";
const PERSON_FIELDS: &str = "names,emailAddresses,phoneNumbers,addresses,organizations,birthdays,biographies,metadata,clientData";
// Sync tokens are bound to the field mask of their original request.
const CONTACT_TOKEN_PREFIX: &str = "client-data-v1:";
const WRITE_FIELDS: &str = "names,emailAddresses,phoneNumbers,addresses,organizations,birthdays,biographies";
const OTHER_FIELDS: &str = "names,emailAddresses,phoneNumbers,metadata";
const RAW_GROUPS: [&str; 7] = ["names", "emailAddresses", "phoneNumbers", "addresses", "organizations", "birthdays", "biographies"];
const OUTPUT_ONLY: [&str; 7] = ["metadata", "formattedValue", "formattedType", "canonicalForm", "displayName", "displayNameLastFirst", "unstructuredName"];
const MANAGED_KEYS: [&str; 10] = [
    "names", "emailAddresses", "phoneNumbers", "addresses", "organizations", "birthdays", "biographies", "resourceName", "etag", "metadata",
];

fn text(value: &Value) -> String {
    value.as_str().unwrap_or("").to_string()
}
fn label(item: &Value) -> String {
    item["type"].as_str().or_else(|| item["formattedType"].as_str()).unwrap_or("").to_lowercase()
}
fn labeled(list: &Value, value_key: &str, fallback_key: &str) -> Vec<Labeled> {
    list.as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| Labeled { label: label(item), value: item[value_key].as_str().or_else(|| item[fallback_key].as_str()).unwrap_or("").to_string() })
                .filter(|l| !l.value.trim().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// A person from the API as a remote contact; everything not managed here goes to `unmanaged`.
pub fn person_to_remote(person: &Value) -> RemoteContact {
    let name = &person["names"][0];
    let display = name["displayName"].as_str().or_else(|| name["unstructuredName"].as_str()).map(String::from).unwrap_or_else(|| {
        format!("{} {}", text(&name["givenName"]), text(&name["familyName"])).trim().to_string()
    });
    let date = &person["birthdays"][0]["date"];
    let birthday = match (date["year"].as_u64(), date["month"].as_u64(), date["day"].as_u64()) {
        (Some(y), Some(m), Some(d)) => format!("{y:04}-{m:02}-{d:02}"),
        (None, Some(m), Some(d)) => format!("--{m:02}-{d:02}"),
        _ => String::new(),
    };
    let mut unmanaged: Map<String, Value> = person
        .as_object()
        .map(|o| o.iter().filter(|(k, _)| !MANAGED_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    // The arrays as read, so groups the user did not touch are written back with their full structure.
    let raw: Map<String, Value> = RAW_GROUPS
        .iter()
        .filter_map(|key| person[*key].as_array().filter(|a| !a.is_empty()).map(|a| (key.to_string(), Value::Array(a.clone()))))
        .collect();
    if !raw.is_empty() {
        unmanaged.insert("_raw".into(), Value::Object(raw));
    }
    let source_etag = person["metadata"]["sources"]
        .as_array()
        .and_then(|sources| sources.iter().find(|s| s["type"] == "CONTACT"))
        .map(|s| text(&s["etag"]))
        .filter(|e| !e.is_empty());
    RemoteContact {
        remote_id: text(&person["resourceName"]),
        etag: source_etag.unwrap_or_else(|| text(&person["etag"])),
        deleted: person["metadata"]["deleted"].as_bool().unwrap_or(false),
        fields: ContactFields {
            name: display,
            org: text(&person["organizations"][0]["name"]),
            title: text(&person["organizations"][0]["title"]),
            birthday,
            emails: labeled(&person["emailAddresses"], "value", "value"),
            phones: labeled(&person["phoneNumbers"], "value", "value"),
            addresses: labeled(&person["addresses"], "formattedValue", "streetAddress"),
            notes: text(&person["biographies"][0]["value"]),
        },
        unmanaged: if unmanaged.is_empty() { Value::Null } else { Value::Object(unmanaged) },
    }
}

fn typed(items: &[Labeled], key: &str) -> Value {
    Value::Array(
        items
            .iter()
            .filter(|i| !i.value.trim().is_empty())
            .map(|i| {
                let mut entry = Map::new();
                entry.insert(key.to_string(), json!(i.value));
                if !i.label.is_empty() {
                    entry.insert("type".into(), json!(i.label));
                }
                Value::Object(entry)
            })
            .collect(),
    )
}

fn birthday_date(value: &str) -> Option<Value> {
    let parts: Vec<&str> = value.trim().split('-').collect();
    match parts.as_slice() {
        [y, m, d] if !y.is_empty() => Some(json!({"year": y.parse::<u32>().ok()?, "month": m.parse::<u32>().ok()?, "day": d.parse::<u32>().ok()?})),
        ["", "", m, d] => Some(json!({"month": m.parse::<u32>().ok()?, "day": d.parse::<u32>().ok()?})),
        _ => None,
    }
}

/// The writable part of a person, always carrying every managed array so removals are sent.
pub fn fields_to_person(fields: &ContactFields) -> Value {
    let mut words: Vec<&str> = fields.name.split_whitespace().collect();
    let names = if words.is_empty() {
        json!([])
    } else if words.len() == 1 {
        json!([{ "givenName": words[0] }])
    } else {
        let family = words.pop().unwrap_or_default();
        json!([{ "givenName": words.join(" "), "familyName": family }])
    };
    json!({
        "names": names,
        "emailAddresses": typed(&fields.emails, "value"),
        "phoneNumbers": typed(&fields.phones, "value"),
        "addresses": typed(&fields.addresses, "streetAddress"),
        "organizations": if fields.org.trim().is_empty() && fields.title.trim().is_empty() { json!([]) } else { json!([{ "name": fields.org, "title": fields.title }]) },
        "birthdays": birthday_date(&fields.birthday).map(|date| json!([{ "date": date }])).unwrap_or_else(|| json!([])),
        "biographies": if fields.notes.trim().is_empty() { json!([]) } else { json!([{ "value": fields.notes, "contentType": "TEXT_PLAIN" }]) },
    })
}

/// A raw element as it can be sent back: output-only members are dropped, `primary` is kept.
fn writable(item: &Value) -> Value {
    let mut item = item.clone();
    let primary = item["metadata"]["primary"].as_bool().unwrap_or(false);
    if let Some(map) = item.as_object_mut() {
        for key in OUTPUT_ONLY {
            map.remove(key);
        }
        if primary {
            map.insert("metadata".into(), json!({"primary": true}));
        }
    }
    item
}

/// Like `fields_to_person`, but a group whose value did not change since it was read keeps its
/// original structure (middle names, address parts, other organizations, types).
pub fn fields_to_person_preserving(fields: &ContactFields, unmanaged: &Value) -> Value {
    let mut person = fields_to_person(fields);
    let raw = &unmanaged["_raw"];
    if !raw.is_object() {
        return person;
    }
    let before = person_to_remote(raw).fields;
    let keep = |person: &mut Value, key: &str, unchanged: bool| {
        if unchanged {
            if let Some(items) = raw[key].as_array() {
                person[key] = Value::Array(items.iter().map(writable).collect());
            }
        }
    };
    keep(&mut person, "names", before.name == fields.name);
    keep(&mut person, "emailAddresses", before.emails == fields.emails);
    keep(&mut person, "phoneNumbers", before.phones == fields.phones);
    keep(&mut person, "addresses", before.addresses == fields.addresses);
    keep(&mut person, "birthdays", before.birthday == fields.birthday);
    keep(&mut person, "biographies", before.notes == fields.notes);
    if before.org == fields.org && before.title == fields.title {
        keep(&mut person, "organizations", true);
    } else if let Some(items) = raw["organizations"].as_array().filter(|a| !a.is_empty()) {
        // Only the first organization is edited here; the others stay as they are.
        let mut items: Vec<Value> = items.iter().map(writable).collect();
        for (key, value) in [("name", &fields.org), ("title", &fields.title)] {
            match items[0].as_object_mut() {
                Some(map) if value.trim().is_empty() => {
                    map.remove(key);
                }
                Some(map) => {
                    map.insert(key.into(), json!(value));
                }
                None => {}
            }
        }
        if items[0].as_object().is_some_and(|m| !m.contains_key("name") && !m.contains_key("title")) {
            items.remove(0);
        }
        person["organizations"] = Value::Array(items);
    }
    person
}

/// Path and query parameters of a list request.
pub fn list_request(other: bool, page: Option<&str>, sync_token: Option<&str>) -> (String, Vec<(String, String)>) {
    let (path, mask) = if other { ("/otherContacts", ("readMask", OTHER_FIELDS)) } else { ("/people/me/connections", ("personFields", PERSON_FIELDS)) };
    let mut params = vec![(mask.0.to_string(), mask.1.to_string()), ("pageSize".into(), "1000".into()), ("requestSyncToken".into(), "true".into())];
    if let Some(token) = sync_token {
        params.push(("syncToken".into(), token.into()));
    }
    if let Some(page) = page {
        params.push(("pageToken".into(), page.into()));
    }
    (path.to_string(), params)
}

/// Maps an HTTP failure to the error codes the service understands.
pub fn map_error(status: u16, body: &Value) -> AppError {
    let message = body["error"]["message"].as_str().unwrap_or("Google Contacts rejected the operation").to_string();
    let lowered = message.to_lowercase();
    let code = match status {
        401 => "consent_required",
        403 if lowered.contains("scope") || lowered.contains("insufficient") => "consent_required",
        403 => "contacts_permission_denied",
        404 => "not_found",
        410 => "sync_token_expired",
        400 | 412 if body.to_string().contains("EXPIRED_SYNC_TOKEN") => "sync_token_expired",
        400 | 412 if lowered.contains("etag") || body.to_string().to_lowercase().contains("failed_precondition") || body.to_string().contains("failedPrecondition") => "etag_conflict",
        429 => "rate_limited",
        _ => "contacts_provider_error",
    };
    AppError::new(code, message)
}

fn resource(remote_id: &str) -> AppResult<&str> {
    let tail = remote_id.strip_prefix("people/").or_else(|| remote_id.strip_prefix("otherContacts/"));
    match tail {
        Some(id) if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) => Ok(remote_id),
        _ => Err(AppError::validation("Unexpected contact identifier from Google")),
    }
}

type TokenSource = Arc<dyn Fn(&Account) -> AppResult<String> + Send + Sync>;
type TokenCache = Arc<Mutex<HashMap<u32, (String, Instant)>>>;

pub struct GooglePeopleGateway {
    secrets: Arc<dyn SecretStore>,
    base: String,
    token: TokenSource,
    cache: TokenCache,
}

fn credentials(secrets: &dyn SecretStore, account: &Account) -> AppResult<(String, String)> {
    client_credentials(account, secrets.get(GOOGLE_SECRET)?)
}

/// For the consent the person gives in the browser: credentials of another Windows user count as
/// none, and the token saved afterwards sets them aside.
fn sign_in_credentials(secrets: &dyn SecretStore, account: &Account) -> AppResult<(String, String)> {
    secrets.check_writable()?;
    client_credentials(account, secrets.get_for_sign_in(GOOGLE_SECRET)?)
}

fn client_credentials(account: &Account, saved_secret: String) -> AppResult<(String, String)> {
    if account.oauth_client_id.is_empty() {
        return Err(AppError::new("sign_in_required", "Use Google OAuth sign-in in Settings first"));
    }
    let secret = super::google_oauth_config::secret_for_client(&account.oauth_client_id, &saved_secret);
    Ok((account.oauth_client_id.clone(), secret))
}

impl GooglePeopleGateway {
    pub fn new(secrets: Arc<dyn SecretStore>) -> Self {
        let cache: TokenCache = Arc::new(Mutex::new(HashMap::new()));
        let (source_secrets, source_cache) = (secrets.clone(), cache.clone());
        let token: TokenSource = Arc::new(move |account: &Account| {
            if let Some((token, until)) = source_cache.lock().ok().and_then(|c| c.get(&account.id).cloned()) {
                if Instant::now() < until {
                    return Ok(token);
                }
            }
            let refresh = source_secrets.get(&format!("contacts:{}", account.id))?;
            if refresh.is_empty() {
                return Err(AppError::new("consent_required", "Allow access to Google Contacts for this account first"));
            }
            let (client, secret) = credentials(source_secrets.as_ref(), account)?;
            let tokens = super::oauth::refresh(super::oauth::TOKEN_URL, &client, &secret, &refresh).map_err(|e| AppError::new("consent_required", e))?;
            let until = Instant::now() + Duration::from_secs(tokens.expires_in.saturating_sub(60));
            if let Ok(mut cache) = source_cache.lock() {
                cache.insert(account.id, (tokens.access.clone(), until));
            }
            Ok(tokens.access)
        });
        Self { secrets, base: API.into(), token, cache }
    }

    /// Test hook: a fixed base URL and access token.
    pub fn with_base(secrets: Arc<dyn SecretStore>, base: &str, token: &str) -> Self {
        let fixed = token.to_string();
        Self { secrets, base: base.trim_end_matches('/').into(), token: Arc::new(move |_| Ok(fixed.clone())), cache: Arc::new(Mutex::new(HashMap::new())) }
    }

    fn request(&self, account: &Account, method: &str, path: &str, params: &[(String, String)], body: Option<&Value>) -> AppResult<Value> {
        let access = (self.token)(account)?;
        let query = params.iter().map(|(k, v)| format!("{}={}", super::google_calendar::encode(k), super::google_calendar::encode(v))).collect::<Vec<_>>().join("&");
        let url = format!("{}{}{}{}", self.base, path, if query.is_empty() { "" } else { "?" }, query);
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).http_status_as_error(false).build().into();
        let auth = format!("Bearer {access}");
        let empty = json!({});
        let sent = match method {
            "GET" => agent.get(&url).header("Authorization", &auth).call(),
            "POST" => agent.post(&url).header("Authorization", &auth).send_json(body.unwrap_or(&empty)),
            "PATCH" => agent.patch(&url).header("Authorization", &auth).send_json(body.unwrap_or(&empty)),
            "DELETE" => agent.delete(&url).header("Authorization", &auth).call(),
            _ => return Err(AppError::validation("Unsupported contacts HTTP method")),
        };
        let mut response = sent.map_err(|e| AppError::new("provider_unavailable", format!("Google Contacts request did not complete: {e}")))?;
        let status = response.status().as_u16();
        let raw = response.body_mut().read_to_string().unwrap_or_default();
        let value: Value = if raw.trim().is_empty() { json!({}) } else { serde_json::from_str(&raw).unwrap_or(Value::Null) };
        if status >= 400 {
            return Err(map_error(status, &value));
        }
        Ok(value)
    }

    fn person_query() -> Vec<(String, String)> {
        vec![("personFields".into(), PERSON_FIELDS.into())]
    }
}

impl ContactGateway for GooglePeopleGateway {
    fn list(&self, account: &Account, other: bool, page: Option<&str>, sync_token: Option<&str>) -> AppResult<RemotePage> {
        let sync_token = match (other, sync_token) {
            (false, Some(stored)) => Some(stored.strip_prefix(CONTACT_TOKEN_PREFIX)
                .ok_or_else(|| AppError::new("sync_token_expired", "Contacts field mask changed"))?),
            _ => sync_token,
        };
        let (path, params) = list_request(other, page, sync_token);
        let reply = self.request(account, "GET", &path, &params, None)?;
        let key = if other { "otherContacts" } else { "connections" };
        Ok(RemotePage {
            items: reply[key].as_array().map(|items| items.iter().map(person_to_remote).collect()).unwrap_or_default(),
            next_page: reply["nextPageToken"].as_str().filter(|t| !t.is_empty()).map(String::from),
            next_sync_token: reply["nextSyncToken"].as_str().filter(|t| !t.is_empty()).map(|token| {
                if other { token.to_string() } else { format!("{CONTACT_TOKEN_PREFIX}{token}") }
            }),
        })
    }
    fn create(&self, account: &Account, fields: &ContactFields, request_id: &str) -> AppResult<RemoteContact> {
        let mut person = fields_to_person(fields);
        person["clientData"] = json!([{"key": crate::application::contact_ports::CREATE_REQUEST_KEY, "value": request_id}]);
        let reply = self.request(account, "POST", "/people:createContact", &Self::person_query(), Some(&person))?;
        let remote = person_to_remote(&reply);
        if remote.remote_id.is_empty() { return Err(AppError::external("Google returned a contact without a resource name")); }
        Ok(remote)
    }
    fn update(&self, account: &Account, remote_id: &str, etag: &str, fields: &ContactFields, unmanaged: &Value) -> AppResult<RemoteContact> {
        let path = format!("/{}:updateContact", resource(remote_id)?);
        let mut person = fields_to_person_preserving(fields, unmanaged);
        let id = remote_id.strip_prefix("people/").unwrap_or(remote_id);
        person["metadata"] = json!({"sources": [{"type": "CONTACT", "id": id, "etag": etag}]});
        let mut params = Self::person_query();
        params.push(("updatePersonFields".into(), WRITE_FIELDS.into()));
        Ok(person_to_remote(&self.request(account, "PATCH", &path, &params, Some(&person))?))
    }
    fn delete(&self, account: &Account, remote_id: &str) -> AppResult<()> {
        self.request(account, "DELETE", &format!("/{}:deleteContact", resource(remote_id)?), &[], None).map(|_| ())
    }
    fn get(&self, account: &Account, remote_id: &str) -> AppResult<RemoteContact> {
        Ok(person_to_remote(&self.request(account, "GET", &format!("/{}", resource(remote_id)?), &Self::person_query(), None)?))
    }
    fn promote_other(&self, account: &Account, remote_id: &str) -> AppResult<RemoteContact> {
        let path = format!("/{}:copyOtherContactToMyContactsGroup", resource(remote_id)?);
        let body = json!({"copyMask": ["names", "emailAddresses", "phoneNumbers"], "readMask": PERSON_FIELDS});
        Ok(person_to_remote(&self.request(account, "POST", &path, &[], Some(&body))?))
    }
    fn authorize(&self, account: &Account, write: bool) -> AppResult<()> {
        let (client, secret) = sign_in_credentials(self.secrets.as_ref(), account)?;
        let scope = if write { WRITE_SCOPE } else { READ_SCOPE };
        let (email, tokens) = super::oauth::sign_in_with_scope(&client, &secret, &AtomicBool::new(false), scope, &account.email).map_err(AppError::external)?;
        if !email.eq_ignore_ascii_case(&account.email) {
            return Err(AppError::validation("The Google sign-in used a different email; no credentials were changed"));
        }
        self.secrets.set(&format!("contacts:{}", account.id), &tokens.refresh)?;
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove(&account.id);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn labeled(label: &str, value: &str) -> Labeled {
        Labeled { label: label.into(), value: value.into() }
    }
    fn full_person() -> Value {
        json!({
            "resourceName": "people/c1",
            "etag": "%EgUBAj0",
            "metadata": {"sources": [{"type": "PROFILE", "id": "p9", "etag": "profileEtag"}, {"type": "CONTACT", "id": "c1", "etag": "sourceEtag"}]},
            "names": [{"displayName": "Ada Lovelace", "givenName": "Ada", "familyName": "Lovelace"}],
            "emailAddresses": [{"value": "ada@x.it", "type": "work"}, {"value": "ada@y.it", "formattedType": "Home"}, {"value": "ada@z.it"}],
            "phoneNumbers": [{"value": "+39 333 1234567", "type": "mobile"}],
            "addresses": [{"formattedValue": "Via Roma 1\nMilano", "type": "home"}],
            "organizations": [{"name": "Acme", "title": "Analista"}],
            "birthdays": [{"date": {"year": 1815, "month": 12, "day": 10}}],
            "biographies": [{"value": "Prima programmatrice"}],
            "memberships": [{"contactGroupMembership": {"contactGroupId": "family"}}],
            "photos": [{"url": "https://x.test/p.png"}]
        })
    }

    #[test]
    fn a_person_maps_to_every_managed_field() {
        let remote = person_to_remote(&full_person());
        assert_eq!((remote.remote_id.as_str(), remote.etag.as_str(), remote.deleted), ("people/c1", "sourceEtag", false));
        let f = remote.fields;
        assert_eq!((f.name.as_str(), f.org.as_str(), f.title.as_str(), f.birthday.as_str(), f.notes.as_str()), ("Ada Lovelace", "Acme", "Analista", "1815-12-10", "Prima programmatrice"));
        assert_eq!(f.emails, vec![labeled("work", "ada@x.it"), labeled("home", "ada@y.it"), labeled("", "ada@z.it")]);
        assert_eq!(f.phones, vec![labeled("mobile", "+39 333 1234567")]);
        assert_eq!(f.addresses, vec![labeled("home", "Via Roma 1\nMilano")]);
    }

    #[test]
    fn unmanaged_keys_are_kept_and_managed_ones_are_not() {
        let remote = person_to_remote(&full_person());
        let keys: Vec<&String> = remote.unmanaged.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["_raw", "memberships", "photos"]);
        assert_eq!(person_to_remote(&json!({"resourceName": "people/c2", "etag": "x", "names": []})).unmanaged, Value::Null);
    }

    #[test]
    fn a_deleted_person_is_flagged() {
        let remote = person_to_remote(&json!({"resourceName": "people/c3", "metadata": {"deleted": true}}));
        assert!(remote.deleted);
    }

    #[test]
    fn fields_round_trip_through_person_json() {
        let original = person_to_remote(&full_person()).fields;
        let mut person = fields_to_person(&original);
        person["resourceName"] = json!("people/c1");
        person["etag"] = json!("x");
        for address in person["addresses"].as_array_mut().unwrap() {
            let street = address["streetAddress"].clone();
            address["formattedValue"] = street;
        }
        assert_eq!(person_to_remote(&person).fields, original);
    }

    #[test]
    fn empty_fields_still_send_every_managed_array_so_removals_reach_google() {
        let person = fields_to_person(&ContactFields::default());
        for key in ["names", "emailAddresses", "phoneNumbers", "addresses", "organizations", "birthdays", "biographies"] {
            assert_eq!(person[key], json!([]), "{key}");
        }
        for key in ["memberships", "photos", "etag", "resourceName"] {
            assert!(person.get(key).is_none(), "{key} must not be written by the field mapper");
        }
    }

    #[test]
    fn names_are_split_into_given_and_family_and_birthdays_without_a_year_work() {
        let mut fields = ContactFields { name: "Maria Teresa Verdi".into(), birthday: "--03-04".into(), ..Default::default() };
        let person = fields_to_person(&fields);
        assert_eq!(person["names"][0]["givenName"], "Maria Teresa");
        assert_eq!(person["names"][0]["familyName"], "Verdi");
        assert_eq!(person["birthdays"][0]["date"], json!({"month": 3, "day": 4}));
        fields.birthday = "not a date".into();
        assert_eq!(fields_to_person(&fields)["birthdays"], json!([]));
    }

    #[test]
    fn list_requests_ask_for_a_sync_token_and_pass_the_stored_one() {
        let (path, params) = list_request(false, None, None);
        assert_eq!(path, "/people/me/connections");
        assert!(params.contains(&("requestSyncToken".into(), "true".into())));
        assert!(params.contains(&("pageSize".into(), "1000".into())));
        assert!(params.iter().any(|(k, v)| k == "personFields" && v.contains("emailAddresses")));
        assert!(!params.iter().any(|(k, _)| k == "syncToken" || k == "pageToken"));
        let (_, params) = list_request(false, Some("p2"), Some("T9"));
        assert!(params.contains(&("syncToken".into(), "T9".into())) && params.contains(&("pageToken".into(), "p2".into())));
        let (path, params) = list_request(true, None, None);
        assert_eq!(path, "/otherContacts");
        assert!(params.iter().any(|(k, v)| k == "readMask" && v.contains("names")));
    }

    #[test]
    fn http_failures_map_to_service_codes() {
        let body = |message: &str| json!({"error": {"message": message, "status": "X"}});
        assert_eq!(map_error(401, &body("expired")).code, "consent_required");
        assert_eq!(map_error(403, &body("Request had insufficient authentication scopes.")).code, "consent_required");
        assert_eq!(map_error(404, &body("gone")).code, "not_found");
        assert_eq!(map_error(410, &body("gone")).code, "sync_token_expired");
        assert_eq!(map_error(400, &body("Sync token is expired. EXPIRED_SYNC_TOKEN")).code, "sync_token_expired");
        assert_eq!(map_error(400, &body("Request person.etag is different than the current person.etag")).code, "etag_conflict");
        assert_eq!(map_error(412, &body("etag")).code, "etag_conflict");
        assert_eq!(map_error(429, &body("slow down")).code, "rate_limited");
        assert_eq!(map_error(500, &body("boom")).code, "contacts_provider_error");
    }

    #[test]
    fn a_person_without_source_metadata_falls_back_to_the_person_etag() {
        let remote = person_to_remote(&json!({"resourceName": "people/c2", "etag": "personEtag", "names": [{"displayName": "Bea"}]}));
        assert_eq!(remote.etag, "personEtag");
    }

    #[test]
    fn the_failed_precondition_of_a_stale_source_etag_is_an_etag_conflict() {
        let body = json!({"error": {"code": 400, "status": "FAILED_PRECONDITION", "message": "Precondition check failed.", "details": [{"reason": "failedPrecondition"}]}});
        assert_eq!(map_error(400, &body).code, "etag_conflict");
        let expired = json!({"error": {"code": 400, "status": "FAILED_PRECONDITION", "message": "x", "details": [{"reason": "EXPIRED_SYNC_TOKEN"}]}});
        assert_eq!(map_error(400, &expired).code, "sync_token_expired");
    }

    fn rich_person() -> Value {
        json!({
            "resourceName": "people/c7", "etag": "pe", "metadata": {"sources": [{"type": "CONTACT", "id": "c7", "etag": "se"}]},
            "names": [{"displayName": "Dott. Maria Teresa Verdi", "givenName": "Maria", "middleName": "Teresa", "familyName": "Verdi", "honorificPrefix": "Dott.", "metadata": {"primary": true, "source": {"type": "CONTACT"}}}],
            "addresses": [{"formattedValue": "Via Roma 1
20100 Milano MI
Italia", "streetAddress": "Via Roma 1", "city": "Milano", "postalCode": "20100", "country": "Italia", "type": "home", "metadata": {"primary": true}}],
            "organizations": [{"name": "Acme", "title": "Analista"}, {"name": "Beta Srl", "title": "Consulente"}],
            "emailAddresses": [{"value": "m@x.it", "type": "work", "formattedType": "Work"}]
        })
    }
    fn sent_body(person: &Value, change: impl FnOnce(&mut ContactFields)) -> Value {
        let remote = person_to_remote(person);
        let mut fields = remote.fields.clone();
        change(&mut fields);
        let answer = json!({"resourceName": "people/c7", "etag": "n", "metadata": {"sources": [{"type": "CONTACT", "etag": "n2"}]}}).to_string();
        let (base, seen) = serve(vec![(200, answer)]);
        let gateway = GooglePeopleGateway::with_base(Arc::new(NoSecrets), &base, "t");
        gateway.update(&account(), "people/c7", &remote.etag, &fields, &remote.unmanaged).unwrap();
        let request = seen.lock().unwrap()[0].clone();
        serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap()
    }

    #[test]
    fn groups_the_user_did_not_touch_are_sent_back_with_their_structure() {
        let body = sent_body(&rich_person(), |f| f.notes = "una nota".into());
        assert_eq!(body["names"][0]["middleName"], "Teresa");
        assert_eq!(body["names"][0]["honorificPrefix"], "Dott.");
        assert!(body["names"][0].get("displayName").is_none(), "output-only fields are not sent");
        assert_eq!(body["names"][0]["metadata"], json!({"primary": true}));
        assert_eq!(body["addresses"][0]["city"], "Milano");
        assert_eq!(body["addresses"][0]["postalCode"], "20100");
        assert!(body["addresses"][0].get("formattedValue").is_none());
        assert_eq!(body["emailAddresses"][0]["type"], "work");
        assert_eq!(body["biographies"][0]["value"], "una nota");
    }

    #[test]
    fn editing_the_title_keeps_the_other_organizations() {
        let body = sent_body(&rich_person(), |f| f.title = "Direttore".into());
        let orgs = body["organizations"].as_array().unwrap();
        assert_eq!(orgs.len(), 2, "{orgs:?}");
        assert_eq!((orgs[0]["name"].as_str(), orgs[0]["title"].as_str()), (Some("Acme"), Some("Direttore")));
        assert_eq!(orgs[1]["name"], "Beta Srl");
    }

    #[test]
    fn an_edited_group_is_regenerated_and_the_others_stay_structured() {
        let body = sent_body(&rich_person(), |f| f.addresses[0].value = "Via Verdi 9, Torino".into());
        assert_eq!(body["addresses"], json!([{"streetAddress": "Via Verdi 9, Torino", "type": "home"}]));
        assert_eq!(body["names"][0]["middleName"], "Teresa");
    }

    #[test]
    fn clearing_a_group_sends_an_empty_array_so_the_removal_reaches_google() {
        let body = sent_body(&rich_person(), |f| f.emails.clear());
        assert_eq!(body["emailAddresses"], json!([]));
    }

    /// Serves canned responses and records each request head and body.
    fn serve(responses: Vec<(u16, String)>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        std::thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = vec![0u8; 16384];
                let mut length = 0;
                loop {
                    let n = stream.read(&mut buffer[length..]).unwrap();
                    length += n;
                    let text = String::from_utf8_lossy(&buffer[..length]).to_string();
                    if let Some(split) = text.find("\r\n\r\n") {
                        let wanted = text.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                        if length >= split + 4 + wanted || n == 0 {
                            break;
                        }
                    } else if n == 0 {
                        break;
                    }
                }
                record.lock().unwrap().push(String::from_utf8_lossy(&buffer[..length]).to_string());
                let reply = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(reply.as_bytes()).unwrap();
            }
        });
        (address, seen)
    }
    fn account() -> Account {
        Account { id: 1, email: "alex@work.test".into(), oauth_client_id: "client".into(), ..Default::default() }
    }
    struct NoSecrets;
    impl SecretStore for NoSecrets {
        fn get(&self, _: &str) -> AppResult<String> {
            Ok(String::new())
        }
        fn set(&self, _: &str, _: &str) -> AppResult<()> {
            Ok(())
        }
    }

    #[test]
    fn list_sends_the_token_and_parses_pages() {
        let page = json!({"connections": [full_person()], "nextPageToken": "p2", "nextSyncToken": null}).to_string();
        let (base, seen) = serve(vec![(200, page)]);
        let gateway = GooglePeopleGateway::with_base(Arc::new(NoSecrets), &base, "access-1");
        assert_eq!(gateway.list(&account(), false, None, Some("T9")).unwrap_err().code, "sync_token_expired");
        let reply = gateway.list(&account(), false, None, Some("client-data-v1:T9")).unwrap();
        assert_eq!((reply.items.len(), reply.next_page.as_deref(), reply.next_sync_token), (1, Some("p2"), None));
        let head = seen.lock().unwrap()[0].clone();
        assert!(head.starts_with("GET /people/me/connections?"), "{head}");
        assert!(head.contains("syncToken=T9") && head.contains("requestSyncToken=true"), "{head}");
        assert!(head.to_lowercase().contains("authorization: bearer access-1"), "{head}");
    }

    #[test]
    fn create_sends_an_identity_that_can_be_read_back_after_a_lost_reply() {
        let answer = json!({"resourceName": "people/c1", "clientData": [{"key": "qamille.contact.create", "value": "request-7"}]});
        let (base, seen) = serve(vec![(200, answer.to_string())]);
        let gateway = GooglePeopleGateway::with_base(Arc::new(NoSecrets), &base, "t");
        let created = gateway.create(&account(), &ContactFields::default(), "request-7").unwrap();
        assert_eq!(created.create_request(), Some("request-7"));
        let request = seen.lock().unwrap()[0].clone();
        assert!(request.starts_with("POST /people:createContact?") && request.contains("clientData"));
        let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["clientData"], answer["clientData"]);
    }

    #[test]
    fn update_patches_with_the_etag_and_only_the_managed_fields() {
        let answer = json!({"resourceName": "people/c1", "etag": "new", "names": [{"displayName": "Ada"}]}).to_string();
        let (base, seen) = serve(vec![(200, answer)]);
        let gateway = GooglePeopleGateway::with_base(Arc::new(NoSecrets), &base, "t");
        let fields = ContactFields { name: "Ada".into(), ..Default::default() };
        let remote = gateway.update(&account(), "people/c1", "old-etag", &fields, &json!({"memberships": []})).unwrap();
        assert_eq!(remote.etag, "new");
        let request = seen.lock().unwrap()[0].clone();
        assert!(request.starts_with("PATCH /people/c1:updateContact?"), "{request}");
        assert!(request.contains("updatePersonFields=names%2CemailAddresses") || request.contains("updatePersonFields=names,emailAddresses"), "{request}");
        let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["metadata"]["sources"], json!([{"type": "CONTACT", "id": "c1", "etag": "old-etag"}]));
        assert!(body.get("memberships").is_none(), "unmanaged data is never sent");
    }

    #[test]
    fn a_stale_etag_response_becomes_etag_conflict() {
        let error = json!({"error": {"code": 400, "message": "Request person.etag is different than the current person.etag", "status": "FAILED_PRECONDITION"}}).to_string();
        let (base, _) = serve(vec![(400, error)]);
        let gateway = GooglePeopleGateway::with_base(Arc::new(NoSecrets), &base, "t");
        let failure = gateway.update(&account(), "people/c1", "old", &ContactFields::default(), &Value::Null).unwrap_err();
        assert_eq!(failure.code, "etag_conflict");
    }

    #[test]
    fn delete_and_promote_use_the_documented_endpoints() {
        let promoted = json!({"resourceName": "people/c9", "etag": "e", "names": [{"displayName": "Dan"}]}).to_string();
        let (base, seen) = serve(vec![(200, "{}".into()), (200, promoted)]);
        let gateway = GooglePeopleGateway::with_base(Arc::new(NoSecrets), &base, "t");
        gateway.delete(&account(), "people/c1").unwrap();
        let remote = gateway.promote_other(&account(), "otherContacts/c5").unwrap();
        assert_eq!(remote.remote_id, "people/c9");
        let requests = seen.lock().unwrap().clone();
        assert!(requests[0].starts_with("DELETE /people/c1:deleteContact"), "{}", requests[0]);
        assert!(requests[1].starts_with("POST /otherContacts/c5:copyOtherContactToMyContactsGroup"), "{}", requests[1]);
    }
}
