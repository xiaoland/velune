package app.velune.android

import android.content.ContentResolver
import android.net.Uri
import android.provider.OpenableColumns
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.AttachFile
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import java.io.ByteArrayOutputStream
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

private const val MAX_ATTACHMENT_BYTES = 20 * 1024 * 1024

@Composable
internal fun ComposerAttachmentButton(
    enabled: Boolean,
    uploading: Boolean,
    addAttachments: (List<suspend () -> Pair<String, ByteArray>>) -> Unit,
) {
    val resolver = LocalContext.current.contentResolver
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        addAttachments(uris.map { uri -> suspend { readAttachment(resolver, uri) } })
    }
    IconButton(
        enabled = enabled && !uploading,
        onClick = { picker.launch(arrayOf("*/*")) },
        modifier = Modifier.size(40.dp),
    ) {
        if (uploading) {
            CircularProgressIndicator(
                Modifier.size(18.dp).semantics { contentDescription = "正在添加附件" },
                strokeWidth = 2.dp,
            )
        } else {
            Icon(Icons.Outlined.AttachFile, contentDescription = "添加附件")
        }
    }
}

private suspend fun readAttachment(resolver: ContentResolver, uri: Uri): Pair<String, ByteArray> =
    withContext(Dispatchers.IO) {
        var name = "未命名文件"
        try {
            val size = resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { row ->
                if (!row.moveToFirst()) return@use null
                val nameIndex = row.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                if (nameIndex >= 0) row.getString(nameIndex)?.takeIf { it.isNotBlank() }?.let { name = it }
                val sizeIndex = row.getColumnIndex(OpenableColumns.SIZE)
                if (sizeIndex >= 0 && !row.isNull(sizeIndex)) row.getLong(sizeIndex) else null
            }
            if (size != null && size > MAX_ATTACHMENT_BYTES) throw AttachmentTooLarge()
            val bytes = checkNotNull(resolver.openInputStream(uri)).use { input ->
                val output = ByteArrayOutputStream()
                val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
                while (true) {
                    ensureActive()
                    val count = input.read(buffer)
                    if (count < 0) break
                    if (output.size() + count > MAX_ATTACHMENT_BYTES) throw AttachmentTooLarge()
                    output.write(buffer, 0, count)
                }
                output.toByteArray()
            }
            name to bytes
        } catch (error: CancellationException) {
            throw error
        } catch (_: AttachmentTooLarge) {
            error("“$name”超过 20 MB，请选择较小的文件")
        } catch (_: Exception) {
            error("无法读取“$name”，请重新选择文件")
        }
    }

private class AttachmentTooLarge : Exception()
