//! Presentation state belongs to a client view; execution sessions remain shared.
use super::*;

#[derive(Clone)]
pub(crate) struct ClientState {
    session: session::SessionKey,
    window: organization::BrowserWindow,
    pub(super) preferences: crate::config::ConversationBrowserPreferences,
    pub(super) limits: crate::ConversationBrowserLimits,
    pub(super) outline: crate::config::OutlinePresentation,
    pub(super) linked_opening: crate::config::LinkedConversationOpening,
    pub(super) usage_display: crate::config::UsageDisplay,
    pub(super) status: Vec<crate::config::StatusLineItem>,
    drafts: BTreeMap<String, String>,
    runtime: Option<String>,
    listed: BTreeMap<String, organization::ListedSource>,
    archived: Vec<ConversationSummary>,
}
impl CoreRuntime {
    pub(crate) fn retain_private_runtime_settings(&self, runtime: &mut RuntimeInstance) {
        if !runtime.settings.contains_key("acpLaunch")
            && let Some(launch) = self
                .runtime_instances
                .iter()
                .find(|stored| stored.id == runtime.id && stored.type_id == runtime.type_id)
                .and_then(|stored| stored.settings.get("acpLaunch"))
        {
            runtime.settings.insert("acpLaunch".into(), launch.clone());
        }
    }

    pub(crate) fn client_state(&self) -> ClientState {
        ClientState {
            session: session::SessionKey::default(),
            window: Default::default(),
            preferences: self.conversation_browser_preferences.clone(),
            limits: self.conversation_browser_limits,
            outline: self.outline_presentation,
            linked_opening: self.linked_conversation_opening,
            usage_display: self.usage_display,
            status: self.status_line.clone(),
            drafts: self.composer_drafts.clone(),
            runtime: None,
            listed: Default::default(),
            archived: Vec::new(),
        }
    }
    pub(crate) fn enter_client_state(&mut self, state: &mut ClientState, isolated: bool) {
        self.local_presentation_commit.borrow_mut().take();
        self.client_baseline = isolated.then(|| Box::new(self.client_state()));
        self.swap_view_fields(state);
        let current = self.session.key;
        self.focus_key(state.session);
        state.session = current;
    }
    pub(crate) fn leave_client_state(&mut self, state: &mut ClientState) {
        let current = self.session.key;
        self.focus_key(state.session);
        state.session = current;
        self.swap_view_fields(state);
        if let Some(committed) = self.local_presentation_commit.borrow_mut().take() {
            // Use the committed values even if response construction later failed or changed a view.
            // Existing views keep their choices; failed or isolated saves cannot change defaults.
            self.conversation_browser_preferences = committed.preferences;
            self.conversation_browser_limits = committed.limits;
            self.outline_presentation = committed.outline;
            self.linked_conversation_opening = committed.linked_opening;
            self.usage_display = committed.usage_display;
            self.status_line = committed.status;
        }
        self.client_baseline = None;
    }
    fn swap_view_fields(&mut self, state: &mut ClientState) {
        std::mem::swap(&mut self.browser_window, &mut state.window);
        std::mem::swap(
            &mut self.conversation_browser_preferences,
            &mut state.preferences,
        );
        std::mem::swap(&mut self.conversation_browser_limits, &mut state.limits);
        std::mem::swap(&mut self.outline_presentation, &mut state.outline);
        std::mem::swap(
            &mut self.linked_conversation_opening,
            &mut state.linked_opening,
        );
        std::mem::swap(&mut self.usage_display, &mut state.usage_display);
        std::mem::swap(&mut self.status_line, &mut state.status);
        std::mem::swap(&mut self.composer_drafts, &mut state.drafts);
        std::mem::swap(&mut self.next_turn_runtime_id, &mut state.runtime);
        std::mem::swap(self.last_listed.get_mut(), &mut state.listed);
        std::mem::swap(self.archived_listing.get_mut(), &mut state.archived);
    }
    pub(crate) fn advance(&mut self) -> Result<(), RuntimeError> {
        let result = self.drain_sessions();
        self.serve_collaboration();
        self.sync_due_resources();
        self.trim_sessions();
        result
    }
}
