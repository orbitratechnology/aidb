//! A question from the UI to a finished task: which provider, what may be sent, the context, and
//! one task at a time.

use super::{CancelSignal, RunningTasks, SessionTools, TaskInput, run};
use crate::adapters::credentials::Credentials;
use crate::adapters::database::DatabaseAdapter;
use crate::adapters::local_store::LocalStore;
use crate::domain::agent::ask::{AskRequest, AssistantProvider, CheckedAsk, ContextPreview};
use crate::domain::agent::context::{self, BuiltContext, ConnectionContext, ContextRequest};
use crate::domain::agent::plan::{PlanTarget, TaskId, TaskPlan};
use crate::domain::agent::task::{TaskEvent, TaskOutcome};
use crate::domain::connections::ConnectionId;
use crate::domain::providers::{ProviderProfile, is_loopback_url};
use crate::errors::{AppError, AppResult};
use crate::services::approvals::Approvals;
use crate::services::catalog::{self, CatalogCache};
use crate::services::sessions::{Sessions, not_connected};
use crate::services::{providers, settings};

/// The provider chosen in Settings › AI, if the assistant is on and it can call tools.
async fn chosen_provider(store: &LocalStore) -> AppResult<ProviderProfile> {
    let ai = settings::load(store).await?.ai;
    let id = match ai.provider_id {
        Some(id) if ai.enabled => id,
        _ => {
            return Err(AppError::unavailable(
                "The assistant is off. Turn it on and choose a provider in Settings › AI.",
            ));
        }
    };
    let profile = providers::get(store, id).await?;
    if !profile.features().tool_calls {
        return Err(AppError::invalid_input(
            "The assistant needs a model that can call tools, and the last provider test found \
             that this one cannot. Choose another model in Settings › AI.",
        ));
    }
    Ok(profile)
}

/// Whether the provider runs on another machine.
fn hosted(profile: &ProviderProfile) -> AppResult<bool> {
    let url = profile.settings.endpoint_url().map_err(|error| {
        AppError::invalid_input(format!("The provider's endpoint is not valid: {error}."))
    })?;
    Ok(!is_loopback_url(&url))
}

struct Prepared {
    profile: ProviderProfile,
    ask: CheckedAsk,
    built: BuiltContext,
    target: PlanTarget,
    data_allowed: bool,
    hosted: bool,
}

async fn prepare<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    cache: &CatalogCache,
    id: ConnectionId,
    request: AskRequest,
) -> AppResult<Prepared> {
    let ask = request
        .checked()
        .map_err(|error| AppError::invalid_input(error.to_string()))?;
    let profile = chosen_provider(store).await?;
    let hosted = hosted(&profile)?;
    let data_allowed = !hosted || ask.share_data;
    if !data_allowed && !ask.attachments.is_empty() {
        return Err(AppError::invalid_input(format!(
            "Attached files go to {} only when you allow sharing data for this conversation.",
            profile.settings.name
        )));
    }

    let active = sessions
        .list()
        .into_iter()
        .find(|session| session.connection_id == id)
        .ok_or_else(not_connected)?;
    let connection = store
        .connections()
        .get(id)
        .await?
        .ok_or_else(|| AppError::not_found("The connection no longer exists."))?;
    let snapshot = catalog::overview(store, sessions, cache, id, false).await?;
    let mut selected = Vec::with_capacity(ask.selected.len());
    for object in &ask.selected {
        let details = catalog::describe(store, sessions, cache, id, object.clone()).await?;
        selected.push((*details).clone());
    }
    let connection_context = ConnectionContext {
        database: active.server.database.clone(),
        environment: connection.settings.environment,
        server_version: active.server.version.clone(),
        read_only: active.server.read_only || active.server.in_recovery,
    };
    let built = context::build(&ContextRequest {
        question: &ask.question,
        connection: &connection_context,
        catalog: Some(&snapshot.overview),
        selected: &selected,
        attachments: &ask.attachments,
        budget_tokens: profile
            .settings
            .context_budget_tokens
            .unwrap_or(context::DEFAULT_CONTEXT_TOKENS),
    })
    .map_err(|error| AppError::invalid_input(error.to_string()))?;
    let target = PlanTarget {
        connection_id: id,
        database: active.server.database,
        environment: connection.settings.environment,
        read_only: active.server.read_only || active.server.in_recovery,
    };
    Ok(Prepared {
        profile,
        ask,
        built,
        target,
        data_allowed,
        hosted,
    })
}

fn provider_view(prepared: &Prepared) -> AssistantProvider {
    AssistantProvider {
        name: prepared.profile.settings.name.clone(),
        model: prepared.profile.settings.model.clone(),
        hosted: prepared.hosted,
    }
}

