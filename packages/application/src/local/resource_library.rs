//! Templates applied to runtime instances as managed native entries.
//! Every write goes through the native resource API with a revision, so a change made
//! outside Velune is reported as a conflict instead of being overwritten.
use super::*;
use crate::{
    BindingResolution, BoundEntry, ResourceBinding, ResourceTemplate, ResourceTemplateContent,
    ResourceTemplateSummary, RuntimeResourceKey, RuntimeResourceKind, RuntimeResourceScope,
    RuntimeTemplateStatus, TemplateApplyOutcome, TemplateApplyReport, TemplateApplyTarget,
    TemplateSyncState,
};
use management::{ExpectedRevision, Rendering};
use std::collections::BTreeMap;
use velune_agent_runtime::management;

/// What is currently at a native key.
enum Native {
    Absent,
    Present {
        revision: String,
        fingerprint: String,
    },
}

struct PlannedWrite {
    file: management::RenderedFile,
    expected: ExpectedRevision,
    fingerprint: String,
}

struct Plan {
    outcome: TemplateApplyOutcome,
    notes: Vec<String>,
    writes: Vec<PlannedWrite>,
    deletes: Vec<(management::ResourceKey, ExpectedRevision)>,
    /// Entries whose native content already is what the template renders.
    unchanged: Vec<(String, String)>,
}

impl Plan {
    fn stopped(outcome: TemplateApplyOutcome, notes: Vec<String>) -> Self {
        Self {
            outcome,
            notes,
            writes: Vec::new(),
            deletes: Vec::new(),
            unchanged: Vec::new(),
        }
    }
}

fn management_kind(kind: RuntimeResourceKind) -> management::ResourceKind {
    match kind {
        RuntimeResourceKind::UserInstructions => management::ResourceKind::UserInstructions,
        RuntimeResourceKind::McpServers => management::ResourceKind::McpServers,
        RuntimeResourceKind::Skills => management::ResourceKind::Skills,
        RuntimeResourceKind::NativeAgentDefinitions => {
            management::ResourceKind::NativeAgentDefinitions
        }
    }
}

fn user_key(kind: RuntimeResourceKind, path: &str) -> management::ResourceKey {
    management::ResourceKey {
        kind: management_kind(kind),
        scope: management::ResourceScope::User,
        path: path.into(),
    }
}

fn canonical(content: &ResourceTemplateContent) -> Result<management::Canonical, RuntimeError> {
    Ok(serde_json::from_value(serde_json::to_value(content)?)?)
}

fn read_native(
    config: &management::Config,
    key: &management::ResourceKey,
) -> Result<Native, RuntimeError> {
    match management::read(config, key) {
        Ok(document) => Ok(Native::Present {
            revision: document.entry.revision.clone().unwrap_or_default(),
            fingerprint: management::fingerprint(key.kind, &document.bytes),
        }),
        Err(error) if error.code() == management::ErrorCode::Missing => Ok(Native::Absent),
        Err(error) => Err(error.into()),
    }
}

/// Removes the skill directories a deletion left empty, deepest first. A directory that still
/// holds the user's own files is not empty, so the native API refuses and it stays.
fn prune_directories(config: &management::Config, kind: RuntimeResourceKind, removed: &[&str]) {
    if kind != RuntimeResourceKind::Skills {
        return;
    }
    let mut directories = std::collections::BTreeSet::new();
    for path in removed {
        let mut parent = std::path::Path::new(path).parent();
        while let Some(directory) = parent.filter(|path| !path.as_os_str().is_empty()) {
            directories.insert(directory.to_path_buf());
            parent = directory.parent();
        }
    }
    // Directories are only visible, with their revisions, through the catalog.
    let Ok(catalog) = management::catalog(
        config,
        management_kind(kind),
        management::ResourceScope::User,
    ) else {
        return;
    };
    for directory in directories.iter().rev() {
        let path = directory.to_string_lossy();
        let found = catalog
            .entries
            .iter()
            .find(|entry| entry.key.path == path && entry.node == management::Node::Directory);
        if let Some(entry) = found
            && let Some(revision) = &entry.revision
        {
            let _ = management::delete(
                config,
                &entry.key,
                &ExpectedRevision::Match(revision.clone()),
            );
        }
    }
}

