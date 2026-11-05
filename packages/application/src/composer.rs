//! The application owner persists shared input independently of client views.
use crate::{Error, repository::commit};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
use velune_application_api::*;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    schema_version: u32,
    state: ComposerInputState,
    attachments: Vec<ComposerAttachment>,
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
                    ComposerContextKey::NewConversation => "draft:new-conversation".to_owned(),
                    ComposerContextKey::Conversation {
                        owner_id,
                        conversation_id,
                    } if owner_id == &self.stored.state.owner_id => conversation_id.clone(),
                    _ => return None,
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
                    ComposerContextKey::NewConversation => "draft:new-conversation".into(),
                    ComposerContextKey::Conversation {
                        owner_id,
                        conversation_id,
                    } if owner_id == &self.owner_id() => conversation_id.clone(),
                    _ => return None,
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
        if id.is_empty() || id.starts_with("draft:") {
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
            });
        }
        let state = self.operate(operation)?;
        Ok(ComposerInputUpdate {
            revision: state.revision,
            state: Some(state),
        })
    }
    pub(crate) fn open(home: &Path) -> Result<Self, Error> {
        let path = home.join("composer-inputs.json");
        let stored = match fs::read(&path) {
            Ok(bytes) => {
                let value: serde_json::Value = serde_json::from_slice(&bytes)?;
                if value.get("schemaVersion").and_then(|value| value.as_u64()) == Some(1) {
                    return Err(Error::Invalid("输入存储需要显式迁移：请退出 Velune 后执行 python3 scripts/migrations/composer-state.py --home <VELUNE_HOME> --apply；现有输入保持原样".into()));
                }
                let stored: Stored = serde_json::from_value(value)?;
                if stored.schema_version != 2 {
                    return Err(Error::invalid("composer input schema version"));
                }
                validate_state(&stored)?;
                stored
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let journals = home.join("mac-window-workspaces");
                let legacy_journal = if journals.is_dir() {
                    fs::read_dir(journals)?.try_fold(
                        false,
                        |found, entry| -> Result<bool, Error> {
                            let path = entry?.path();
                            if path.extension().is_none_or(|extension| extension != "json") {
                                return Ok(found);
                            }
                            let value: serde_json::Value =
                                serde_json::from_slice(&fs::read(path)?)?;
                            Ok(found
                                || value.get("composerWorkspace").is_some_and(|workspace| {
                                    workspace
                                        .get("schemaVersion")
                                        .and_then(|version| version.as_u64())
                                        != Some(3)
                                })
                                || value.get("composer").is_some_and(|value| !value.is_null()))
                        },
                    )?
                } else {
                    false
                };
                if home.join("composer-drafts.json").exists()
                    || home.join("composer-choices.json").exists()
                    || legacy_journal
                {
                    return Err(Error::Invalid("输入存储需要显式迁移：请退出 Velune 后执行 python3 scripts/migrations/composer-state.py --home <VELUNE_HOME> --apply；现有输入保持原样".into()));
                }
                let input = ComposerInput {
                    id: identity(),
                    revision: 1,
                    content: Default::default(),
                };
                let stored = Stored {
                    schema_version: 2,
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
                let context = ComposerContextKey::Conversation {
                    owner_id: owner_id.clone(),
                    conversation_id: conversation_id.clone(),
                };
                validate_context(&context)?;
                if runtime_id.is_empty() {
                    return Err(Error::invalid("composer creation runtime"));
                }
                if let Some(creation) = next.state.creations.iter().find(|creation| {
                    creation.owner_id == owner_id && creation.conversation_id == conversation_id
                }) {
                    if creation.input_id != input_id || creation.runtime_id != runtime_id {
                        return Err(Error::invalid("composer creation identity changed"));
                    }
                    return Ok(self.stored.state.clone());
                }
                next.state.creations.push(ComposerCreation {
                    input_id: input_id.clone(),
                    owner_id: owner_id.clone(),
                    runtime_id: runtime_id.clone(),
                    conversation_id,
                    revision: next.state.revision + 1,
                });
                if !next.state.inputs.iter().any(|input| input.id == input_id) {
                    return Err(Error::invalid("composer input does not exist"));
                }
                // The native target starts from the creation's execution snapshot;
                // the global new target remains independent for the next creation.
                if !next
                    .state
                    .targets
                    .iter()
                    .any(|target| target.context == context)
                {
                    let choices = ComposerTargetChoices {
                        runtime_id: Some(runtime_id),
                        runtime_owner_id: Some(owner_id),
                        ..Default::default()
                    };
                    next.state.targets.push(ComposerTarget {
                        context: context.clone(),
                        revision: 1,
                        choices,
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

    pub(crate) fn upload(
        &mut self,
        name: String,
        bytes: Vec<u8>,
    ) -> Result<ComposerAttachment, Error> {
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
        fs::create_dir_all(&directory)?;
        commit(&directory.join(&attachment.name), &bytes)?;
        let mut next = self.stored.clone();
        next.attachments.push(attachment.clone());
        self.publish(next)?;
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
mod tests {
    use super::*;
    use crate::{ApplicationService, Options};

    fn call(
        service: &ApplicationService,
        view: &str,
        operation: ComposerInputOperation,
    ) -> ComposerInputState {
        let command = Command::ManageComposerInputs { operation };
        let command = serde_json::from_slice(&serde_json::to_vec(&command).unwrap()).unwrap();
        match service
            .execute_checked(
                ClientViewId(view.into()),
                ExecutionContext {
                    expected_revision: None,
                },
                command,
            )
            .unwrap()
            .response
        {
            Response::ManageComposerInputs(update) => update.state.unwrap(),
            _ => panic!("unexpected response"),
        }
    }

    #[test]
    fn borrowed_input_promotes_without_consuming_new_and_legacy_edits_preserve_content() {
        let home = std::env::temp_dir().join(format!("velune-composer-{}", identity()));
        fs::create_dir_all(&home).unwrap();
        let mut store = ComposerInputs::open(&home).unwrap();
        let new_id = store.stored.state.bindings[0].input_id.clone();
        let attachment = store
            .upload("work.txt".into(), b"synthetic work".to_vec())
            .unwrap();
        let input_id = identity();
        let context = ComposerContextKey::Conversation {
            owner_id: store.owner_id(),
            conversation_id: "A".into(),
        };
        let content = ComposerInputContent {
            text: "borrowed X".into(),
            attachments: vec![attachment.clone()],
        };
        store
            .operate(ComposerInputOperation::Ensure {
                context: context.clone(),
                input_id: input_id.clone(),
                seed: content,
            })
            .unwrap();
        store
            .operate(ComposerInputOperation::EnsureTarget {
                context: context.clone(),
                seed: ComposerTargetChoices {
                    runtime_id: Some("chosen-runtime".into()),
                    runtime_owner_id: Some(store.owner_id()),
                    choices: vec![
                        ComposerRuntimeChoice {
                            owner_id: store.owner_id(),
                            runtime_id: "same-runtime-id".into(),
                            choice: Default::default(),
                        },
                        ComposerRuntimeChoice {
                            owner_id: identity(),
                            runtime_id: "same-runtime-id".into(),
                            choice: Default::default(),
                        },
                    ],
                    ..Default::default()
                },
            })
            .unwrap();
        store
            .legacy_edit("A".into(), Some("edited X".into()), None)
            .unwrap();
        let input = store.legacy_input("A").unwrap();
        assert_eq!(input.content.attachments, vec![attachment]);
        assert_eq!(
            store
                .stored
                .state
                .targets
                .iter()
                .find(|target| target.context == context)
                .unwrap()
                .choices
                .choices
                .len(),
            2,
            "same runtime ID on different execution hosts must remain distinct"
        );
        assert_eq!(
            store
                .stored
                .state
                .targets
                .iter()
                .find(|target| target.context == context)
                .unwrap()
                .choices
                .runtime_id
                .as_deref(),
            Some("chosen-runtime")
        );
        let state = store
            .operate(ComposerInputOperation::Promote {
                input_id: input_id.clone(),
                owner_id: store.owner_id(),
                conversation_id: "created-from-X".into(),
                runtime_id: "chosen-runtime".into(),
            })
            .unwrap();
        assert_eq!(
            state
                .bindings
                .iter()
                .find(|binding| binding.context == ComposerContextKey::NewConversation)
                .unwrap()
                .input_id,
            new_id
        );
        assert_eq!(
            state
                .bindings
                .iter()
                .filter(|binding| binding.input_id == input_id)
                .count(),
            2
        );
        let unchanged = store
            .update(ComposerInputOperation::Read {
                since_revision: Some(state.revision),
            })
            .unwrap();
        assert!(unchanged.state.is_none());
        assert_eq!(state.creations.len(), 1);
        let rebound = store
            .operate(ComposerInputOperation::Bind {
                context: ComposerContextKey::NewConversation,
                input_id: input_id.clone(),
                expected_revision: Some(
                    state
                        .bindings
                        .iter()
                        .find(|binding| binding.context == ComposerContextKey::NewConversation)
                        .unwrap()
                        .revision,
                ),
            })
            .unwrap();
        let replayed = store
            .operate(ComposerInputOperation::Promote {
                input_id: input_id.clone(),
                owner_id: store.owner_id(),
                conversation_id: "created-from-X".into(),
                runtime_id: "chosen-runtime".into(),
            })
            .unwrap();
        assert_eq!(
            replayed, rebound,
            "a recovered creation must not consume a later new-conversation binding"
        );
        let stale = store.operate(ComposerInputOperation::Bind {
            context: ComposerContextKey::NewConversation,
            input_id: new_id.clone(),
            expected_revision: Some(1),
        });
        assert!(matches!(
            stale,
            Err(Error::Contract(Failure {
                kind: FailureKind::Conflict,
                ..
            }))
        ));
        let input = state
            .inputs
            .iter()
            .find(|input| input.id == input_id)
            .unwrap();
        store
            .operate(ComposerInputOperation::Save {
                input_id: input_id.clone(),
                expected_revision: Some(input.revision),
                content: Default::default(),
            })
            .unwrap();
        assert!(
            store
                .legacy_drafts()
                .iter()
                .filter(|draft| draft.conversation_id == "A"
                    || draft.conversation_id == "created-from-X")
                .all(|draft| draft.text.is_empty())
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn directory_and_attachments_outlive_views_and_native_promotion() {
        let home = std::env::temp_dir().join(format!("velune-composer-{}", identity()));
        fs::create_dir_all(&home).unwrap();
        let options = Options {
            home_directory: home.clone(),
            resources_directory: home.clone(),
        };
        let service = ApplicationService::open(options.clone()).unwrap();
        let first = call(
            &service,
            "desktop",
            ComposerInputOperation::Read {
                since_revision: None,
            },
        );
        let input_id = first.bindings[0].input_id.clone();
        let attachment = match service
            .execute_checked(
                ClientViewId("desktop".into()),
                ExecutionContext {
                    expected_revision: None,
                },
                Command::UploadComposerAttachment {
                    name: "pasted.png".into(),
                    bytes: b"synthetic-image".to_vec(),
                },
            )
            .unwrap()
            .response
        {
            Response::UploadComposerAttachment(attachment) => attachment,
            _ => panic!("unexpected attachment response"),
        };
        let payload = ComposerInputContent {
            text: "UNSENT X".into(),
            attachments: vec![attachment.clone()],
        };
        call(
            &service,
            "desktop",
            ComposerInputOperation::SaveTarget {
                context: ComposerContextKey::NewConversation,
                expected_revision: Some(1),
                choices: ComposerTargetChoices {
                    runtime_id: Some("runtime-a".into()),
                    runtime_owner_id: Some(first.owner_id.clone()),
                    choices: vec![ComposerRuntimeChoice {
                        owner_id: first.owner_id.clone(),
                        runtime_id: "runtime-a".into(),
                        choice: TurnChoice {
                            model: Some(ModelSelection::Native {
                                model_id: "model-a".into(),
                                provider_id: None,
                            }),
                            effort: Some("high".into()),
                            permission_mode: Some("allow-edits".into()),
                        },
                    }],
                    choice_edited: true,
                },
            },
        );
        let written = call(
            &service,
            "desktop",
            ComposerInputOperation::Save {
                input_id: input_id.clone(),
                expected_revision: Some(1),
                content: payload.clone(),
            },
        );
        let phone = call(
            &service,
            "phone",
            ComposerInputOperation::Read {
                since_revision: None,
            },
        );
        assert_eq!(
            written, phone,
            "the directory must not swap with ClientState"
        );
        service
            .execute(ClientViewId("desktop".into()), Command::ReleaseView)
            .unwrap();
        let reopened = call(
            &service,
            "another-view",
            ComposerInputOperation::Ensure {
                context: ComposerContextKey::NewConversation,
                input_id: identity(),
                seed: Default::default(),
            },
        );
        assert_eq!(reopened, phone, "opening new must not reset its state");
        let promoted = call(
            &service,
            "phone",
            ComposerInputOperation::Promote {
                input_id: input_id.clone(),
                owner_id: first.owner_id.clone(),
                conversation_id: "created-native-session".into(),
                runtime_id: "runtime-a".into(),
            },
        );
        assert_eq!(
            promoted
                .inputs
                .iter()
                .find(|input| input.id == input_id)
                .unwrap()
                .content,
            payload
        );
        let new_id = &promoted
            .bindings
            .iter()
            .find(|binding| binding.context == ComposerContextKey::NewConversation)
            .unwrap()
            .input_id;
        assert_ne!(new_id, &input_id);
        let next = &promoted
            .inputs
            .iter()
            .find(|input| &input.id == new_id)
            .unwrap()
            .content;
        assert!(next.text.is_empty() && next.attachments.is_empty());
        assert_eq!(
            promoted
                .targets
                .iter()
                .find(|target| target.context == ComposerContextKey::NewConversation)
                .unwrap()
                .choices
                .runtime_id
                .as_deref(),
            Some("runtime-a")
        );
        let bytes = match service
            .execute_checked(
                ClientViewId("another-view".into()),
                ExecutionContext {
                    expected_revision: None,
                },
                Command::ReadComposerAttachment {
                    attachment_id: attachment.id.clone(),
                },
            )
            .unwrap()
            .response
        {
            Response::ReadComposerAttachment(content) => content.bytes,
            _ => panic!("unexpected attachment content"),
        };
        assert_eq!(bytes, b"synthetic-image");
        service.close(false).unwrap();
        let service = ApplicationService::open(options).unwrap();
        assert_eq!(
            call(
                &service,
                "after-restart",
                ComposerInputOperation::Read {
                    since_revision: None
                }
            ),
            promoted
        );
        service.close(false).unwrap();
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn content_and_target_choices_have_independent_revisions_and_stale_targets_cannot_win() {
        let home = std::env::temp_dir().join(format!("velune-composer-{}", identity()));
        fs::create_dir_all(&home).unwrap();
        let mut store = ComposerInputs::open(&home).unwrap();
        let id = store.stored.state.inputs[0].id.clone();
        let owner = store.owner_id();
        let target = ComposerContextKey::Conversation {
            owner_id: owner.clone(),
            conversation_id: "pi-target".into(),
        };
        let pi = ComposerTargetChoices {
            runtime_id: Some("pi".into()),
            runtime_owner_id: Some(owner),
            choice_edited: true,
            ..Default::default()
        };
        store
            .operate(ComposerInputOperation::SaveTarget {
                context: target.clone(),
                expected_revision: None,
                choices: pi.clone(),
            })
            .unwrap();
        store
            .operate(ComposerInputOperation::Save {
                input_id: id.clone(),
                expected_revision: Some(1),
                content: ComposerInputContent {
                    text: "retry".into(),
                    ..Default::default()
                },
            })
            .unwrap();
        assert_eq!(
            store
                .stored
                .state
                .targets
                .iter()
                .find(|value| value.context == target)
                .unwrap()
                .revision,
            1
        );
        let mut changed = pi.clone();
        changed.choices.push(ComposerRuntimeChoice {
            owner_id: store.owner_id(),
            runtime_id: "pi".into(),
            choice: TurnChoice {
                effort: Some("high".into()),
                ..Default::default()
            },
        });
        store
            .operate(ComposerInputOperation::SaveTarget {
                context: target.clone(),
                expected_revision: Some(1),
                choices: changed.clone(),
            })
            .unwrap();
        assert!(
            store
                .operate(ComposerInputOperation::SaveTarget {
                    context: target.clone(),
                    expected_revision: Some(1),
                    choices: pi
                })
                .is_err()
        );
        let content = store
            .stored
            .state
            .inputs
            .iter()
            .find(|input| input.id == id)
            .unwrap();
        assert_eq!(content.revision, 2);
        assert_eq!(content.content.text, "retry");
        assert_eq!(
            store
                .stored
                .state
                .targets
                .iter()
                .find(|value| value.context == target)
                .unwrap()
                .choices,
            changed
        );
        drop(store);
        let reopened = ComposerInputs::open(&home).unwrap();
        assert_eq!(
            reopened
                .stored
                .state
                .targets
                .iter()
                .find(|value| value.context == target)
                .unwrap()
                .choices
                .runtime_id
                .as_deref(),
            Some("pi")
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn stale_write_cannot_resurrect_consumed_input_or_attachment_path() {
        let home = std::env::temp_dir().join(format!("velune-composer-{}", identity()));
        fs::create_dir_all(&home).unwrap();
        let mut store = ComposerInputs::open(&home).unwrap();
        let state = store
            .operate(ComposerInputOperation::Read {
                since_revision: None,
            })
            .unwrap();
        let id = state.bindings[0].input_id.clone();
        let written = store
            .operate(ComposerInputOperation::Save {
                input_id: id.clone(),
                expected_revision: Some(1),
                content: ComposerInputContent {
                    text: "X".into(),
                    ..Default::default()
                },
            })
            .unwrap();
        let revision = written
            .inputs
            .iter()
            .find(|input| input.id == id)
            .unwrap()
            .revision;
        store
            .operate(ComposerInputOperation::Save {
                input_id: id.clone(),
                expected_revision: Some(revision),
                content: Default::default(),
            })
            .unwrap();
        let failure = store
            .operate(ComposerInputOperation::Save {
                input_id: id.clone(),
                expected_revision: Some(revision),
                content: ComposerInputContent {
                    text: "STALE X".into(),
                    ..Default::default()
                },
            })
            .unwrap_err();
        assert!(matches!(
            failure,
            Error::Contract(Failure {
                kind: FailureKind::Conflict,
                ..
            })
        ));
        assert!(
            store
                .operate(ComposerInputOperation::Read {
                    since_revision: None
                })
                .unwrap()
                .inputs
                .iter()
                .find(|input| input.id == id)
                .unwrap()
                .content
                .text
                .is_empty()
        );
        assert!(store.upload("../escape.png".into(), vec![0]).is_err());
        fs::remove_dir_all(home).unwrap();
    }
}
