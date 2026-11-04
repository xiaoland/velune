package app.velune.android

import app.velune.bindings.application.ComposerAttachment
import app.velune.bindings.application.ComposerInput
import app.velune.bindings.application.ComposerInputContent
import app.velune.bindings.application.ComposerTargetChoices
import app.velune.bindings.application.ComposerRuntimeChoice
import app.velune.bindings.application.TurnChoice
import app.velune.bindings.conversation.ModelSelection
import org.json.JSONArray
import org.json.JSONObject

internal data class PendingComposerCreation(val inputId: String, val ownerId: String, val runtimeId: String, val conversationId: String, val choices: ComposerTargetChoices? = null)

/** Content and target caches keep independent pending baselines. */
internal data class SavedComposerInput(
    val input: ComposerInput, val pending: Boolean, val baseline: ULong?, val unbound: Boolean,
    val directoryRevision: ULong?, val creation: PendingComposerCreation?, val bindingRevision: ULong?, val bindingBaseline: ULong?,
    val target: ComposerTargetChoices? = null, val targetRevision: ULong? = null,
    val targetPending: Boolean = false, val targetBaseline: ULong? = null,
) {
    fun encode(): String = JSONObject().put("version", 2).put("id", input.id).put("revision", input.revision.toString())
        .put("text", input.content.text).put("attachments", JSONArray(input.content.attachments.map { JSONObject().put("id", it.id).put("name", it.name).put("bytes", it.bytes.toString()) }))
        .put("pending", pending).put("baseline", baseline?.toString()).put("unbound", unbound)
        .put("directoryRevision", directoryRevision?.toString()).put("bindingRevision", bindingRevision?.toString()).put("bindingBaseline", bindingBaseline?.toString())
        .put("target", target?.let(::encodeTarget)).put("targetRevision", targetRevision?.toString()).put("targetPending", targetPending).put("targetBaseline", targetBaseline?.toString())
        .put("creation", creation?.let { JSONObject().put("input", it.inputId).put("owner", it.ownerId).put("runtime", it.runtimeId).put("conversation", it.conversationId).put("choices", it.choices?.let(::encodeTarget)) }).toString()

    companion object {
        fun decode(value: String): SavedComposerInput {
            val json = JSONObject(value)
            require(json.getInt("version") == 2) { "本机输入需要显式迁移，原数据已保留" }
            val files = json.getJSONArray("attachments")
            val content = ComposerInputContent(json.getString("text"), List(files.length()) { index -> files.getJSONObject(index).let { ComposerAttachment(it.getString("id"), it.getString("name"), it.getString("bytes").toULong()) } })
            return SavedComposerInput(
                ComposerInput(json.getString("id"), json.getString("revision").toULong(), content),
                json.getBoolean("pending"), json.optional("baseline")?.toULong(), json.getBoolean("unbound"), json.optional("directoryRevision")?.toULong(),
                json.optJSONObject("creation")?.let { PendingComposerCreation(it.getString("input"), it.getString("owner"), it.getString("runtime"), it.getString("conversation"), it.optJSONObject("choices")?.let(::decodeTarget)) },
                json.optional("bindingRevision")?.toULong(), json.optional("bindingBaseline")?.toULong(),
                json.optJSONObject("target")?.let(::decodeTarget), json.optional("targetRevision")?.toULong(), json.optBoolean("targetPending"), json.optional("targetBaseline")?.toULong(),
            )
        }
    }
}

private fun JSONObject.optional(key: String): String? = if (isNull(key) || !has(key)) null else getString(key)
private fun encodeModel(value: ModelSelection?): JSONObject? = when (value) {
    null -> null
    is ModelSelection.Managed -> JSONObject().put("managed", value.modelRecordKey)
    is ModelSelection.Native -> JSONObject().put("native", value.modelId).put("provider", value.providerId)
}
private fun decodeModel(value: JSONObject): ModelSelection = value.optional("managed")?.let(ModelSelection::Managed)
    ?: ModelSelection.Native(providerId = value.optional("provider"), modelId = value.getString("native"))
private fun encodeTarget(value: ComposerTargetChoices): JSONObject = JSONObject()
    .put("runtime", value.runtimeId).put("owner", value.runtimeOwnerId).put("edited", value.choiceEdited)
    .put("choices", JSONArray(value.choices.map { JSONObject().put("owner", it.ownerId).put("runtime", it.runtimeId)
        .put("model", encodeModel(it.choice.model)).put("effort", it.choice.effort).put("permission", it.choice.permissionMode) }))
private fun decodeTarget(value: JSONObject): ComposerTargetChoices {
    val choices = value.getJSONArray("choices")
    return ComposerTargetChoices(value.optional("runtime"), value.optional("owner"), List(choices.length()) { index ->
        choices.getJSONObject(index).let { ComposerRuntimeChoice(it.getString("owner"), it.getString("runtime"), TurnChoice(it.optJSONObject("model")?.let(::decodeModel), it.optional("effort"), it.optional("permission"))) }
    }, value.getBoolean("edited"))
}