fn random_id() -> Result<String, RuntimeError> {
    let mut random = [0; 16];
    getrandom::fill(&mut random).map_err(|_| RuntimeError::invalid("无法生成模板标识"))?;
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
}

impl CoreRuntime {
    fn require_idle_for_resources(&self) -> Result<(), RuntimeError> {
        if self.busy() || self.collaboration.is_some() {
            return Err(RuntimeError::invalid(
                "请先结束运行中的轮次并关闭协作范围，再修改原生资源",
            ));
        }
        Ok(())
    }

    fn template(&self, id: &str) -> Result<&ResourceTemplate, RuntimeError> {
        self.resource_templates
            .iter()
            .find(|template| template.id == id)
            .ok_or_else(|| RuntimeError::invalid("资源模板不存在"))
    }

    pub(super) fn resource_templates_action(
        &mut self,
        request: &Value,
    ) -> Result<Value, RuntimeError> {
        let payload = &request["payload"];
        match payload["operation"].as_str() {
            Some("list") => Ok(json!(
                self.resource_templates
                    .iter()
                    .map(|template| ResourceTemplateSummary {
                        id: template.id.clone(),
                        name: template.name.clone(),
                        kind: template.content.kind(),
                    })
                    .collect::<Vec<_>>()
            )),
            // An explicit read returns the content, including any secret values.
            Some("read") => {
                let id = payload["templateID"].as_str().unwrap_or_default();
                Ok(serde_json::to_value(self.template(id)?)?)
            }
            Some("import") => self.import_runtime_resource(payload),
            Some("save") => {
                self.save_resource_template(serde_json::from_value(payload["template"].clone())?)
            }
            Some("delete") => {
                let id = payload["templateID"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                self.template(&id)?;
                let templates: Vec<_> = self
                    .resource_templates
                    .iter()
                    .filter(|template| template.id != id)
                    .cloned()
                    .collect();
                let bindings: Vec<_> = self
                    .resource_bindings
                    .iter()
                    .filter(|binding| binding.template_id != id)
                    .cloned()
                    .collect();
                // Bindings go first: a binding without its template is the invalid state.
                self.repository.store_resource_bindings(&bindings)?;
                self.resource_bindings = bindings;
                self.repository.store_resource_templates(&templates)?;
                self.resource_templates = templates;
                Ok(Value::Null)
            }
            _ => Err(RuntimeError::invalid("资源模板操作")),
        }
    }

    /// Reads one native entry back into an unsaved template. Nothing is bound: applying the saved
    /// template to the same runtime later finds identical content and starts managing it.
    fn import_runtime_resource(&self, payload: &Value) -> Result<Value, RuntimeError> {
        let runtime_id = payload["runtimeInstanceID"].as_str().unwrap_or_default();
        let config = self.management_config(runtime_id)?;
        let kind: RuntimeResourceKind = serde_json::from_value(payload["kind"].clone())?;
        let path = payload["path"].as_str().unwrap_or_default();
        let scope = management::ResourceScope::User;
        let (name, files) = match kind {
            RuntimeResourceKind::Skills => {
                let catalog = management::catalog(&config, management_kind(kind), scope)?;
                let prefix = format!("{path}/");
                let mut files = Vec::new();
                for entry in catalog.entries.iter().filter(|entry| {
                    entry.node == management::Node::File && entry.key.path.starts_with(&prefix)
                }) {
                    let document = management::read(&config, &entry.key)?;
                    files.push(management::SkillFile {
                        path: entry.key.path[prefix.len()..].to_owned(),
                        bytes: document.bytes,
                        executable: document.executable,
                    });
                }
                (path.to_owned(), files)
            }
            _ => {
                let document = management::read(&config, &user_key(kind, path))?;
                let name = match kind {
                    RuntimeResourceKind::UserInstructions => {
                        self.runtime_instance(runtime_id)?.name.clone()
                    }
                    RuntimeResourceKind::NativeAgentDefinitions => std::path::Path::new(path)
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    _ => path.to_owned(),
                };
                (
                    name,
                    vec![management::SkillFile {
                        path: path.to_owned(),
                        bytes: document.bytes,
                        executable: document.executable,
                    }],
                )
            }
        };
        let (content, notes) =
            management::parse(&config.variant_id, management_kind(kind), &name, files)?;
        Ok(json!(crate::ResourceTemplateImport {
            template: ResourceTemplate {
                id: String::new(),
                name,
                content: serde_json::from_value(serde_json::to_value(content)?)?,
            },
            notes,
        }))
    }

    fn save_resource_template(
        &mut self,
        mut template: ResourceTemplate,
    ) -> Result<Value, RuntimeError> {
        if template.name.trim().is_empty() {
            return Err(RuntimeError::invalid("模板名称不能为空"));
        }
        let kind = template.content.kind();
        if kind != RuntimeResourceKind::UserInstructions {
            management::validate_name(&template.name)?;
        }
        // Rendering against a variant that holds every kind checks the content itself.
        management::render(
            "claude-code-2.1.286",
            &template.name,
            &canonical(&template.content)?,
        )?;
        if self.resource_templates.iter().any(|other| {
            other.id != template.id && other.name == template.name && other.content.kind() == kind
        }) {
            return Err(RuntimeError::invalid("同类模板中已有相同名称"));
        }
        let existing = self
            .resource_templates
            .iter()
            .position(|other| other.id == template.id && !template.id.is_empty());
        if let Some(index) = existing {
            if self.resource_templates[index].content.kind() != kind {
                return Err(RuntimeError::invalid("模板的类别不能更改"));
            }
        } else {
            template.id = random_id()?;
        }
        let mut templates = self.resource_templates.clone();
        match existing {
            Some(index) => templates[index] = template.clone(),
            None => templates.push(template.clone()),
        }
        self.repository.store_resource_templates(&templates)?;
        self.resource_templates = templates;
        Ok(json!(ResourceTemplateSummary {
            id: template.id,
            name: template.name,
            kind
        }))
    }

    pub(super) fn resource_sync_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        let payload = &request["payload"];
        let applying = matches!(payload["operation"].as_str(), Some("apply" | "resolve"));
        if applying {
            self.require_idle_for_resources()?;
        }
        match payload["operation"].as_str() {
            Some("status") => {
                let runtime_id = payload["runtimeInstanceID"].as_str().unwrap_or_default();
                Ok(json!(self.sync_status(runtime_id)?))
            }
            Some("preview") | Some("apply") => {
                let template = self
                    .template(payload["templateID"].as_str().unwrap_or_default())?
                    .clone();
                let targets: Vec<TemplateApplyTarget> =
                    serde_json::from_value(payload["targets"].clone())?;
                let mut reports = Vec::new();
                for target in targets {
                    reports.push(self.apply_to_runtime(&template, &target, applying)?);
                }
                Ok(json!(reports))
            }
            Some("resolve") => {
                let runtime_id = payload["runtimeInstanceID"].as_str().unwrap_or_default();
                let template = self
                    .template(payload["templateID"].as_str().unwrap_or_default())?
                    .clone();
                let resolution: BindingResolution =
                    serde_json::from_value(payload["resolution"].clone())?;
                self.resolve_binding(runtime_id, &template, resolution)?;
                Ok(Value::Null)
            }
            _ => Err(RuntimeError::invalid("资源同步操作")),
        }
    }

