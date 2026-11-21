//! A scoped view of the running application. Client protocols translate at this boundary.
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use velune_application::ApplicationService;
use velune_application_api::{
    ClientViewId, Command, ComposedTurn, DeliveredSnapshot, ExecutionContext, Failure, FailureKind,
    Response,
    conversation::{
        ApprovalOptionKind, ConversationSummary, InteractionKind, InteractionOption,
        RuntimeInteractionReply,
    },
};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct SessionGrant {
    pub(super) runtime_id: String,
    pub(super) conversation_id: String,
}

type SendResult = Result<Option<String>, Failure>;

struct Submission {
    fingerprint: [u8; 32],
    result: Option<SendResult>,
}

/// The grant and its request results live for one activation of a control endpoint.
#[derive(Clone)]
pub(super) struct SessionControl {
    application: ApplicationService,
    view: ClientViewId,
    shared: Arc<BTreeMap<String, String>>,
    active: Arc<AtomicBool>,
    submissions: Arc<Mutex<HashMap<(String, String), Submission>>>,
}

impl SessionControl {
    pub(super) fn new(
        application: ApplicationService,
        grants: Vec<SessionGrant>,
        view: ClientViewId,
    ) -> Result<Self, Failure> {
        if grants.is_empty() || grants.len() > 16 {
            return Err(Failure::invalid("每次可授权 1 至 16 个会话"));
        }
        let mut shared = BTreeMap::new();
        for SessionGrant {
            runtime_id,
            conversation_id,
        } in grants
        {
            if [&runtime_id, &conversation_id]
                .iter()
                .any(|id| id.is_empty() || id.len() > 512 || id.chars().any(char::is_control))
                || shared.insert(conversation_id, runtime_id).is_some()
            {
                return Err(Failure::invalid("会话授权包含无效或重复的标识"));
            }
        }
        Ok(Self {
            application,
            view,
            shared: Arc::new(shared),
            active: Arc::new(AtomicBool::new(true)),
            submissions: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub(super) fn revoke(&self) {
        self.active.store(false, Ordering::Release);
    }

    pub(super) fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub(super) async fn list(&self) -> Result<Vec<ConversationSummary>, Failure> {
        self.run(|this| {
            let mut summaries = Vec::new();
            for (id, runtime) in this.shared.iter() {
                match this.execute(Command::FindConversation {
                    conversation_id: id.clone(),
                })? {
                    Response::FindConversation(Some(item))
                        if item.conversation.id == *id
                            && item.conversation.runtime_id == *runtime =>
                    {
                        summaries.push(item.conversation);
                    }
                    Response::FindConversation(_) => {}
                    _ => return Err(invalid_response()),
                }
            }
            summaries.sort_by_key(|summary| std::cmp::Reverse(summary.updated_at_unix_ms));
            Ok(summaries)
        })
        .await
    }

    pub(super) async fn detail(&self, id: &str) -> Result<DeliveredSnapshot, Failure> {
        let id = id.to_owned();
        self.run(move |this| this.snapshot(&id)).await
    }

    pub(super) async fn send(
        &self,
        id: &str,
        text: String,
        operation_id: Option<String>,
    ) -> SendResult {
        if text.trim().is_empty() || text.len() > 64 * 1024 {
            return Err(Failure::invalid("消息须包含正文且不超过 64 KiB"));
        }
        if operation_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 128 || id.chars().any(char::is_control))
        {
            return Err(Failure::invalid("消息操作标识无效"));
        }
        let id = id.to_owned();
        self.run(move |this| {
            this.check_active()?;
            let runtime_id = this.runtime(&id)?.to_owned();
            let key = operation_id.map(|operation| (id.clone(), operation));
            if let Some(key) = &key {
                let fingerprint = Sha256::digest(text.as_bytes()).into();
                let mut submissions = this.submissions.lock().map_err(|_| unavailable())?;
                if let Some(previous) = submissions.get(key) {
                    if previous.fingerprint != fingerprint {
                        return Err(Failure::new(
                            FailureKind::Conflict,
                            "此操作标识已用于其他消息",
                        ));
                    }
                    return previous.result.clone().unwrap_or_else(|| {
                        Err(Failure::new(
                            FailureKind::Conflict,
                            "正在接纳此消息，请刷新会话",
                        ))
                    });
                }
                if submissions.len() >= 4096 {
                    return Err(Failure::new(
                        FailureKind::Unavailable,
                        "已达到此次会话共享的消息上限",
                    ));
                }
                submissions.insert(
                    key.clone(),
                    Submission {
                        fingerprint,
                        result: None,
                    },
                );
            }
            let result = match this.execute(Command::SendCurrentTurn {
                source_runtime_id: runtime_id,
                conversation_id: id,
                turn: ComposedTurn {
                    text,
                    effort: None,
                    permission_mode: None,
                    attachments: Vec::new(),
                },
            }) {
                Ok(Response::SendTurn(result)) => Ok(result.submitted_message_id),
                Ok(_) => Err(invalid_response()),
                Err(error) => Err(error),
            };
            if let Some(key) = key
                && let Some(submission) = this
                    .submissions
                    .lock()
                    .map_err(|_| unavailable())?
                    .get_mut(&key)
            {
                submission.result = Some(result.clone());
            }
            result
        })
        .await
    }

    pub(super) async fn stop(&self, id: &str) -> Result<(), Failure> {
        let id = id.to_owned();
        self.run(move |this| {
            let snapshot = this.snapshot(&id)?;
            if !snapshot.actions.can_cancel {
                return Err(Failure::new(FailureKind::Conflict, "会话没有可停止的轮次"));
            }
            match this.execute(Command::Cancel {
                runtime_id: snapshot.context_runtime_id,
                conversation_id: id,
            })? {
                Response::Cancel(_) => Ok(()),
                _ => Err(invalid_response()),
            }
        })
        .await
    }

    pub(super) async fn decide(
        &self,
        id: &str,
        request_id: &str,
        allow: bool,
    ) -> Result<(), Failure> {
        self.reply_to_approval(id, request_id, move |options| {
            let kind = if allow {
                ApprovalOptionKind::AllowOnce
            } else {
                ApprovalOptionKind::RejectOnce
            };
            one_shot_options(options)
                .find(|option| option.approval_kind == Some(kind))
                .map(|option| option.id.clone())
        })
        .await
    }

    pub(super) async fn reply(
        &self,
        id: &str,
        request_id: &str,
        option_id: String,
    ) -> Result<(), Failure> {
        self.reply_to_approval(id, request_id, move |options| {
            one_shot_options(options)
                .any(|option| option.id == option_id)
                .then_some(option_id)
        })
        .await
    }

    async fn reply_to_approval(
        &self,
        id: &str,
        request_id: &str,
        option: impl FnOnce(&[InteractionOption]) -> Option<String> + Send + 'static,
    ) -> Result<(), Failure> {
        let id = id.to_owned();
        let request_id = request_id.to_owned();
        self.run(move |this| {
            let snapshot = this.snapshot(&id)?;
            let interaction = snapshot
                .pending_interactions
                .iter()
                .find(|item| item.id == request_id)
                .ok_or_else(|| {
                    Failure::new(FailureKind::StaleInteraction, "此审批已被处理或已失效")
                })?;
            let InteractionKind::Approval { options, .. } = &interaction.kind else {
                return Err(Failure::new(
                    FailureKind::Unsupported,
                    "此请求需要其他类型的答复",
                ));
            };
            let option_id = option(options).ok_or_else(|| {
                Failure::new(FailureKind::Unsupported, "此审批不支持该一次性答复")
            })?;
            match this.execute(Command::ReplyRuntimeInteraction {
                runtime_id: snapshot.context_runtime_id,
                conversation_id: id,
                interaction_id: request_id,
                reply: RuntimeInteractionReply::Decision { option_id },
            })? {
                Response::ReplyRuntimeInteraction(_) => Ok(()),
                _ => Err(invalid_response()),
            }
        })
        .await
    }

    fn runtime(&self, id: &str) -> Result<&str, Failure> {
        self.shared
            .get(id)
            .map(String::as_str)
            .ok_or_else(conversation_not_shared)
    }

    fn snapshot(&self, id: &str) -> Result<DeliveredSnapshot, Failure> {
        let runtime_id = self.runtime(id)?.to_owned();
        match self.execute(Command::ReadConversation {
            runtime_id,
            conversation_id: id.to_owned(),
        })? {
            Response::ReadConversation(result) => result
                .snapshot
                .filter(|snapshot| snapshot.conversation.id == id)
                .ok_or_else(conversation_not_shared),
            _ => Err(invalid_response()),
        }
    }

    fn check_active(&self) -> Result<(), Failure> {
        if !self.is_active() {
            return Err(Failure::new(FailureKind::Closed, "会话共享已关闭"));
        }
        Ok(())
    }

    fn execute(&self, command: Command) -> Result<Response, Failure> {
        self.check_active()?;
        self.application
            .execute_checked(self.view.clone(), ExecutionContext::default(), command)
            .map(|reply| reply.response)
    }

    async fn run<T: Send + 'static>(
        &self,
        task: impl FnOnce(Self) -> Result<T, Failure> + Send + 'static,
    ) -> Result<T, Failure> {
        let this = self.clone();
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        tokio::task::spawn_blocking(move || {
            tracing::dispatcher::with_default(&dispatch, || task(this))
        })
        .await
        .map_err(|_| unavailable())?
    }
}

pub(super) fn one_shot_options(
    options: &[InteractionOption],
) -> impl Iterator<Item = &InteractionOption> {
    options.iter().filter(|option| {
        matches!(
            option.approval_kind,
            Some(ApprovalOptionKind::AllowOnce | ApprovalOptionKind::RejectOnce)
        ) && options.iter().filter(|other| other.id == option.id).count() == 1
    })
}

fn conversation_not_shared() -> Failure {
    let mut failure = Failure::invalid("会话不存在或未共享");
    failure.code = "conversation_not_shared".into();
    failure
}

fn unavailable() -> Failure {
    Failure::new(FailureKind::Unavailable, "执行端会话服务不可用")
}

fn invalid_response() -> Failure {
    Failure::new(FailureKind::Contract, "执行端返回了不匹配的会话响应")
}
