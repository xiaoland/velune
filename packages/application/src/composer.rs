//! The application owner persists shared input independently of client views.
use crate::{Error, repository::commit};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
use velune_application_api::*;

mod migration;
pub(crate) use migration::prepare_migration;

const CURRENT_SCHEMA: u32 = 2;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Stored {
    schema_version: u32,
    state: ComposerInputState,
    attachments: Vec<ComposerAttachment>,
}

fn decode_stored(value: serde_json::Value) -> Result<Stored, Error> {
    if value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(2)
    {
        return Err(Error::invalid(
            "composer-inputs.json schema 不受支持；原文件保留",
        ));
    }
    let stored: Stored = serde_json::from_value(value.clone())?;
    validate_state(&stored)?;
    if serde_json::to_value(&stored)? != value {
        return Err(Error::invalid(
            "composer input 含当前 schema 未知字段；原文件保留",
        ));
    }
    Ok(stored)
}

pub(crate) struct ComposerInputs {
    path: PathBuf,
    attachments: PathBuf,
    stored: Stored,
}

fn identity() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("operating system randomness");
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

fn apply_promotion(
    next: &mut Stored,
    revision: u64,
    promotion: ComposerPromotion,
) -> Result<(), Error> {
    let ComposerPromotion {
        input_id,
        owner_id,
        conversation_id,
        runtime_id,
        choices,
    } = promotion;
    let context = ComposerContextKey::Conversation {
        owner_id: owner_id.clone(),
        conversation_id: conversation_id.clone(),
    };
    validate_context(&context)?;
    if let Some(choices) = &choices {
        if LocalConversation::of(&conversation_id).is_some() {
            return Err(Error::invalid(
                "composer promotion requires a native identity",
            ));
        }
        validate_choices(choices)?;
    }
    if runtime_id.is_empty() {
        return Err(Error::invalid("composer creation runtime"));
    }
    if let Some(creation) = next.state.creations.iter().find(|creation| {
        creation.owner_id == owner_id && creation.conversation_id == conversation_id
    }) {
        if creation.input_id != input_id || creation.runtime_id != runtime_id {
            return Err(Error::invalid("composer creation identity changed"));
        }
        return Ok(());
    }
    next.state.creations.push(ComposerCreation {
        input_id: input_id.clone(),
        owner_id: owner_id.clone(),
        runtime_id: runtime_id.clone(),
        conversation_id,
        revision,
    });
    if !next.state.inputs.iter().any(|input| input.id == input_id) {
        return Err(Error::invalid("composer input does not exist"));
    }
    let seed = choices.map(normalize_composer_target_choices);
    if let Some(target) = next
        .state
        .targets
        .iter_mut()
        .find(|target| target.context == context)
    {
        if let Some(seed) = seed
            && target.revision == 1
            && target.choices == ComposerTargetChoices::default()
            && seed != ComposerTargetChoices::default()
        {
            target.choices = seed;
            target.revision += 1;
        }
    } else {
        let fallback = || ComposerTargetChoices {
            runtime_id: Some(runtime_id),
            runtime_owner_id: Some(owner_id),
            ..Default::default()
        };
        next.state.targets.push(ComposerTarget {
            context: context.clone(),
            revision: 1,
            choices: seed
                .filter(|seed| *seed != ComposerTargetChoices::default())
                .unwrap_or_else(fallback),
        });
    }
    if let Some(binding) = next
        .state
        .bindings
        .iter_mut()
        .find(|binding| binding.context == context)
    {
        if binding.input_id != input_id {
            binding.input_id = input_id.clone();
            binding.revision = revision;
        }
    } else {
        next.state.bindings.push(ComposerInputBinding {
            context,
            input_id: input_id.clone(),
            revision,
        });
    }
    let new = next
        .state
        .bindings
        .iter_mut()
        .find(|binding| binding.context == ComposerContextKey::NewConversation)
        .unwrap();
    if new.input_id == input_id {
        let fresh = ComposerInput {
            id: identity(),
            revision: 1,
            content: Default::default(),
        };
        new.input_id = fresh.id.clone();
        new.revision = revision;
        next.state.inputs.push(fresh);
    }
    Ok(())
}

impl ComposerInputs {
    pub(crate) fn target(&self, context: &ComposerContextKey) -> Option<ComposerTarget> {
        self.stored
            .state
            .targets
            .iter()
            .find(|target| &target.context == context)
            .cloned()
    }

