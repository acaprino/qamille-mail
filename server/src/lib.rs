//! `qamille-server`: utenti, dispositivi e accesso con password (ADR-0012) e calcolo
//! (embedding, spezzatura, estrazione dei documenti; spec del server di calcolo, D1–D3).
//!
//! Il server ha due ascolti: quello pubblico (`bind`) con le rotte dell'account,
//! della registrazione e del recupero, e quello amministrativo (`admin_bind`,
//! facoltativo) con la pagina `/admin` e `/admin/api/*`.

pub mod accounts;
pub mod cli;
pub mod compute;
pub mod config;
pub mod db;
pub mod embedding;
pub mod error;
pub mod ext;
pub mod http;
pub mod limiter;
pub mod llm;
pub mod llm_builtin;
pub mod origin;
pub mod password;
pub mod process;
pub mod sync;

use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;

pub use accounts::{Accounts, AuthContext, Clock, DeviceInfo, ManualClock, SystemClock};
pub use config::{AdminKey, Config, ConfigError, Edition};
pub use db::{Db, DbError};
pub use error::ServerError;
pub use http::{AdminAuth, AppState, DeviceAuth};

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("non riesco a creare la cartella dei dati {path}: {reason}")]
    DataDir { path: String, reason: String },
    #[error("non riesco ad ascoltare su {addr}: {reason}")]
    Bind { addr: SocketAddr, reason: String },
    #[error("il server si è fermato per un errore: {0}")]
    Serve(String),
    #[error("una rotta di un'estensione c'è già nel core: {0}")]
    DuplicateRoute(String),
}

/// Il server pronto a partire: database aperto e stato dei router. Il core da solo risponde
/// 404 alle rotte dell'enterprise; le estensioni si montano con i metodi `with_*`, in
/// qualunque ordine, e i router si compongono una volta sola alla prima richiesta di uno.
pub struct Server {
    config: Config,
    state: AppState,
    /// Backend delle rotte di calcolo (modalità remota), da [`Server::with_compute`].
    compute_parts: Option<ComputeParts>,
    /// Sink aggiuntivi, oltre ai contatori che il core salva da sé.
    usage_sinks: Vec<Arc<dyn compute::UsageSink>>,
    /// Lo stato del calcolo, costruito alla prima richiesta di un router o dello stato.
    compute: OnceLock<compute::ComputeState>,
    /// Rotte delle estensioni, con il loro stato già applicato, senza fallback né strati.
    public: Vec<Router>,
    admin: Vec<Router>,
    admin_page: ext::AdminPage,
    on_start: Vec<Box<dyn Fn() + Send + Sync>>,
    /// The supervisor of the built-in LLM child, if one was handed in (tests).
    llm_supervisor: Option<Arc<llm_builtin::LlmSupervisor>>,
    /// The installable `llm-*` models, handed in (tests) or built from the configuration.
    llm_models: Option<Arc<dyn compute::llm_models::LlmModels>>,
    real_llm_models: bool,
    /// Il servizio di sincronizzazione delle caselle; `None` senza `QAMILLE_SYNC_KEY`.
    sync: Option<Arc<sync::SyncService>>,
    /// L'assistente della configurazione (file e ambiente), prima delle impostazioni salvate dal
    /// pannello: ci si torna quando il pannello le toglie.
    llm_base: llm::LlmConfig,
}

struct ComputeParts {
    backend: Arc<dyn compute::ComputeBackend>,
    speech: Arc<dyn compute::speech::SpeechBackend>,
    transcription_parallelism: usize,
}

/// I contatori di calcolo per utente vanno nel database, fuori dalla richiesta.
struct AccountsUsage(Arc<Accounts>);

