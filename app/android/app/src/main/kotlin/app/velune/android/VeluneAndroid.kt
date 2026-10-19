package app.velune.android

import android.app.Application
import android.content.Context

object NativePlatform {
    init { System.loadLibrary("velune_bindings") }
    external fun initialize(context: Context)
}

class VeluneAndroid : Application() {
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