    // Older platform views use field-specific commands. They share the same
    // records; editing text must preserve attachments and execution choices.
    pub(crate) fn legacy_drafts(&self) -> Vec<ComposerDraft> {
        self.stored
            .state
            .bindings
            .iter()
            .filter_map(|binding| {
                let conversation_id = match &binding.context {
                    ComposerContextKey::Conversation { owner_id, .. }
                        if owner_id != &self.stored.state.owner_id =>
                    {
                        return None;
                    }
                    context => context.conversation_id().to_owned(),
                };
                self.stored
                    .state
                    .inputs
                    .iter()
                    .find(|input| input.id == binding.input_id)
                    .map(|input| ComposerDraft {
                        conversation_id,
                        text: input.content.text.clone(),
                    })
            })
            .collect()
    }
    pub(crate) fn legacy_intents(&self) -> Vec<NextTurnIntent> {
        self.stored
            .state
            .targets
            .iter()
            .filter_map(|target| {
                let conversation_id = match &target.context {
                    ComposerContextKey::Conversation { owner_id, .. }
                        if owner_id != &self.owner_id() =>
                    {
                        return None;
                    }
                    context => context.conversation_id().to_owned(),
                };
                Some(NextTurnIntent {
                    conversation_id,
                    runtime_id: target.choices.runtime_id.clone(),
                    choices: target
                        .choices
                        .choices
                        .iter()
                        .filter(|choice| choice.owner_id == self.owner_id())
                        .map(|choice| RuntimeTurnChoice {
                            runtime_id: choice.runtime_id.clone(),
                            choice: choice.choice.clone(),
                        })
                        .collect(),
                })
            })
            .collect()
    }
    fn legacy_context(&self, id: &str) -> ComposerContextKey {
        if id.is_empty() || id == LocalConversation::LEGACY_NEW_CONVERSATION_ID {
            ComposerContextKey::NewConversation
        } else {
            ComposerContextKey::Conversation {
                owner_id: self.owner_id(),
                conversation_id: id.into(),
            }
        }
    }
    fn legacy_input(&self, id: &str) -> Option<&ComposerInput> {
        let context = self.legacy_context(id);
        let binding = self
            .stored
            .state
            .bindings
            .iter()
            .find(|binding| binding.context == context)?;
        self.stored
            .state
            .inputs
            .iter()
            .find(|input| input.id == binding.input_id)
    }
    pub(crate) fn legacy_edit(
        &mut self,
        id: String,
        text: Option<String>,
        intent: Option<NextTurnIntent>,
    ) -> Result<(), Error> {
        let context = self.legacy_context(&id);
        self.operate(ComposerInputOperation::Ensure {
            context: context.clone(),
            input_id: identity(),
            seed: Default::default(),
        })?;
        let input = self.legacy_input(&id).unwrap();
        let (id, revision, mut content) = (input.id.clone(), input.revision, input.content.clone());
        if let Some(text) = text {
            content.text = text;
        }
        if let Some(intent) = intent {
            let owner_id = self.owner_id();
            let target = self
                .stored
                .state
                .targets
                .iter()
                .find(|target| target.context == context);
            let mut choices = target
                .map(|target| target.choices.clone())
                .unwrap_or_default();
            let expected_revision = target.map(|target| target.revision);
            choices.runtime_owner_id = intent.runtime_id.as_ref().map(|_| owner_id.clone());
            choices.runtime_id = intent.runtime_id;
            choices.choices.retain(|choice| choice.owner_id != owner_id);
            choices
                .choices
                .extend(
                    intent
                        .choices
                        .into_iter()
                        .map(|choice| ComposerRuntimeChoice {
                            owner_id: owner_id.clone(),
                            runtime_id: choice.runtime_id,
                            choice: choice.choice,
                        }),
                );
            choices.choice_edited = true;
            self.operate(ComposerInputOperation::SaveTarget {
                context,
                expected_revision,
                choices,
            })?;
        }
        self.operate(ComposerInputOperation::Save {
            input_id: id,
            expected_revision: Some(revision),
            content,
        })?;
        Ok(())
    }
    pub(crate) fn update(
        &mut self,
        operation: ComposerInputOperation,
    ) -> Result<ComposerInputUpdate, Error> {
        if matches!(operation, ComposerInputOperation::Read { since_revision: Some(revision) } if revision == self.stored.state.revision)
        {
            return Ok(ComposerInputUpdate {
                revision: self.stored.state.revision,
                state: None,
                atomic_creation: true,
            });
        }
        let state = self.operate(operation)?;
        Ok(ComposerInputUpdate {
            revision: state.revision,
            state: Some(state),
            atomic_creation: true,
        })
    }
    pub(crate) fn open(home: &Path) -> Result<Self, Error> {
        let path = home.join("composer-inputs.json");
        let stored = match fs::read(&path) {
            Ok(bytes) => decode_stored(serde_json::from_slice(&bytes)?)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                migration::guard_missing_catalog(home)?;
                let input = ComposerInput {
                    id: identity(),
                    revision: 1,
                    content: Default::default(),
                };
                let stored = Stored {
                    schema_version: CURRENT_SCHEMA,
                    state: ComposerInputState {
                        owner_id: identity(),
                        revision: 1,
                        creations: Vec::new(),
                        targets: vec![ComposerTarget {
                            context: ComposerContextKey::NewConversation,
                            revision: 1,
                            choices: Default::default(),
                        }],
                        bindings: vec![ComposerInputBinding {
                            context: ComposerContextKey::NewConversation,
                            input_id: input.id.clone(),
                            revision: 1,
                        }],
                        inputs: vec![input],
                    },
                    attachments: Vec::new(),
                };
                commit(&path, &serde_json::to_vec_pretty(&stored)?)?;
                stored
            }
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            path,
            attachments: home.join("composer-attachments"),
            stored,
        })
    }

    pub(crate) fn operate(
        &mut self,
        operation: ComposerInputOperation,
    ) -> Result<ComposerInputState, Error> {
        use ComposerInputOperation as Op;
        if matches!(operation, Op::Read { .. }) {
            return Ok(self.stored.state.clone());
        }
        let mut next = self.stored.clone();
        let revision = next.state.revision + 1;
        match operation {
            Op::Read { .. } => unreachable!(),
            Op::Ensure {
                context,
                input_id,
                seed,
            } => {
                validate_context(&context)?;
                validate_id(&input_id)?;
                if !next
                    .state
                    .bindings
                    .iter()
                    .any(|binding| binding.context == context)
                {
                    self.validate_content(&seed)?;
                    if !next.state.inputs.iter().any(|input| input.id == input_id) {
                        next.state.inputs.push(ComposerInput {
                            id: input_id.clone(),
                            revision: 1,
                            content: seed,
                        });
                    }
                    next.state.bindings.push(ComposerInputBinding {
                        context,
                        input_id,
                        revision,
                    });
                } else if context == ComposerContextKey::NewConversation {
                    // The first caller may seed a brand-new empty global slot.
                    let id = &next
                        .state
                        .bindings
                        .iter()
                        .find(|binding| binding.context == context)
                        .unwrap()
                        .input_id;
                    let input = next
                        .state
                        .inputs
                        .iter_mut()
                        .find(|input| &input.id == id)
                        .unwrap();
                    if input.revision == 1
                        && input.content == ComposerInputContent::default()
                        && seed != ComposerInputContent::default()
                    {
                        self.validate_content(&seed)?;
                        input.content = seed;
                        input.revision += 1;
                    }
                }
            }
            Op::Save {
                input_id,
                expected_revision,
                content,
            } => {
                validate_id(&input_id)?;
                self.validate_content(&content)?;
                match next
                    .state
                    .inputs
                    .iter_mut()
                    .find(|input| input.id == input_id)
                {
                    Some(input) if expected_revision == Some(input.revision) => {
                        if input.content != content {
                            input.content = content;
                            input.revision += 1;
                        }
                    }
                    None if expected_revision.is_none() => next.state.inputs.push(ComposerInput {
                        id: input_id,
                        revision: 1,
                        content,
                    }),
                    _ => {
                        return Err(Failure::new(
                            FailureKind::Conflict,
                            "输入已在另一端更新，请接续最新内容",
                        )
                        .into());
                    }
                }
            }
            Op::EnsureTarget { context, seed } => {
                validate_context(&context)?;
                validate_choices(&seed)?;
                let seed = normalize_composer_target_choices(seed);
                if let Some(target) = next
                    .state
                    .targets
                    .iter_mut()
                    .find(|target| target.context == context)
                {
                    if target.revision == 1
                        && target.choices == ComposerTargetChoices::default()
                        && seed != ComposerTargetChoices::default()
                    {
                        target.choices = seed;
                        target.revision += 1;
                    }
                } else {
                    next.state.targets.push(ComposerTarget {
                        context,
                        revision: 1,
                        choices: seed,
                    });
                }
            }
            Op::SaveTarget {
                context,
                expected_revision,
                choices,
            } => {
                validate_context(&context)?;
                validate_choices(&choices)?;
                let choices = normalize_composer_target_choices(choices);
                match next
                    .state
                    .targets
                    .iter_mut()
                    .find(|target| target.context == context)
                {
                    Some(target) if expected_revision == Some(target.revision) => {
                        if target.choices != choices {
                            target.choices = choices;
                            target.revision += 1;
                        }
                    }
                    None if expected_revision.is_none() => {
                        next.state.targets.push(ComposerTarget {
                            context,
                            revision: 1,
                            choices,
                        })
                    }
                    _ => {
                        return Err(Failure::new(
                            FailureKind::Conflict,
                            "目标执行配置已在另一端更新，当前选择已保留",
                        )
                        .into());
                    }
                }
            }
            Op::Bind {
                context,
                input_id,
                expected_revision,
            } => {
                validate_context(&context)?;
                if !next.state.inputs.iter().any(|input| input.id == input_id) {
                    return Err(Error::invalid("composer input does not exist"));
                }
                match next
                    .state
                    .bindings
                    .iter_mut()
                    .find(|binding| binding.context == context)
                {
                    Some(binding) if expected_revision == Some(binding.revision) => {
                        if binding.input_id != input_id {
                            binding.input_id = input_id;
                            binding.revision = revision;
                        }
                    }
                    None if expected_revision.is_none() => {
                        next.state.bindings.push(ComposerInputBinding {
                            context,
                            input_id,
                            revision,
                        })
                    }
                    _ => {
                        return Err(Failure::new(
                            FailureKind::Conflict,
                            "输入关联已在另一端更新，当前输入已保留",
                        )
                        .into());
                    }
                }
            }
            Op::Promote {
                input_id,
                owner_id,
                conversation_id,
                runtime_id,
            } => {
                apply_promotion(
                    &mut next,
                    revision,
                    ComposerPromotion {
                        input_id,
                        owner_id,
                        conversation_id,
                        runtime_id,
                        choices: None,
                    },
                )?;
            }
            Op::PromoteWithTarget { promotion } => {
                apply_promotion(&mut next, revision, promotion)?;
            }
            Op::Forget { context } => {
                if context == ComposerContextKey::NewConversation {
                    return Err(Error::invalid(
                        "cannot remove the global new-conversation input",
                    ));
                }
                validate_context(&context)?;
                next.state
                    .bindings
                    .retain(|binding| binding.context != context);
                next.state
                    .targets
                    .retain(|target| target.context != context);
            }
        }
        if next.state != self.stored.state {
            next.state.revision += 1;
            self.publish(next)?;
        }
        Ok(self.stored.state.clone())
    }

    fn validate_content(&self, content: &ComposerInputContent) -> Result<(), Error> {
        let mut attachments = BTreeSet::new();
        if content.attachments.iter().any(|attachment| {
            !attachments.insert(&attachment.id) || !self.stored.attachments.contains(attachment)
        }) {
            return Err(Error::invalid("composer attachment reference"));
        }
        Ok(())
    }

    fn publish(&mut self, next: Stored) -> Result<(), Error> {
        commit(&self.path, &serde_json::to_vec_pretty(&next)?)?;
        self.stored = next;
        Ok(())
    }

    pub(crate) fn restore_queued_turn(
        &mut self,
        conversation_id: &str,
        request: &crate::local::TurnRequest,
    ) -> Result<(), Error> {
        let owner_id = self.owner_id();
        let context = ComposerContextKey::Conversation {
            owner_id: owner_id.clone(),
            conversation_id: conversation_id.into(),
        };
        let mut next = self.stored.clone();
        let state = &mut next.state;
        let revision = state
            .revision
            .checked_add(1)
            .ok_or_else(|| Error::invalid("共享输入修订已达上限"))?;
        let input_id = identity();
        state.bindings.retain(|binding| binding.context != context);
        state.bindings.push(ComposerInputBinding {
            context: context.clone(),
            input_id: input_id.clone(),
            revision,
        });
        let mut target = self.target(&context).unwrap_or(ComposerTarget {
            context,
            revision: 0,
            choices: ComposerTargetChoices::default(),
        });
        target.choices.choice_edited = true;
        target.choices.runtime_id = Some(request.runtime_id.clone());
        target.choices.runtime_owner_id = Some(owner_id.clone());
        target.choices.choices.retain(|choice| {
            choice.owner_id != owner_id || choice.runtime_id != request.runtime_id
        });
        let choice = TurnChoice {
            model: request.selection.clone(),
            effort: request.turn.effort.clone(),
            permission_mode: request.turn.permission_mode.clone(),
        };
        if !choice.is_empty() {
            target.choices.choices.push(ComposerRuntimeChoice {
                owner_id,
                runtime_id: request.runtime_id.clone(),
                choice,
            });
        }
        validate_choices(&target.choices)?;
        target.revision = target
            .revision
            .checked_add(1)
            .ok_or_else(|| Error::invalid("目标执行配置修订已达上限"))?;
        state
            .targets
            .retain(|record| record.context != target.context);
        state.targets.push(target);

        let mut added = Vec::new();
        let result = (|| {
            let mut attachments: Vec<ComposerAttachment> = Vec::new();
            for path in &request.turn.attachments {
                let path = Path::new(path);
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| Error::invalid("排队附件名称不可用"))?;
                if !path.is_absolute() || !fs::metadata(path)?.is_file() {
                    return Err(Error::invalid("排队附件已不可用"));
                }
                // Queue paths were resolved on acceptance. Only reuse blobs
                // whose exact stored path is present in the attachment catalog.
                let stored = path.parent().and_then(Path::file_name).and_then(|id| {
                    self.stored.attachments.iter().find(|record| {
                        id == record.id.as_str()
                            && path == self.attachments.join(&record.id).join(&record.name)
                    })
                });
                if let Some(stored) = stored.filter(|record| !attachments.contains(record)) {
                    fs::File::open(path)?;
                    attachments.push(stored.clone());
                    continue;
                }
                let bytes = crate::commands::read_content(path)?;
                let attachment = self.write_attachment(name.to_owned(), bytes)?;
                added.push(attachment.clone());
                attachments.push(attachment);
            }
            let state = &mut next.state;
            state.inputs.push(ComposerInput {
                id: input_id,
                revision: 1,
                content: ComposerInputContent {
                    text: request.turn.text.clone(),
                    attachments,
                },
            });
            state.revision = revision;
            next.attachments.extend(added.iter().cloned());
            self.publish(next)
        })();
        if result.is_err() {
            for attachment in added {
                let _ = fs::remove_dir_all(self.attachments.join(attachment.id));
            }
        }
        result
    }

    pub(crate) fn upload(
        &mut self,
        name: String,
        bytes: Vec<u8>,
    ) -> Result<ComposerAttachment, Error> {
        let attachment = self.write_attachment(name, bytes)?;
        let mut next = self.stored.clone();
        next.attachments.push(attachment.clone());
        if let Err(error) = self.publish(next) {
            let _ = fs::remove_dir_all(self.attachments.join(&attachment.id));
            return Err(error);
        }
        Ok(attachment)
    }

    fn write_attachment(&self, name: String, bytes: Vec<u8>) -> Result<ComposerAttachment, Error> {
        validate_name(&name)?;
        if bytes.len() > 20 * 1024 * 1024 {
            return Err(Error::invalid("附件名称或大小无效；单个附件最多 20 MiB"));
        }
        let attachment = ComposerAttachment {
            id: identity(),
            name,
            bytes: bytes.len() as u64,
        };
        let directory = self.attachments.join(&attachment.id);
        fs::create_dir_all(&self.attachments)?;
        fs::create_dir(&directory)?;
        if let Err(error) = commit(&directory.join(&attachment.name), &bytes) {
            let _ = fs::remove_dir_all(directory);
            return Err(error);
        }
        Ok(attachment)
    }

    pub(crate) fn read_attachment(&self, id: &str) -> Result<ComposerAttachmentContent, Error> {
        let path = self.attachment_path(id)?;
        let attachment = self
            .stored
            .attachments
            .iter()
            .find(|attachment| attachment.id == id)
            .unwrap()
            .clone();
        Ok(ComposerAttachmentContent {
            attachment,
            bytes: fs::read(path)?,
        })
    }
    pub(crate) fn attachment_path(&self, id: &str) -> Result<PathBuf, Error> {
        let attachment = self
            .stored
            .attachments
            .iter()
            .find(|attachment| attachment.id == id)
            .ok_or_else(|| Error::invalid("composer attachment does not exist"))?;
        Ok(self.attachments.join(id).join(&attachment.name))
    }

    pub(crate) fn owner_id(&self) -> String {
        self.stored.state.owner_id.clone()
    }
}