impl compute::UsageSink for AccountsUsage {
    fn record(&self, user_id: &str, usage: compute::Usage) {
        let accounts = Arc::clone(&self.0);
        let user_id = user_id.to_owned();
        let usage = accounts::ComputeUsage {
            requests: usage.requests,
            audio_seconds: usage.audio_seconds,
            embedded_tokens: usage.embedded_tokens,
            llm_prompt_tokens: usage.llm_prompt_tokens,
            llm_completion_tokens: usage.llm_completion_tokens,
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if let Err(error) = accounts.record_usage(&user_id, usage).await {
                    tracing::warn!(code = error.code(), "contatori di calcolo non salvati");
                }
            });
        }
    }
}

/// Ogni consumo va a tutti i sink, nell'ordine: il contatore del core per primo.
struct FanOutUsage(Vec<Arc<dyn compute::UsageSink>>);

impl compute::UsageSink for FanOutUsage {
    fn record(&self, user_id: &str, usage: compute::Usage) {
        for sink in &self.0 {
            sink.record(user_id, usage);
        }
    }
}

impl Server {
    /// Il core da solo: utenti creati dall'amministratore, accesso con password, dispositivi
    /// e calcolo. L'enterprise sta sopra il core, in un crate a parte (il repository enterprise, privato).
    pub fn open(config: Config) -> Result<Self, StartError> {
        Self::open_with_migrations(config, Arc::new(SystemClock), db::CORE_MIGRATIONS)
    }

    /// Come [`Server::open`], con l'orologio iniettato e le migrazioni che il file
    /// `server.db` può contenere (le estensioni aggiungono le loro).
    pub fn open_with_migrations(
        mut config: Config,
        clock: Arc<dyn Clock>,
        migrations: &[db::Migration],
    ) -> Result<Self, StartError> {
        config.validate()?;
        std::fs::create_dir_all(&config.data_dir).map_err(|e| StartError::DataDir {
            path: config.data_dir.display().to_string(),
            reason: e.to_string(),
        })?;
        // Le impostazioni dell'assistente salvate dal pannello valgono sopra il file e l'ambiente.
        let llm_base = config.llm.clone();
        if let Some(settings) = llm::LlmSettings::load(&config.data_dir.join(llm::SETTINGS_FILE)) {
            match settings.applied_to(&config.llm) {
                Ok(llm) => config.llm = llm,
                Err(error) => tracing::error!(%error, "impostazioni dell'assistente del pannello non valide: vale la configurazione"),
            }
        }
        let db = Db::open(&config.database_path(), migrations)?;
        let accounts = Arc::new(Accounts::new(db, &config, Arc::clone(&clock)));
        let sync = sync::SyncService::new(Arc::clone(&accounts), &config).map(Arc::new);
        let admin = config.admin_key.as_ref().map(|key| {
            Arc::new(http::AdminState::new(
                key,
                config.admin_max_failures,
                Duration::from_secs(config.admin_failure_window_secs),
                config.admin_plaintext_from.clone(),
            ))
        });
        let state = AppState {
            accounts,
            trusted_proxies: config.trusted_proxies.clone().into(),
            plaintext_from: config.plaintext_from.clone().into(),
            edition: config.edition,
            admin,
            entitlement: None,
            llm: llm::LlmHolder::default(),
            sync: config.sync.enabled().then(|| Arc::new(sync::SyncInfo { available: true })),
            info: None,
            account: None,
            admin_users: None,
            reauth: None,
        };
        Ok(Server {
            config,
            state,
            compute_parts: None,
            usage_sinks: Vec::new(),
            compute: OnceLock::new(),
            public: Vec::new(),
            admin: Vec::new(),
            admin_page: ext::AdminPage::Core,
            on_start: Vec::new(),
            llm_supervisor: None,
            llm_models: None,
            real_llm_models: false,
            sync,
            llm_base,
        })
    }

    /// Chi decide se un utente può usare il calcolo; senza, ogni utente attivo può.
    pub fn with_entitlement(mut self, entitlement: Arc<dyn ext::Entitlement>) -> Self {
        self.state.entitlement = Some(entitlement);
        self.compute = OnceLock::new();
        self
    }

