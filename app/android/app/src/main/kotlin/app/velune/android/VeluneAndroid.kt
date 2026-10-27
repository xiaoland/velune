package app.velune.android

import android.app.Application
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import java.io.File

object NativePlatform {
    init { System.loadLibrary("velune_bindings") }
    external fun initialize(context: Context)
}

class VeluneAndroid : Application() {
    private val owner = lazy { DeviceHub(this).also { it.start() } }
    val hub: DeviceHub get() = owner.value
    var telemetryProfile: String? = null
        private set
    var telemetryRunId: String? = null
        private set
    fun configureDevelopment(intent: Intent) {
        if (owner.isInitialized() || applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE == 0) return
        val profile = intent.getStringExtra("velune.telemetry.profile") ?: return
        val file = File(profile).canonicalFile
        val root = File(noBackupFilesDir, "development-telemetry").canonicalFile
        if (!file.path.startsWith(root.path + File.separator) || !file.isFile) return
        telemetryProfile = file.path
        telemetryRunId = intent.getStringExtra("velune.telemetry.run")
    }
    var initializationFailure: String? = null
        private set

    override fun onCreate() {
        super.onCreate()
        try {
            NativePlatform.initialize(applicationContext)
        } catch (_: LinkageError) {
            initializationFailure = "无法加载远控核心，请重新安装完整 APK。"
        } catch (_: Exception) {
            initializationFailure = "无法初始化远控核心。"
        }
    }
}