    fn binding_index(&self, runtime_id: &str, template_id: &str) -> Option<usize> {
        self.resource_bindings
            .iter()
            .position(|item| item.runtime_id == runtime_id && item.template_id == template_id)
    }

    fn save_bindings(&mut self, bindings: Vec<ResourceBinding>) -> Result<(), RuntimeError> {
        self.repository.store_resource_bindings(&bindings)?;
        self.resource_bindings = bindings;
        Ok(())
    }

    /// Drops a deleted runtime's bindings; its native entries are not touched.
    pub(super) fn discard_runtime_bindings(&mut self, runtime_id: &str) {
        let bindings: Vec<_> = self
            .resource_bindings
            .iter()
            .filter(|binding| binding.runtime_id != runtime_id)
            .cloned()
            .collect();
        if bindings.len() != self.resource_bindings.len() {
            let _ = self.save_bindings(bindings);
        }
    }

    fn sync_status(&self, runtime_id: &str) -> Result<Vec<RuntimeTemplateStatus>, RuntimeError> {
        let config = self.management_config(runtime_id)?;
        let mut statuses = Vec::new();
        for binding in self
            .resource_bindings
            .iter()
            .filter(|binding| binding.runtime_id == runtime_id)
        {
            let template = self.template(&binding.template_id)?;
            let kind = template.content.kind();
            let (mut drifted, mut missing) = (Vec::new(), Vec::new());
            for entry in &binding.entries {
                let key = user_key(kind, &entry.key.path);
                match read_native(&config, &key)? {
                    Native::Absent => missing.push(entry.key.path.clone()),
                    Native::Present { fingerprint, .. } if fingerprint != entry.fingerprint => {
                        drifted.push(entry.key.path.clone())
                    }
                    Native::Present { .. } => {}
                }
            }
            let state = if !drifted.is_empty() {
                TemplateSyncState::Drifted { paths: drifted }
            } else if !missing.is_empty() {
                TemplateSyncState::Missing { paths: missing }
            } else {
                match management::render(
                    &config.variant_id,
                    &template.name,
                    &canonical(&template.content)?,
                ) {
                    Ok(Rendering::Skipped { reason }) => TemplateSyncState::Skipped { reason },
                    Ok(Rendering::Files { files, .. }) => {
                        let rendered: BTreeMap<_, _> = files
                            .iter()
                            .map(|file| {
                                (
                                    file.key.path.clone(),
                                    management::fingerprint(file.key.kind, &file.bytes),
                                )
                            })
                            .collect();
                        let bound: BTreeMap<_, _> = binding
                            .entries
                            .iter()
                            .map(|entry| (entry.key.path.clone(), entry.fingerprint.clone()))
                            .collect();
                        if rendered == bound {
                            TemplateSyncState::Synced
                        } else {
                            TemplateSyncState::TemplateUpdated
                        }
                    }
                    Err(error) => TemplateSyncState::Skipped {
                        reason: error.to_string(),
                    },
                }
            };
            statuses.push(RuntimeTemplateStatus {
                template_id: template.id.clone(),
                template_name: template.name.clone(),
                kind,
                state,
            });
        }
        Ok(statuses)
    }