    /// Un sink in più per i consumi per utente, oltre ai contatori che il core salva nel suo
    /// database (E4). Si può chiamare più volte.
    pub fn with_usage_sink(mut self, sink: Arc<dyn compute::UsageSink>) -> Self {
        self.usage_sinks.push(sink);
        self.compute = OnceLock::new();
        self
    }

    /// Campi in più di `GET /v1/info`.
    pub fn with_info(mut self, extension: ext::InfoExtension) -> Self {
        self.state.info = Some(extension);
        self
    }

    /// Campi in più di `GET /v1/account`.
    pub fn with_account(mut self, extension: ext::AccountExtension) -> Self {
        self.state.account = Some(extension);
        self
    }

    /// Campi in più di `GET /admin/api/users`.
    pub fn with_admin_users(mut self, extension: ext::AdminUsersExtension) -> Self {
        self.state.admin_users = Some(extension);
        self
    }

    /// La conferma della cancellazione dell'account rifacendo l'accesso con un provider.
    pub fn with_reauth(mut self, extension: ext::ReauthExtension) -> Self {
        self.state.reauth = Some(extension);
        self
    }

    /// Chi può fermare la cancellazione di un utente (da lui stesso o dall'amministratore).
    pub fn with_deletion_guard(self, guard: ext::DeletionGuard) -> Self {
        self.state.accounts.set_deletion_guard(guard);
        self
    }

    /// Chi viene avvisato quando un utente è cancellato o un dispositivo revocato.
    pub fn with_observer(self, observer: Arc<dyn ext::AccountObserver>) -> Self {
        self.state.accounts.add_observer(observer);
        self
    }

    /// Rotte pubbliche in più, con il loro stato già applicato (senza fallback né strati: li
    /// mette il core). Una rotta che il core ha già è un [`StartError::DuplicateRoute`].
    pub fn with_public(mut self, routes: Router) -> Self {
        self.public.push(routes);
        self
    }

    /// Rotte amministrative in più, come [`Server::with_public`].
    pub fn with_admin(mut self, routes: Router) -> Self {
        self.admin.push(routes);
        self
    }

    /// Chi serve la pagina `/admin` (predefinito: il core).
    pub fn admin_page(mut self, page: ext::AdminPage) -> Self {
        self.admin_page = page;
        self
    }

    /// Il supervisore del figlio interno (`llm_provider = "builtin"`), costruito dal chiamante: i
    /// test gli danno un finto `llama-server`; in produzione lo costruisce il server dalla
    /// configurazione.
    pub fn with_llm_supervisor(mut self, supervisor: Arc<llm_builtin::LlmSupervisor>) -> Self {
        self.llm_supervisor = Some(supervisor);
        self.compute = OnceLock::new();
        self
    }

    /// I modelli `llm-*` installabili, costruiti dal chiamante (i test usano un finto catalogo).
    pub fn with_llm_models(mut self, models: Arc<dyn compute::llm_models::LlmModels>) -> Self {
        self.llm_models = Some(models);
        self.compute = OnceLock::new();
        self
    }

    /// Una riga di log (o altro) all'avvio di [`Server::serve`].
    pub fn with_on_start(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.on_start.push(Box::new(hook));
        self
    }

    /// Monta le rotte di calcolo dietro le chiavi dei dispositivi (D2, A1): `backend` e
    /// `speech` sono l'inferenza (quella vera in `serve`, un finto nei test). Le rotte
    /// d'installazione dei modelli vogliono la chiave amministratore.
    pub fn with_compute(
        mut self,
        backend: Arc<dyn compute::ComputeBackend>,
        speech: Arc<dyn compute::speech::SpeechBackend>,
        transcription_parallelism: usize,
    ) -> Self {
        self.compute_parts = Some(ComputeParts {
            backend,
            speech,
            transcription_parallelism,
        });
        self.compute = OnceLock::new();
        self
    }

