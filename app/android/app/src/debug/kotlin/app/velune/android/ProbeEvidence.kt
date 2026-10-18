package app.velune.android

import android.os.SystemClock
import android.system.Os
import android.system.OsConstants
import org.json.JSONObject
import java.io.File
import java.io.FileOutputStream
import java.util.UUID

/** S1-only evidence is private, excludes payloads, and never reaches Logcat. */
internal class ProbeEvidence(filesDirectory: File) {
    val directory = File(filesDirectory, "s1-probe")
    private val events: File
    private var sequence = 0L
    private val runId = UUID.randomUUID().toString()

    init {
        check(directory.isDirectory || directory.mkdirs()) { "probe_directory_unavailable" }
        Os.chmod(directory.absolutePath, OsConstants.S_IRWXU)
        events = File(directory, "events.jsonl")
        if (!events.exists()) check(events.createNewFile()) { "probe_events_unavailable" }
        Os.chmod(events.absolutePath, OsConstants.S_IRUSR or OsConstants.S_IWUSR)
    }

    fun readTarget(): JSONObject {
        val target = File(directory, "target.json")
        check(target.isFile && target.length() in 1..65_536) { "probe_target_unavailable" }
        return JSONObject(target.readText(Charsets.UTF_8))
    }

    @Synchronized
    fun record(kind: String, fields: JSONObject = JSONObject(), observedAtMs: Long = SystemClock.elapsedRealtime()) {
        val entry = JSONObject()
            .put("event", kind)
            .put("runId", runId)
            .put("eventSequence", ++sequence)
            .put("elapsedRealtimeMs", observedAtMs)
            .put("writtenAtElapsedRealtimeMs", SystemClock.elapsedRealtime())
            .put("fields", fields)
        FileOutputStream(events, true).use { output ->
            output.write((entry.toString() + "\n").toByteArray(Charsets.UTF_8))
            output.fd.sync()
        }
    }
}