/// What a question would send, so the user can review it first. Nothing is sent.
///
/// # Errors
///
/// `unavailable` when the assistant is off; `invalid_input` for a request over its limits, or
/// files without permission to share them; the session's or catalog's errors.
pub async fn preview<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    cache: &CatalogCache,
    id: ConnectionId,
    request: AskRequest,
) -> AppResult<ContextPreview> {
    let prepared = prepare(store, sessions, cache, id, request).await?;
    Ok(ContextPreview {
        provider: provider_view(&prepared),
        estimated_tokens: prepared.built.estimated_tokens,
        budget_tokens: prepared
            .profile
            .settings
            .context_budget_tokens
            .unwrap_or(context::DEFAULT_CONTEXT_TOKENS),
        items: prepared.built.items,
        data_allowed: prepared.data_allowed,
    })
}

/// Removes the task from the running ones however the command ends.
struct Registration<'a> {
    running: &'a RunningTasks,
    id: TaskId,
}

impl Drop for Registration<'_> {
    fn drop(&mut self) {
        self.running.finish(self.id);
    }
}

/// The open connection's services, as the assistant uses them.
#[derive(Debug, Clone, Copy)]
pub struct Workspace<'a, A: DatabaseAdapter> {
    pub store: &'a LocalStore,
    pub credentials: &'a Credentials,
    pub sessions: &'a Sessions<A>,
    pub cache: &'a CatalogCache,
    pub running: &'a RunningTasks,
    /// Where changes the assistant prepares wait for the user.
    pub approvals: &'a Approvals,
}

