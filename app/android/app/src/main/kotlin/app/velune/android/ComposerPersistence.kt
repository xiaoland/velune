package app.velune.android

import app.velune.bindings.application.ComposerAttachment
import app.velune.bindings.application.ComposerInput
import app.velune.bindings.application.ComposerInputContent
import app.velune.bindings.application.ComposerRuntimeChoice
import app.velune.bindings.application.TurnChoice
import app.velune.bindings.conversation.ModelSelection
import org.json.JSONArray
import org.json.JSONObject

internal data class PendingComposerCreation(val inputId: String, val ownerId: String, val runtimeId: String, val conversationId: String)

/** Local cache carries an explicit pending baseline; a clean cache never writes back. */
internal data class SavedComposerInput(val input: ComposerInput, val pending: Boolean, val baseline: ULong?, val unbound: Boolean, val directoryRevision: ULong?, val creation: PendingComposerCreation?, val bindingRevision: ULong?, val bindingBaseline: ULong?) {
    fun encode(): String {
        val content = input.content
        fun model(value: ModelSelection?): JSONObject? = when (value) {
            null -> null
            is ModelSelection.Managed -> JSONObject().put("managed", value.modelRecordKey)
            is ModelSelection.Native -> JSONObject().put("native", value.modelId).put("provider", value.providerId)
        }
        return JSONObject().put("version", 1).put("id", input.id).put("revision", input.revision.toString())
            .put("unbound", unbound).put("bindingRevision", bindingRevision?.toString()).put("bindingBaseline", bindingBaseline?.toString()).put("creation", creation?.let { JSONObject().put("input", it.inputId).put("owner", it.ownerId).put("runtime", it.runtimeId).put("conversation", it.conversationId) }).put("directoryRevision", directoryRevision?.toString()).put("pending", pending).put("baseline", baseline?.toString()).put("text", content.text)
            .put("runtime", content.runtimeId).put("runtimeOwner", content.runtimeOwnerId).put("edited", content.choiceEdited)
            .put("attachments", JSONArray(content.attachments.map { JSONObject().put("id", it.id).put("name", it.name).put("bytes", it.bytes.toString()) }))
            .put("choices", JSONArray(content.choices.map { JSONObject().put("owner", it.ownerId).put("runtime", it.runtimeId).put("model", model(it.choice.model)).put("effort", it.choice.effort).put("permission", it.choice.permissionMode) })).toString()
    }
    companion object {
        fun decode(value: String): SavedComposerInput {
            val json = JSONObject(value)
            require(json.getInt("version") == 1) { "不支持的本机输入格式，原数据已保留" }
            fun JSONObject.optional(key: String): String? = if (isNull(key) || !has(key)) null else getString(key)
            val files = json.getJSONArray("attachments")
            val choices = json.getJSONArray("choices")
            return SavedComposerInput(ComposerInput(json.getString("id"), json.getString("revision").toULong(), ComposerInputContent(
                json.getString("text"), List(files.length()) { index -> files.getJSONObject(index).let { ComposerAttachment(it.getString("id"), it.getString("name"), it.getString("bytes").toULong()) } },
                json.optional("runtime"), json.optional("runtimeOwner"), List(choices.length()) { index -> choices.getJSONObject(index).let {
                    val model = it.optJSONObject("model")?.let { model -> model.optional("managed")?.let(ModelSelection::Managed) ?: ModelSelection.Native(providerId = model.optional("provider"), modelId = model.getString("native")) }
                    ComposerRuntimeChoice(it.getString("owner"), it.getString("runtime"), TurnChoice(model, it.optional("effort"), it.optional("permission")))
                } }, json.getBoolean("edited"))), json.getBoolean("pending"), json.optional("baseline")?.toULong(), json.getBoolean("unbound"), json.optional("directoryRevision")?.toULong(), json.optJSONObject("creation")?.let { PendingComposerCreation(it.getString("input"), it.getString("owner"), it.getString("runtime"), it.getString("conversation")) }, json.optional("bindingRevision")?.toULong(), json.optional("bindingBaseline")?.toULong())
        }
    }
}