    /// Compares the rendered template with the native entries and the bindings' record of what
    /// Velune wrote, and decides what applying would do.
    fn plan(
        &self,
        config: &management::Config,
        runtime_id: &str,
        template: &ResourceTemplate,
        overwrite: bool,
    ) -> Result<Plan, RuntimeError> {
        let kind = template.content.kind();
        let rendering = match management::render(
            &config.variant_id,
            &template.name,
            &canonical(&template.content)?,
        ) {
            Ok(rendering) => rendering,
            Err(error) => {
                return Ok(Plan::stopped(
                    TemplateApplyOutcome::Failed {
                        reason: error.to_string(),
                    },
                    Vec::new(),
                ));
            }
        };
        let (files, notes) = match rendering {
            Rendering::Skipped { reason } => {
                return Ok(Plan::stopped(
                    TemplateApplyOutcome::Skipped { reason },
                    Vec::new(),
                ));
            }
            Rendering::Files { files, notes } => (files, notes),
        };
        // Which template manages a key on this runtime, and what it wrote there.
        let mut holders: BTreeMap<(RuntimeResourceKind, &str), (&str, &str)> = BTreeMap::new();
        for binding in self
            .resource_bindings
            .iter()
            .filter(|binding| binding.runtime_id == runtime_id)
        {
            for entry in &binding.entries {
                holders.insert(
                    (entry.key.kind, entry.key.path.as_str()),
                    (binding.template_id.as_str(), entry.fingerprint.as_str()),
                );
            }
        }
        let (mut unmanaged, mut drifted) = (Vec::new(), Vec::new());
        let (mut writes, mut deletes, mut unchanged) = (Vec::new(), Vec::new(), Vec::new());
        for file in files {
            let rendered_fingerprint = management::fingerprint(file.key.kind, &file.bytes);
            let holder = holders.get(&(kind, file.key.path.as_str())).copied();
            let expected = match read_native(config, &file.key)? {
                Native::Absent => Some(ExpectedRevision::Missing),
                Native::Present { fingerprint, .. } if fingerprint == rendered_fingerprint => {
                    unchanged.push((file.key.path.clone(), fingerprint));
                    None
                }
                Native::Present {
                    revision,
                    fingerprint,
                } => match holder {
                    Some((owner, written)) if owner == template.id && written == fingerprint => {
                        Some(ExpectedRevision::Match(revision))
                    }
                    Some((owner, _)) if owner == template.id => {
                        drifted.push(file.key.path.clone());
                        overwrite.then_some(ExpectedRevision::Match(revision))
                    }
                    _ => {
                        unmanaged.push(file.key.path.clone());
                        overwrite.then_some(ExpectedRevision::Match(revision))
                    }
                },
            };
            if let Some(expected) = expected {
                writes.push(PlannedWrite {
                    fingerprint: rendered_fingerprint,
                    file,
                    expected,
                });
            }
        }
        // Files the template used to write and no longer renders.
        if let Some(index) = self.binding_index(runtime_id, &template.id) {
            let rendered: Vec<_> = writes
                .iter()
                .map(|write| write.file.key.path.as_str())
                .chain(unchanged.iter().map(|(path, _)| path.as_str()))
                .collect();
            let rendered_all: Vec<String> =
                rendered.iter().map(|path| (*path).to_owned()).collect();
            let stale: Vec<_> = self.resource_bindings[index]
                .entries
                .iter()
                .filter(|entry| !rendered_all.contains(&entry.key.path))
                .collect();
            for entry in stale {
                let key = user_key(kind, &entry.key.path);
                match read_native(config, &key)? {
                    Native::Absent => {}
                    Native::Present {
                        revision,
                        fingerprint,
                    } => {
                        if fingerprint != entry.fingerprint {
                            drifted.push(entry.key.path.clone());
                        }
                        if fingerprint == entry.fingerprint || overwrite {
                            deletes.push((key, ExpectedRevision::Match(revision)));
                        }
                    }
                }
            }
        }
        if (!unmanaged.is_empty() || !drifted.is_empty()) && !overwrite {
            return Ok(Plan::stopped(
                TemplateApplyOutcome::Conflict { unmanaged, drifted },
                notes,
            ));
        }
        let outcome = if writes.is_empty() && deletes.is_empty() {
            TemplateApplyOutcome::Unchanged
        } else if writes
            .iter()
            .all(|write| matches!(write.expected, ExpectedRevision::Missing))
            && deletes.is_empty()
        {
            TemplateApplyOutcome::Create
        } else {
            TemplateApplyOutcome::Update
        };
        Ok(Plan {
            outcome,
            notes,
            writes,
            deletes,
            unchanged,
        })
    }

