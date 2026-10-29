import Foundation

import VeluneBindings

/// The Mac's own aggregates of generated records: a conversation joins its
/// summary with the list's organization, a snapshot can also be a local draft
/// or preview, and quota history is pivoted into series. Everything else uses
/// the generated contract types directly (see ContractExtensions.swift).
enum BindingMapping {
    static func conversation(_ value: ConversationSummary) -> Conversation {
        Conversation(id: value.id, title: value.title.displayText(), updatedAtUnixMs: value.updatedAtUnixMs, createdAtUnixMs: value.createdAtUnixMs,
                     runtimeID: value.runtimeId, cwd: value.cwd, canRename: value.canRename, canDelete: value.canDelete,
                     parentConversationID: value.parentConversationId)
    }

    static func listed(_ value: ListedConversation) -> Conversation {
        var mapped = conversation(value.conversation)
        mapped.nativeID = value.nativeId; mapped.pinned = value.pinned; mapped.unread = value.unread
        mapped.agentsURI = value.agentsUri
        mapped.archived = value.archived; mapped.project = value.project; mapped.moved = value.moved; mapped.failed = value.failed; mapped.group = value.group
        return mapped
    }

    /// A complete delivery. An incremental one (non-nil `base`) carries only
    /// changed messages; see `snapshotUpdate`.
    static func snapshot(_ value: DeliveredSnapshot) -> ConversationSnapshot {
        var mapped = snapshotHeader(value)
        mapped.messages = value.messages
        return mapped
    }

    /// Every field except messages, which an incremental update merges by identity.
    static func snapshotHeader(_ value: DeliveredSnapshot) -> ConversationSnapshot {
        ConversationSnapshot(cursor: value.cursor, revision: value.revision, conversation: conversation(value.conversation), contextRuntimeID: value.contextRuntimeId,
                             modelRecordKey: value.modelRecordKey, runState: value.runState,
                             messages: [], transcriptTurns: value.transcriptTurns, usage: value.usage, transcriptItems: value.transcriptItems, transcriptOutline: value.transcriptOutline,
                             transcriptMessageIdentities: value.transcriptMessageIdentities, messageIdentityConfirmations: value.messageIdentityConfirmations,
                             pendingInteractions: value.pendingInteractions, actions: value.actions, followUp: value.followUp,
                             subAgents: value.subAgents,
                             collaborators: value.collaborators)
    }

    static func quotaObservation(_ value: QuotaHistoryPoint) -> QuotaHistorySample {
        var values: [QuotaMetric: Double] = [:]
        for window in value.windows { values[.window(id: window.id, modelIDs: window.modelIds)] = window.usedPercent }
        for balance in value.balances { values[.balance(id: balance.id, unit: balance.unit)] = balance.amount }
        return QuotaHistorySample(observedAt: Date(timeIntervalSince1970: Double(value.observedAtMs) / 1000), available: value.available, values: values)
    }
}