/// Answers a question on the open connection, reporting progress to `on_event`. The task's id
/// comes first, in [`TaskEvent::Started`], so it can be cancelled.
///
/// # Errors
///
/// As [`preview`]; `conflict` while another question is being answered; `credential_store` if
/// the provider's key cannot be read. Once the task starts, problems end it with a status
/// instead.
pub async fn ask<A: DatabaseAdapter>(
    workspace: Workspace<'_, A>,
    id: ConnectionId,
    request: AskRequest,
    on_event: &mut (dyn FnMut(TaskEvent) + Send),
) -> AppResult<TaskOutcome> {
    let Workspace {
        store,
        credentials,
        sessions,
        cache,
        running,
        approvals,
    } = workspace;
    let prepared = prepare(store, sessions, cache, id, request).await?;
    let (_, client) = providers::client(store, credentials, prepared.profile.id).await?;
    let plan = TaskPlan::new(prepared.target, &prepared.ask.question)
        .map_err(|error| AppError::invalid_input(error.to_string()))?;
    let task_id = plan.task_id;
    let cancel: CancelSignal = running.start_alone(task_id).ok_or_else(|| {
        AppError::conflict("The assistant is already answering a question. Stop it or wait for it.")
    })?;
    let _registration = Registration {
        running,
        id: task_id,
    };
    let tools = SessionTools {
        store,
        sessions,
        cache,
        approvals,
        connection_id: id,
    };
    tracing::info!(
        task = %task_id,
        hosted = prepared.hosted,
        data_allowed = prepared.data_allowed,
        attachments = prepared.ask.attachments.len(),
        history = prepared.ask.history.len(),
        "assistant task started"
    );
    let outcome = run(
        store,
        &client,
        &tools,
        TaskInput {
            context: prepared.built,
            plan,
            history: prepared.ask.history,
            model: prepared.profile.settings.model,
            data_allowed: prepared.data_allowed,
        },
        &cancel,
        on_event,
    )
    .await;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::adapters::local_store::test_support::temp_store;
    use crate::domain::agent::ask::AttachmentInput;
    use crate::domain::connections::ConnectionDraft;
    use crate::domain::providers::{
        CheckedFeatures, ProviderDraft, ProviderFeatures, ProviderPreset,
    };
    use crate::domain::settings::{AiSettings, SettingsPatch};
    use crate::errors::ErrorCode;
    use crate::services::fakes::{FakeAdapter, sessions, target};

    struct Setup {
        _dir: tempfile::TempDir,
        store: LocalStore,
        credentials: Credentials,
        sessions: Sessions<Arc<FakeAdapter>>,
        cache: CatalogCache,
        running: RunningTasks,
        approvals: Approvals,
        connection: ConnectionId,
    }

    impl Setup {
        fn workspace(&self) -> Workspace<'_, Arc<FakeAdapter>> {
            Workspace {
                store: &self.store,
                credentials: &self.credentials,
                sessions: &self.sessions,
                cache: &self.cache,
                running: &self.running,
                approvals: &self.approvals,
            }
        }

        async fn preview(&self, request: AskRequest) -> AppResult<ContextPreview> {
            preview(
                &self.store,
                &self.sessions,
                &self.cache,
                self.connection,
                request,
            )
            .await
        }
    }

    /// A connected session and, when `kind` is given, that provider chosen with AI on.
    async fn setup(kind: Option<ProviderPreset>) -> Setup {
        let (dir, store) = temp_store().await;
        let profile = store
            .connections()
            .insert(
                &ConnectionDraft::new("Shop", "db", "shop", "reader")
                    .validated()
                    .expect("valid"),
            )
            .await
            .expect("insert");
        let (adapter, sessions) = sessions();
        adapter.catalog.set_fingerprint("v1");
        sessions.open(profile.id, target("db")).await.expect("open");
        let credentials = Credentials::session_only("tests");
        if let Some(kind) = kind {
            let provider = providers::create(
                &store,
                &credentials,
                ProviderDraft::new("Model", kind, "model-1"),
            )
            .await
            .expect("provider");
            settings::update(
                &store,
                SettingsPatch {
                    ai: Some(AiSettings {
                        enabled: true,
                        provider_id: Some(provider.id),
                    }),
                    ..SettingsPatch::default()
                },
            )
            .await
            .expect("settings");
        }
        Setup {
            _dir: dir,
            store,
            credentials,
            sessions,
            cache: CatalogCache::default(),
            running: RunningTasks::default(),
            approvals: Approvals::default(),
            connection: profile.id,
        }
    }

    fn question(text: &str) -> AskRequest {
        AskRequest {
            question: text.into(),
            selected: Vec::new(),
            attachments: Vec::new(),
            history: Vec::new(),
            share_data: false,
        }
    }

    fn with_file(mut request: AskRequest) -> AskRequest {
        request.attachments.push(AttachmentInput {
            name: "orders.csv".into(),
            content: "id,total\n1,10".into(),
        });
        request
    }

    #[tokio::test]
    async fn nothing_is_prepared_while_the_assistant_is_off() {
        let setup = setup(None).await;
        let error = setup
            .preview(question("How many orders?"))
            .await
            .expect_err("off");
        assert_eq!(error.code(), ErrorCode::Unavailable);
    }

    #[tokio::test]
    async fn local_providers_may_receive_data() {
        let setup = setup(Some(ProviderPreset::Ollama)).await;
        let preview = setup
            .preview(with_file(question("How many orders?")))
            .await
            .expect("preview");
        assert!(!preview.provider.hosted);
        assert!(preview.data_allowed);
        assert!(preview.items.iter().any(|item| item.contains_data_values));
        assert_eq!(preview.budget_tokens, context::DEFAULT_CONTEXT_TOKENS);
        assert!(preview.estimated_tokens > 0);
    }

    #[tokio::test]
    async fn hosted_providers_get_data_only_with_permission() {
        let setup = setup(Some(ProviderPreset::OpenAi)).await;
        let preview = setup
            .preview(question("How many orders?"))
            .await
            .expect("preview");
        assert!(preview.provider.hosted);
        assert!(!preview.data_allowed);

        let error = setup
            .preview(with_file(question("Summarize this file")))
            .await
            .expect_err("files need permission");
        assert_eq!(error.code(), ErrorCode::InvalidInput);

        let mut allowed = with_file(question("Summarize this file"));
        allowed.share_data = true;
        assert!(setup.preview(allowed).await.expect("preview").data_allowed);
    }

    #[tokio::test]
    async fn models_that_cannot_call_tools_are_refused() {
        let setup = setup(Some(ProviderPreset::Ollama)).await;
        let id = settings::load(&setup.store)
            .await
            .expect("settings")
            .ai
            .provider_id
            .expect("chosen");
        setup
            .store
            .providers()
            .set_checked(
                id,
                &CheckedFeatures {
                    features: ProviderFeatures {
                        tool_calls: false,
                        ..ProviderPreset::Ollama.assumed_capabilities()
                    },
                    checked_at: "2026-01-01T00:00:00Z".into(),
                },
            )
            .await
            .expect("checked");
        let error = setup.preview(question("q")).await.expect_err("no tools");
        assert_eq!(error.code(), ErrorCode::InvalidInput);
    }

    #[tokio::test]
    async fn questions_need_the_open_connection() {
        let mut setup = setup(Some(ProviderPreset::Ollama)).await;
        setup.connection = ConnectionId::new();
        assert!(setup.preview(question("q")).await.is_err());
    }

    #[tokio::test]
    async fn one_question_at_a_time() {
        let setup = setup(Some(ProviderPreset::Ollama)).await;
        assert!(setup.running.start_alone(TaskId::new()).is_some());
        let mut events = Vec::new();
        let error = ask(
            setup.workspace(),
            setup.connection,
            question("How many orders?"),
            &mut |event| events.push(event),
        )
        .await
        .expect_err("busy");
        assert_eq!(error.code(), ErrorCode::Conflict);
        assert!(events.is_empty());
        // The refused task left nothing registered or saved.
        assert!(
            setup
                .store
                .agent_tasks()
                .list(10)
                .await
                .expect("list")
                .is_empty()
        );
    }
}