    fn apply_to_runtime(
        &mut self,
        template: &ResourceTemplate,
        target: &TemplateApplyTarget,
        perform: bool,
    ) -> Result<TemplateApplyReport, RuntimeError> {
        let runtime_id = target.runtime_id.as_str();
        let report = |outcome, notes| TemplateApplyReport {
            runtime_id: runtime_id.to_owned(),
            outcome,
            notes,
        };
        let config = match self.management_config(runtime_id) {
            Ok(config) => config,
            Err(error) => {
                return Ok(report(
                    TemplateApplyOutcome::Failed {
                        reason: error.to_string(),
                    },
                    Vec::new(),
                ));
            }
        };
        let plan = match self.plan(&config, runtime_id, template, target.overwrite) {
            Ok(plan) => plan,
            Err(error) => {
                return Ok(report(
                    TemplateApplyOutcome::Failed {
                        reason: error.to_string(),
                    },
                    Vec::new(),
                ));
            }
        };
        let applicable = matches!(
            plan.outcome,
            TemplateApplyOutcome::Create
                | TemplateApplyOutcome::Update
                | TemplateApplyOutcome::Unchanged
        );
        if !perform || !applicable {
            return Ok(report(plan.outcome, plan.notes));
        }
        let kind = template.content.kind();
        // The entries this binding will hold, as far as the writes succeed.
        let mut entries: Vec<BoundEntry> = plan
            .unchanged
            .iter()
            .map(|(path, fingerprint)| BoundEntry {
                key: RuntimeResourceKey {
                    kind,
                    scope: RuntimeResourceScope::User,
                    path: path.clone(),
                },
                fingerprint: fingerprint.clone(),
            })
            .collect();
        let mut failure = None;
        for write in &plan.writes {
            match management::write(
                &config,
                &write.file.key,
                &write.expected,
                &write.file.bytes,
                write.file.executable,
            ) {
                Ok(_) => entries.push(BoundEntry {
                    key: RuntimeResourceKey {
                        kind,
                        scope: RuntimeResourceScope::User,
                        path: write.file.key.path.clone(),
                    },
                    fingerprint: write.fingerprint.clone(),
                }),
                Err(error) => {
                    failure = Some(error.to_string());
                    break;
                }
            }
        }
        if failure.is_none() {
            for (key, expected) in &plan.deletes {
                if let Err(error) = management::delete(&config, key, expected) {
                    failure = Some(error.to_string());
                    break;
                }
            }
        }
        if failure.is_none() {
            let removed: Vec<_> = plan
                .deletes
                .iter()
                .map(|(key, _)| key.path.as_str())
                .collect();
            prune_directories(&config, kind, &removed);
        }
        // Other templates lose a key this one took over.
        let mut bindings = self.resource_bindings.clone();
        for binding in &mut bindings {
            if binding.runtime_id == runtime_id && binding.template_id != template.id {
                binding
                    .entries
                    .retain(|entry| !entries.iter().any(|mine| mine.key == entry.key));
            }
        }
        bindings.retain(|binding| {
            !(binding.runtime_id == runtime_id
                && binding.entries.is_empty()
                && binding.template_id != template.id)
        });
        match bindings
            .iter()
            .position(|item| item.runtime_id == runtime_id && item.template_id == template.id)
        {
            Some(index) if entries.is_empty() => {
                bindings.remove(index);
            }
            Some(index) => bindings[index].entries = entries,
            None if entries.is_empty() => {}
            None => bindings.push(ResourceBinding {
                runtime_id: runtime_id.to_owned(),
                template_id: template.id.clone(),
                entries,
            }),
        }
        self.save_bindings(bindings)?;
        Ok(match failure {
            Some(reason) => report(TemplateApplyOutcome::Failed { reason }, plan.notes),
            None => report(plan.outcome, plan.notes),
        })
    }