    /// Le rotte di calcolo vere: EmbeddingGemma 2 caricato alla prima richiesta (come in
    /// locale), lettori nel figlio `--parser-worker`, whisper.cpp per la trascrizione.
    pub fn with_real_compute(mut self) -> Result<Self, StartError> {
        self.real_llm_models = true;
        let parser = compute::parser::ParserCommand::current_exe()
            .map_err(|e| StartError::Serve(format!("non trovo il lettore dei documenti: {e}")))?;
        let backend: Arc<dyn compute::ComputeBackend> =
            Arc::new(compute::engine::OnnxBackend::new(self.config.models_dir(), parser));
        // Audio of a transcription lives only in a private folder of the data folder (INV-2).
        let speech = Arc::new(
            compute::speech::WhisperBackend::new(self.config.speech_models_dir())
                .with_work_dir(self.config.data_dir.join("compute-work")),
        );
        let parallelism = self
            .config
            .transcription_parallelism
            .unwrap_or_else(compute::speech::default_parallelism);
        Ok(self.with_compute(backend, speech, parallelism))
    }

    /// Lo stato delle rotte di calcolo, se montate (per i test).
    pub fn compute_state(&self) -> Option<compute::ComputeState> {
        self.compute_state_ref().cloned()
    }

    /// Costruito una volta, alla prima richiesta: a quel punto i `with_*` sono tutti noti.
    fn compute_state_ref(&self) -> Option<&compute::ComputeState> {
        let parts = self.compute_parts.as_ref()?;
        Some(self.compute.get_or_init(|| {
            let gate = compute::DeviceGate::with_failures(
                Arc::clone(&self.state.accounts),
                Arc::clone(&self.state.trusted_proxies),
                self.config.compute_max_in_flight_per_user as usize,
                self.config.compute_max_auth_failures_per_origin,
                compute::DEVICE_AUTH_WINDOW,
            )
            .with_plaintext_from(Arc::clone(&self.state.plaintext_from))
            .with_entitlement(self.state.entitlement.clone());
            let mut sinks: Vec<Arc<dyn compute::UsageSink>> =
                vec![Arc::new(AccountsUsage(Arc::clone(&self.state.accounts)))];
            sinks.extend(self.usage_sinks.iter().cloned());
            let sink: Arc<dyn compute::UsageSink> = Arc::new(FanOutUsage(sinks));
            // Il modello di embedding che `/admin` può cambiare (`/admin/api/embedding`): quello
            // locale, o un servizio interno per i vettori dei testi.
            let remote_embedding = embedding::EmbeddingHolder::default();
            let embedding_admin = embedding::EmbeddingAdmin::new(
                remote_embedding.clone(),
                Arc::clone(&parts.backend),
                self.config.data_dir.join(embedding::SETTINGS_FILE),
                std::time::Duration::from_secs(self.config.embedding_switch_pause_secs),
            );
            let state = compute::ComputeState::new(
                Arc::new(embedding::SwitchableBackend::new(Arc::clone(&parts.backend), remote_embedding)),
                compute::ComputeAuth::Devices(Arc::new(gate)),
                "remote",
            )
            .with_speech(Arc::clone(&parts.speech), parts.transcription_parallelism)
            .with_usage(Arc::clone(&sink))
            .with_embedding_admin(Arc::new(embedding_admin));
            // One supervisor for the gateway and for the models: the removal of a model stops the child.
            let builtin = self.config.llm.provider == llm::LlmProvider::Builtin;
            let supervisor = if builtin {
                self.llm_supervisor.clone().or_else(|| {
                    llm_builtin::settings_from_config(&self.config).map(|s| llm_builtin::LlmSupervisor::new(s, llm_builtin::SupervisorDeps::default()))
                })
            } else {
                None
            };
            let state = match &self.llm_models {
                Some(models) => state.with_llm_models(Arc::clone(models)),
                None if self.real_llm_models => {
                    let models = compute::llm_models::BuiltinLlmModels::from_config(&self.config, supervisor.clone());
                    if builtin {
                        // The configured model is fetched in the background if it is not there (spec L8).
                        models.start_auto_install();
                    }
                    state.with_llm_models(Arc::new(models))
                }
                None => state,
            };
            // The assistant `/v1/info` declares and `/admin/api/llm` changes at run time: one holder
            // for the public state and the compute state.
            let holder = self.state.llm.clone();
            let accounts = Arc::clone(&self.state.accounts);
            let admin = llm::LlmAdmin::new(
                holder.clone(),
                self.llm_base.clone(),
                self.config.data_dir.join(llm::SETTINGS_FILE),
                Arc::clone(&accounts),
                Arc::clone(&sink),
                supervisor.clone().zip(self.config.llm.builtin_model.clone()).map(|(supervisor, model)| (model, supervisor)),
            );
            let route = (self.config.llm.max_request_bytes, self.config.llm.timeout_secs);
            let state = state.with_llm_admin(holder.clone(), route, Arc::new(admin));
            if !self.config.llm.enabled() {
                holder.replace(None);
                return state;
            }
            // `Server::open` ha già validato la configurazione: qui non può fallire.
            let gateway = if builtin {
                match supervisor {
                    Some(supervisor) => llm::LlmGateway::with_supervisor(&self.config.llm, accounts, sink, supervisor),
                    None => Err(ConfigError::Invalid { key: "llm_builtin_model", reason: "non è un modello del catalogo" }),
                }
            } else {
                llm::LlmGateway::new(&self.config.llm, accounts, sink)
            };
            match gateway {
                Ok(gateway) => {
                    holder.replace(Some(Arc::new(gateway)));
                }
                Err(error) => {
                    holder.replace(None);
                    tracing::error!(%error, "gateway LLM non montato");
                }
            }
            state
        }))
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn accounts(&self) -> &Arc<Accounts> {
        &self.state.accounts
    }

    /// La configurazione della sincronizzazione (per i comandi `sync`).
    pub fn sync_config(&self) -> &sync::SyncConfig {
        &self.config.sync
    }

    /// Il servizio di sincronizzazione, se c'è la chiave del server.
    pub fn sync_service(&self) -> Option<&Arc<sync::SyncService>> {
        self.sync.as_ref()
    }

    pub fn state(&self) -> AppState {
        self.state.clone()
    }

    /// Router dell'ascolto pubblico; con il calcolo montato, `/health` è quello del calcolo
    /// (senza chiave `{status, version}`, con una chiave valida anche modelli e stato).
    /// Una rotta delle estensioni che il core ha già è un [`StartError::DuplicateRoute`].
    pub fn try_public_router(&self) -> Result<Router, StartError> {
        let compute = self.compute_state_ref().cloned();
        catch_duplicate_route(|| {
            let mut core = http::compose_public(&self.state, compute.is_none(), &self.public);
            // Le rotte di sincronizzazione hanno il limite del corpo e gli strati loro (un elemento
            // da 64 KiB non sta nei 64 KiB del core); il fallback è quello del core.
            if let Some(sync) = &self.sync {
                core = core.merge(sync::routes::routes(self.state.clone(), Arc::clone(sync)));
            }
            match compute {
                Some(compute) => core.merge(compute::remote_router(compute)),
                None => core,
            }
        })
    }

    /// Router amministrativo, solo con la chiave amministratore configurata. Con il calcolo
    /// montato ha anche `/admin/api/models` e `/admin/api/models/{id}/install`: l'unico posto
    /// dove si usa la chiave amministratore per i modelli (D2).
    pub fn try_admin_router(&self) -> Result<Option<Router>, StartError> {
        if self.state.admin.is_none() {
            return Ok(None);
        }
        let compute = self.compute_state_ref().cloned();
        catch_duplicate_route(|| {
            http::compose_admin(
                &self.state,
                self.admin_page,
                compute.map(compute::admin_routes),
                &self.admin,
            )
        })
        .map(Some)
    }

    /// Come [`Server::try_public_router`], per i test: una rotta doppia è un panico.
    pub fn public_router(&self) -> Router {
        self.try_public_router().expect("router pubblico")
    }

    /// Come [`Server::try_admin_router`], per i test: una rotta doppia è un panico.
    pub fn admin_router(&self) -> Option<Router> {
        self.try_admin_router().expect("router amministrativo")
    }

    /// Avvia l'ascolto pubblico e, se configurato, quello amministrativo, fino a
    /// `shutdown`.
    pub async fn serve(
        self,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> Result<(), StartError> {
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        tokio::spawn(async move {
            shutdown.await;
            let _ = stop_tx.send(true);
        });

        match (self.config.admin_bind, self.config.admin_key.is_some()) {
            (Some(_), false) => tracing::warn!(
                "admin_bind è impostato ma QAMILLE_ADMIN_KEY è vuota: l'ascolto amministrativo resta chiuso"
            ),
            (None, true) => tracing::warn!(
                "QAMILLE_ADMIN_KEY è impostata ma admin_bind è vuoto: l'ascolto amministrativo resta chiuso"
            ),
            _ => {}
        }

        if let Some(notice) = self.config.origin_notice() {
            tracing::warn!("{notice}");
        }

        for hook in &self.on_start {
            hook();
        }

        // Prima dei router: un errore di rotte si ferma qui, prima di aprire le porte.
        let public_router = self.try_public_router()?;
        let admin_router = self.try_admin_router()?;
        let backend = self.compute_state_ref().map(|compute| Arc::clone(&compute.backend));
        if let Some(gateway) = self.compute_state_ref().and_then(|compute| compute.llm_gateway()) {
            gateway.log_startup();
        }
        if let Some(sync) = &self.sync {
            sync.log_startup(self.config.sync.old_key.is_some()).await;
            sync.spawn_purge();
        }
        if let Some(backend) = &backend {
            compute::spawn_idle_release(Arc::clone(backend));
            tracing::info!(models = %self.config.models_dir().display(), "rotte di calcolo montate");
        }

        let public = bind(self.config.bind).await?;
        tracing::info!(addr = %self.config.bind, "ascolto pubblico aperto");
        let public_task = tokio::spawn(run(public, public_router, stop_rx.clone()));

        let admin_task = match (self.config.admin_listener(), admin_router) {
            (Some((addr, _)), Some(router)) => {
                let listener = bind(addr).await?;
                tracing::info!(addr = %addr, "ascolto amministrativo aperto");
                Some(tokio::spawn(run(listener, router, stop_rx.clone())))
            }
            _ => None,
        };

        let joined = |r: Result<Result<(), StartError>, tokio::task::JoinError>| {
            r.map_err(|e| StartError::Serve(e.to_string()))?
        };
        let public_result = joined(public_task.await);
        // I figli dei lettori finiscono qui, anche se un'inferenza è ancora in corso.
        if let Some(backend) = backend {
            backend.shutdown();
        }
        let admin_result = match admin_task {
            Some(task) if public_result.is_ok() => joined(task.await),
            _ => Ok(()),
        };
        // Dopo gli ascolti: nessuna richiesta nuova può più chiedere il figlio interno.
        if let Some(gateway) = self.compute_state_ref().and_then(|compute| compute.llm_gateway()) {
            gateway.shutdown();
        }
        public_result?;
        admin_result?;
        tracing::info!("server fermato");
        Ok(())
    }
}

async fn bind(addr: SocketAddr) -> Result<tokio::net::TcpListener, StartError> {
    tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| StartError::Bind {
            addr,
            reason: e.to_string(),
        })
}

async fn run(
    listener: tokio::net::TcpListener,
    router: Router,
    mut stop: tokio::sync::watch::Receiver<bool>,
) -> Result<(), StartError> {
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = stop.wait_for(|stopped| *stopped).await;
    })
    .await
    .map_err(|e| StartError::Serve(e.to_string()))
}

/// `Router::merge` va in panico su una rotta doppia: qui diventa un errore di avvio leggibile.
fn catch_duplicate_route(build: impl FnOnce() -> Router) -> Result<Router, StartError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(build)).map_err(|panic| {
        let text = panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_else(|| "panico sconosciuto".to_owned());
        StartError::DuplicateRoute(text)
    })
}
