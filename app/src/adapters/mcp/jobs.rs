//! Durable handles for calls that exceed the synchronous response budget.
use crate::{
    application::{AppError, AppResult},
    model::Store,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

#[derive(Serialize, Deserialize)]
struct Job {
    id: String,
    profile: String,
    grants: String,
    accounts: Vec<u32>,
    tool: String,
    status: String,
    created_at: i64,
    updated_at: i64,
    result: Option<Value>,
}
fn path(id: &str) -> AppResult<std::path::PathBuf> {
    let suffix = id
        .strip_prefix("job:")
        .ok_or_else(|| AppError::validation("Invalid job ID"))?;
    if suffix.len() != 32 || !suffix.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::validation("Invalid job ID"));
    }
    let root = Store::data_dir().join("mcp-jobs");
    std::fs::create_dir_all(&root)?;
    Ok(root.join(format!("{suffix}.json")))
}
fn save(job: &Job) -> AppResult<()> {
    let path = path(&job.id)?;
    let tmp = path.with_extension("tmp");
    let mut file = File::create(&tmp)?;
    file.write_all(&serde_json::to_vec(job)?)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(tmp, path)?;
    Ok(())
}
pub fn get(id: &str, profile: &str, grants: &str, accounts: &[u32]) -> AppResult<Value> {
    let mut job: Job = serde_json::from_slice(&std::fs::read(path(id)?)?)?;
    if job.profile != profile
        || job.grants != grants
        || job.accounts.iter().any(|id| !accounts.contains(id))
    {
        return Err(AppError::new(
            "permission_denied",
            "Job is outside this profile's account grants",
        ));
    }
    if job.status == "running" {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path(id)?.with_extension("lock"))?;
        if lock.try_lock().is_ok() {
            // Completion may have committed after our first read and before the lock.
            job = serde_json::from_slice(&std::fs::read(path(id)?)?)?;
            if job.status == "running" {
                job.status = "interrupted".into();
                job.updated_at = crate::model::now_ts();
                save(&job)?;
            }
        }
    }
    Ok(
        json!({"operation_id":job.id,"tool":job.tool,"status":job.status,"created_at":job.created_at,"updated_at":job.updated_at,"result":job.result,"can_cancel":false,"retryable":false,"notice":if job.status=="interrupted"{Some("The host stopped during this operation. Check its provider/local state before repeating any write.")}else{None}}),
    )
}
pub fn run(server: super::server::Server, name: String, args: Value) -> AppResult<Value> {
    let invoked = Arc::new(AtomicBool::new(false));
    let result = run_inner(server.clone(), name.clone(), args, invoked.clone());
    if result.is_err() && !invoked.load(Ordering::Acquire) {
        server.audit(&name, false);
    }
    result
}
fn run_inner(
    server: super::server::Server,
    name: String,
    args: Value,
    invoked: Arc<AtomicBool>,
) -> AppResult<Value> {
    let profile = server.profile_name().to_owned();
    let grants = server.grant_fingerprint()?;
    let accounts = server.visible_account_ids()?;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| AppError::storage("Job ID unavailable"))?;
    let id = format!(
        "job:{}",
        random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let guard = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path(&id)?.with_extension("lock"))?;
    guard.lock()?;
    let now = crate::model::now_ts();
    let job = Job {
        id: id.clone(),
        profile,
        grants,
        accounts,
        tool: name.clone(),
        status: "running".into(),
        created_at: now,
        updated_at: now,
        result: None,
    };
    save(&job)?;
    let (send, receive) = mpsc::channel();
    std::thread::Builder::new()
        .name("qamille-mcp-call".into())
        .spawn(move || {
            invoked.store(true, Ordering::Release);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                server.call_tool(&name, &args)
            }))
            .unwrap_or_else(|_| {
                server.audit(&name, false);
                Err(AppError::new(
                    "interrupted",
                    "Operation stopped unexpectedly; inspect its state before repeating it",
                ))
            });
            let mut job = job;
            job.status = if result.is_ok() {
                "completed"
            } else {
                "failed"
            }
            .into();
            job.updated_at = crate::model::now_ts();
            job.result = Some(match &result {
                Ok(value) => value.clone(),
                Err(error) => json!({"error":error}),
            });
            let persisted = save(&job);
            drop(guard);
            let result = if let Err(error) = persisted {
                Err(AppError::new(
                    "job_result_not_saved",
                    format!(
                        "Operation finished but its result record could not be saved: {}",
                        error.message
                    ),
                ))
            } else {
                result
            };
            let _ = send.send(result);
        })?;
    match receive.recv_timeout(Duration::from_secs(10)) {
        Ok(result) => {
            // Fast read results are returned directly and do not become a second mail archive.
            let _ = std::fs::remove_file(path(&id)?);
            let _ = std::fs::remove_file(path(&id)?.with_extension("lock"));
            result
        }
        Err(mpsc::RecvTimeoutError::Timeout) => Ok(
            json!({"operation_id":id,"data":{"operation_id":id,"status":"running","poll_tool":"get_operation","can_cancel":false},"meta":{"asynchronous":true}}),
        ),
        Err(_) => Err(AppError::new(
            "interrupted",
            "Operation stopped; inspect its job state before retrying",
        )),
    }
}

pub fn cleanup() -> AppResult<()> {
    let root = Store::data_dir().join("mcp-jobs");
    if !root.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if path.extension().is_none_or(|s| s != "json") {
            continue;
        }
        let bytes = std::fs::read(&path)?;
        let job: Job = serde_json::from_slice(&bytes)?;
        if job.status != "running" && crate::model::now_ts().saturating_sub(job.updated_at) > 86400
        {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}