fn validate_id(id: &str) -> Result<(), Error> {
    if id.len() != 36
        || !id.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
    {
        return Err(Error::invalid("composer input identity"));
    }
    Ok(())
}

fn validate_choices(content: &ComposerTargetChoices) -> Result<(), Error> {
    let mut choices = BTreeSet::new();
    if content.runtime_id.as_ref().is_some_and(String::is_empty)
        || content.runtime_id.is_some() != content.runtime_owner_id.is_some()
    {
        return Err(Error::invalid("composer runtime source"));
    }
    if let Some(owner) = &content.runtime_owner_id {
        validate_id(owner)?;
    }
    for choice in &content.choices {
        validate_id(&choice.owner_id)?;
    }
    if content.choices.iter().any(|choice| {
        choice.runtime_id.is_empty() || !choices.insert((&choice.owner_id, &choice.runtime_id))
    }) {
        return Err(Error::invalid("composer runtime choices"));
    }
    Ok(())
}

fn validate_context(context: &ComposerContextKey) -> Result<(), Error> {
    if let ComposerContextKey::Conversation {
        owner_id,
        conversation_id,
    } = context
    {
        validate_id(owner_id)?;
        if conversation_id.is_empty() {
            return Err(Error::invalid("composer conversation identity"));
        }
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), Error> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 240
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
    {
        return Err(Error::invalid("composer attachment name"));
    }
    Ok(())
}

