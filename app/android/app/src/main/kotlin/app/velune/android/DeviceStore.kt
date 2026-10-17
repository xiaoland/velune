package app.velune.android

import android.util.AtomicFile
import app.velune.bindings.BindingRemoteEndpointDescriptor
import app.velune.bindings.BindingRemoteRequestId
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

data class PendingOperation(
    val endpointId: String,
    val requestId: BindingRemoteRequestId,
    val label: String,
    val runtimeId: String?,
    val conversationId: String?,
)

data class SavedDevices(
    val devices: List<BindingRemoteEndpointDescriptor> = emptyList(),
    val pending: PendingOperation? = null,
)

/** Only dial information and request identity are saved; invitations and message bodies stay in memory. */
class DeviceStore(directory: File) {
    private val file = AtomicFile(File(directory, "devices.json"))

    fun load(): SavedDevices {
        if (!file.baseFile.exists()) return SavedDevices()
        val json = JSONObject(file.openRead().bufferedReader().use { it.readText() })
        if (json.optInt("schema") != 1) return SavedDevices()
        val devices = json.getJSONArray("devices").let { array ->
            List(array.length()) { index ->
                array.getJSONObject(index).let { value ->
                    BindingRemoteEndpointDescriptor(
                        version = value.getInt("version").toUShort(),
                        endpointId = value.getString("endpointId"),
                        name = value.getString("name"),
                        incarnation = value.getString("incarnation"),
                        relayUrls = value.getJSONArray("relayUrls").strings(),
                        directAddresses = value.getJSONArray("directAddresses").strings(),
                    )
                }
            }
        }
        val pending = json.optJSONObject("pending")?.let { value ->
            PendingOperation(
                endpointId = value.getString("endpointId"),
                requestId = BindingRemoteRequestId(
                    incarnation = value.getString("incarnation"),
                    client = value.getString("client"),
                    sequence = value.getString("sequence").toULong(),
                ),
                label = value.getString("label"),
                runtimeId = value.optString("runtimeId").takeIf { it.isNotEmpty() },
                conversationId = value.optString("conversationId").takeIf { it.isNotEmpty() },
            )
        }
        return SavedDevices(devices, pending)
    }

    fun save(value: SavedDevices) {
        val json = JSONObject().put("schema", 1).put("devices", JSONArray().apply {
            value.devices.forEach { descriptor ->
                put(JSONObject().apply {
                    put("version", descriptor.version.toInt())
                    put("endpointId", descriptor.endpointId)
                    put("name", descriptor.name)
                    put("incarnation", descriptor.incarnation)
                    put("relayUrls", JSONArray(descriptor.relayUrls))
                    put("directAddresses", JSONArray(descriptor.directAddresses))
                })
            }
        })
        value.pending?.let { pending ->
            json.put("pending", JSONObject().apply {
                put("endpointId", pending.endpointId)
                put("incarnation", pending.requestId.incarnation)
                put("client", pending.requestId.client)
                put("sequence", pending.requestId.sequence.toString())
                put("label", pending.label)
                put("runtimeId", pending.runtimeId ?: "")
                put("conversationId", pending.conversationId ?: "")
            })
        }
        val stream = file.startWrite()
        try {
            stream.write(json.toString().toByteArray(Charsets.UTF_8))
            file.finishWrite(stream)
        } catch (error: Exception) {
            file.failWrite(stream)
            throw error
        }
    }
}

private fun JSONArray.strings(): List<String> = List(length()) { getString(it) }
