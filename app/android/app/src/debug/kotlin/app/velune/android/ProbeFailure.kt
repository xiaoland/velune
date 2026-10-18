package app.velune.android

import app.velune.bindings.BindingException
import app.velune.bindings.BindingFailureKind
import app.velune.bindings.InternalException
import org.json.JSONException
import org.json.JSONObject
import java.io.IOException
import java.net.SocketTimeoutException
import java.net.URISyntaxException
import java.nio.BufferUnderflowException
import java.util.concurrent.TimeoutException

internal enum class ProbeStage(val code: String) {
    PLATFORM_INITIALIZATION("platform_initialization"),
    EVIDENCE_OPEN("evidence_open"),
    TARGET_READ("target_read"),
    TARGET_PARSE("target_parse"),
    ENDPOINT_OPEN("endpoint_open"),
    NETWORK_MONITOR("network_monitor"),
    MDNS_ENABLE("mdns_enable"),
    CONNECT("connect"),
    MDNS_RESOLVE("mdns_resolve"),
    MULTICAST_LOCK("multicast_lock"),
}

internal enum class ProbeErrorKind {
    BINDING_DIAGNOSTIC, BINDING_INVALID, BINDING_UNSUPPORTED, BINDING_IO,
    BINDING_CONTRACT, BINDING_REMOTE_REQUEST, BINDING_CLOSED, BINDING_UNAVAILABLE,
    BINDING_LINKAGE, FFI_INTERNAL, BUFFER_UNDERFLOW, PERMISSION_DENIED, TIMEOUT,
    JSON_PARSE, INVALID_ARGUMENT, IO, UNKNOWN,
}

/** Only bounded types cross into evidence. Exception text, causes, and stacks are never inspected. */
internal data class ProbeFailure(val kind: ProbeErrorKind, val bindingKind: BindingFailureKind? = null) {
    fun evidence(stage: ProbeStage): JSONObject = JSONObject()
        .put("stage", stage.code).put("errorKind", kind.name.lowercase())
        .also { fields -> bindingKind?.let { fields.put("bindingFailureKind", it.name) } }

    fun message(stage: ProbeStage, mdns: Boolean): String {
        if (kind == ProbeErrorKind.BINDING_LINKAGE) {
            return "远控绑定装配异常，请重新安装配套 APK。"
        }
        if (kind == ProbeErrorKind.FFI_INTERNAL || kind == ProbeErrorKind.BUFFER_UNDERFLOW) {
            return "核心接口调用失败，请检查配套原生构建。"
        }
        if (kind == ProbeErrorKind.PERMISSION_DENIED) return "操作被权限限制，请检查应用权限和系统限制。"
        return when (stage) {
            ProbeStage.PLATFORM_INITIALIZATION -> "远控平台初始化失败，请重新安装配套 APK。"
            ProbeStage.EVIDENCE_OPEN -> "无法打开私有诊断文件，请检查应用存储。"
            ProbeStage.TARGET_READ -> "无法读取合成目标，请检查 target.json。"
            ProbeStage.TARGET_PARSE -> "合成目标格式无效，请更新 target.json。"
            ProbeStage.ENDPOINT_OPEN -> "传输端点未能打开，请检查原生构建和网络。"
            ProbeStage.NETWORK_MONITOR -> "网络观察未能启动，请检查应用权限和系统限制。"
            ProbeStage.MDNS_ENABLE -> "mDNS 未能启用，请检查多播环境和原生构建。"
            ProbeStage.CONNECT -> if (kind == ProbeErrorKind.TIMEOUT) {
                "连接超时，请检查宿主在线和网络可达性。"
            } else if (mdns) {
                "mDNS 已启用，但连接未建立；请检查宿主在线和同网可达性。"
            } else {
                "连接未建立，请检查宿主在线和网络可达性。"
            }
            ProbeStage.MDNS_RESOLVE -> "mDNS 地址查询未完成，请检查同网发现环境。"
            ProbeStage.MULTICAST_LOCK -> "多播锁不可用，请检查权限和 Wi-Fi 环境。"
        }
    }

    companion object {
        fun from(error: Throwable): ProbeFailure = when (error) {
            is BindingException -> when (error) {
                is BindingException.Diagnostic -> ProbeFailure(ProbeErrorKind.BINDING_DIAGNOSTIC, error.kind)
                is BindingException.Invalid -> ProbeFailure(ProbeErrorKind.BINDING_INVALID)
                is BindingException.Unsupported -> ProbeFailure(ProbeErrorKind.BINDING_UNSUPPORTED)
                is BindingException.Io -> ProbeFailure(ProbeErrorKind.BINDING_IO)
                is BindingException.Contract -> ProbeFailure(ProbeErrorKind.BINDING_CONTRACT)
                is BindingException.RemoteRequest -> ProbeFailure(ProbeErrorKind.BINDING_REMOTE_REQUEST, error.kind)
                is BindingException.Closed -> ProbeFailure(ProbeErrorKind.BINDING_CLOSED)
                is BindingException.Unavailable -> ProbeFailure(ProbeErrorKind.BINDING_UNAVAILABLE)
            }
            is LinkageError -> ProbeFailure(ProbeErrorKind.BINDING_LINKAGE)
            is InternalException -> ProbeFailure(ProbeErrorKind.FFI_INTERNAL)
            is BufferUnderflowException -> ProbeFailure(ProbeErrorKind.BUFFER_UNDERFLOW)
            is SecurityException -> ProbeFailure(ProbeErrorKind.PERMISSION_DENIED)
            is SocketTimeoutException, is TimeoutException -> ProbeFailure(ProbeErrorKind.TIMEOUT)
            is JSONException -> ProbeFailure(ProbeErrorKind.JSON_PARSE)
            is IllegalArgumentException, is URISyntaxException -> ProbeFailure(ProbeErrorKind.INVALID_ARGUMENT)
            is IOException -> ProbeFailure(ProbeErrorKind.IO)
            else -> ProbeFailure(ProbeErrorKind.UNKNOWN)
        }
    }
}