fn validate_state(stored: &Stored) -> Result<(), Error> {
    validate_id(&stored.state.owner_id)?;
    let mut creations = BTreeSet::new();
    for creation in &stored.state.creations {
        validate_id(&creation.input_id)?;
        validate_id(&creation.owner_id)?;
        if creation.runtime_id.is_empty()
            || creation.conversation_id.is_empty()
            || creation.revision == 0
            || creation.revision > stored.state.revision
            || !creations.insert((&creation.owner_id, &creation.conversation_id))
            || !stored
                .state
                .inputs
                .iter()
                .any(|input| input.id == creation.input_id)
        {
            return Err(Error::invalid("composer creation record"));
        }
    }
    let mut ids = BTreeSet::new();
    for input in &stored.state.inputs {
        validate_id(&input.id)?;
        if input.revision == 0 || !ids.insert(&input.id) {
            return Err(Error::invalid("composer input record"));
        }
    }
    let mut targets = BTreeSet::new();
    for target in &stored.state.targets {
        validate_context(&target.context)?;
        validate_choices(&target.choices)?;
        if target.revision == 0 || !targets.insert(&target.context) {
            return Err(Error::invalid("composer target record"));
        }
    }
    if !targets.contains(&ComposerContextKey::NewConversation) {
        return Err(Error::invalid("global new-conversation target missing"));
    }
    let mut contexts = BTreeSet::new();
    for binding in &stored.state.bindings {
        validate_context(&binding.context)?;
        if binding.revision == 0
            || binding.revision > stored.state.revision
            || !ids.contains(&binding.input_id)
            || !contexts.insert(&binding.context)
        {
            return Err(Error::invalid("composer context binding"));
        }
    }
    if !contexts.contains(&ComposerContextKey::NewConversation) {
        return Err(Error::invalid("global new-conversation input missing"));
    }
    let mut attachments = BTreeSet::new();
    for attachment in &stored.attachments {
        validate_id(&attachment.id)?;
        validate_name(&attachment.name)?;
        if !attachments.insert(&attachment.id) || attachment.bytes > 20 * 1024 * 1024 {
            return Err(Error::invalid("composer attachment record"));
        }
    }
    for input in &stored.state.inputs {
        if input
            .content
            .attachments
            .iter()
            .any(|attachment| !stored.attachments.contains(attachment))
        {
            return Err(Error::invalid("composer attachment record"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
