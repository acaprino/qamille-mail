//! MCP tools for the unified address book. They can read, create, edit and link;
//! none of them deletes a contact or merges cards on a server.
use crate::{
    application::{
        AppError, AppResult, ServiceContext,
        ports::StoreRepository,
        contact_ports::{ContactGateway, RemoteContact, RemotePage},
        contacts::{ContactQuery, ContactService, ContactView},
    },
    domain::{
        contacts::{BookKind, ContactFields, Labeled},
        model::{Account, Store},
    },
    infrastructure::{
        contact_repository::FileContactStore, google_people::GooglePeopleGateway, repository::JsonStoreRepository,
    },
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Google gateway for MCP: everything works except opening a browser for consent.
struct McpGateway(GooglePeopleGateway);
impl ContactGateway for McpGateway {
    fn list(&self, a: &Account, other: bool, page: Option<&str>, token: Option<&str>) -> AppResult<RemotePage> {
        self.0.list(a, other, page, token)
    }
    fn create(&self, a: &Account, f: &ContactFields) -> AppResult<RemoteContact> {
        self.0.create(a, f)
    }
    fn update(&self, a: &Account, id: &str, etag: &str, f: &ContactFields, u: &Value) -> AppResult<RemoteContact> {
        self.0.update(a, id, etag, f, u)
    }
    fn delete(&self, a: &Account, id: &str) -> AppResult<()> {
        self.0.delete(a, id)
    }
    fn get(&self, a: &Account, id: &str) -> AppResult<RemoteContact> {
        self.0.get(a, id)
    }
    fn promote_other(&self, a: &Account, id: &str) -> AppResult<RemoteContact> {
        self.0.promote_other(a, id)
    }
    fn authorize(&self, _: &Account, _: bool) -> AppResult<()> {
        Err(AppError::new("consent_required", "Allow writing to this address book in Qamille Settings first"))
    }
}

fn service(context: &Arc<ServiceContext>) -> AppResult<ContactService> {
    // A profile limited to some accounts sees fewer accounts than the stored workspace has.
    let all_accounts = JsonStoreRepository::new(Store::path()).read()?.accounts.len();
    let service = ContactService::new(
        context.clone(),
        Arc::new(FileContactStore::new(Store::data_dir())),
        Arc::new(McpGateway(GooglePeopleGateway::new(context.secrets.clone()))),
    );
    Ok(if context.read()?.accounts.len() < all_accounts { service.without_local() } else { service })
}

pub fn handles(name: &str) -> bool {
    matches!(name, "search_contacts" | "list_address_books" | "get_contact" | "find_duplicates" | "create_contact" | "update_contact" | "link_contacts")
}

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args[key].as_str().map(str::trim).filter(|s| !s.is_empty())
}
fn strings(args: &Value, key: &str) -> Option<Vec<String>> {
    args[key].as_array().map(|items| items.iter().filter_map(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
}
fn labeled(values: Vec<String>) -> Vec<Labeled> {
    values.into_iter().map(|value| Labeled { label: String::new(), value }).collect()
}
fn account(args: &Value) -> Option<u32> {
    args["account_id"].as_u64().or_else(|| args["account_id"].as_str()?.parse().ok()).and_then(|n| u32::try_from(n).ok())
}

fn item(view: &ContactView) -> Value {
    let only_mail = view.origins.iter().all(|o| o.book.starts_with("mail:"));
    json!({
        "id": view.id,
        "name": view.fields.name,
        "address": view.fields.emails.first().map(|e| e.value.clone()).unwrap_or_default(),
        "emails": view.fields.emails.iter().map(|e| &e.value).collect::<Vec<_>>(),
        "phones": view.fields.phones.iter().map(|p| &p.value).collect::<Vec<_>>(),
        "org": view.fields.org,
        "source": if only_mail { "observed_mail" } else { "address_book" },
        "sources": view.origins.iter().map(|o| json!({"book": o.book, "label": o.label, "writable": o.writable})).collect::<Vec<_>>(),
    })
}

/// Fields the caller provided replace the matching ones of `base`; absent keys stay.
fn fields_from(args: &Value, mut base: ContactFields) -> ContactFields {
    if let Some(v) = args["name"].as_str() {
        base.name = v.trim().to_string();
    }
    if let Some(v) = args["org"].as_str() {
        base.org = v.trim().to_string();
    }
    if let Some(v) = args["title"].as_str() {
        base.title = v.trim().to_string();
    }
    if let Some(v) = args["birthday"].as_str() {
        base.birthday = v.trim().to_string();
    }
    if let Some(v) = args["notes"].as_str() {
        base.notes = v.to_string();
    }
    if let Some(v) = strings(args, "emails") {
        base.emails = labeled(v);
    }
    if let Some(v) = strings(args, "phones") {
        base.phones = labeled(v);
    }
    base
}

pub fn call(context: &Arc<ServiceContext>, name: &str, args: &Value) -> AppResult<Value> {
    let contacts = service(context)?;
    match name {
        "search_contacts" => {
            let query = text(args, "query").unwrap_or("").to_string();
            let wanted = account(args);
            let page = contacts.list(ContactQuery { text: query, limit: 500, ..Default::default() })?;
            let rows: Vec<Value> = page
                .items
                .iter()
                .filter(|c| wanted.is_none_or(|id| c.origins.iter().any(|o| o.book == "local" || o.book.split(':').nth(1).and_then(|n| n.parse::<u32>().ok()) == Some(id))))
                .map(item)
                .collect();
            Ok(json!({ "rows": rows }))
        }
        "list_address_books" => {
            let rows: Vec<Value> = contacts.books()?.iter().map(|b| json!({"id": b.id, "label": b.label, "kind": format!("{:?}", b.kind), "writable": b.writable && (b.kind == BookKind::Local || b.write_granted), "sync_enabled": b.sync_enabled, "last_sync": b.last_sync})).collect();
            Ok(json!({ "rows": rows }))
        }
        "get_contact" => Ok(json!({ "one": serde_json::to_value(contacts.get(text(args, "id").ok_or_else(|| AppError::validation("id is required"))?)?)? })),
        "find_duplicates" => {
            let rows: Vec<Value> = contacts
                .duplicates(args["include_possible"].as_bool().unwrap_or(false))?
                .into_iter()
                .map(|g| json!({"id": g.id, "level": format!("{:?}", g.level), "members": g.members.iter().map(|m| json!({"key": m.key, "name": m.fields.name, "emails": m.fields.emails.iter().map(|e| &e.value).collect::<Vec<_>>(), "label": m.label})).collect::<Vec<_>>()}))
                .collect();
            Ok(json!({ "rows": rows }))
        }
        "create_contact" => {
            let book = text(args, "book").ok_or_else(|| AppError::new("book_required", "Choose an address book"))?;
            let created = contacts.create(book, fields_from(args, ContactFields::default()))?;
            let _ = contacts.flush_queue();
            Ok(json!({ "one": serde_json::to_value(contacts.get(&created.id).unwrap_or(created))? }))
        }
        "update_contact" => {
            let id = text(args, "id").ok_or_else(|| AppError::validation("id is required"))?;
            let current = contacts.get(id)?;
            let updated = contacts.update(id, fields_from(args, current.fields))?;
            let _ = contacts.flush_queue();
            Ok(json!({ "one": serde_json::to_value(contacts.get(&updated.id).unwrap_or(updated))? }))
        }
        "link_contacts" => {
            let keys = strings(args, "keys").unwrap_or_default();
            contacts.same_person(keys.clone())?;
            Ok(json!({ "one": {"linked": keys} }))
        }
        _ => Err(AppError::new("unknown_tool", format!("Unknown tool {name}"))),
    }
}