    fn resolve_binding(
        &mut self,
        runtime_id: &str,
        template: &ResourceTemplate,
        resolution: BindingResolution,
    ) -> Result<(), RuntimeError> {
        let Some(index) = self.binding_index(runtime_id, &template.id) else {
            return Err(RuntimeError::invalid("此运行时没有应用该模板"));
        };
        match resolution {
            BindingResolution::Overwrite => {
                let report = self.apply_to_runtime(
                    template,
                    &TemplateApplyTarget {
                        runtime_id: runtime_id.to_owned(),
                        overwrite: true,
                    },
                    true,
                )?;
                match report.outcome {
                    TemplateApplyOutcome::Failed { reason } => Err(RuntimeError::Invalid(reason)),
                    TemplateApplyOutcome::Skipped { reason } => Err(RuntimeError::Invalid(reason)),
                    _ => Ok(()),
                }
            }
            BindingResolution::Unmanage => {
                let mut bindings = self.resource_bindings.clone();
                bindings.remove(index);
                self.save_bindings(bindings)
            }
            BindingResolution::Remove => {
                let config = self.management_config(runtime_id)?;
                let kind = template.content.kind();
                let entries = self.resource_bindings[index].entries.clone();
                let mut removed = Vec::new();
                for entry in &entries {
                    let key = user_key(kind, &entry.key.path);
                    if let Native::Present { revision, .. } = read_native(&config, &key)? {
                        management::delete(&config, &key, &ExpectedRevision::Match(revision))?;
                    }
                    removed.push(entry.key.path.as_str());
                }
                prune_directories(&config, kind, &removed);
                let mut bindings = self.resource_bindings.clone();
                bindings.remove(index);
                self.save_bindings(bindings)
            }
        }
    }
}
