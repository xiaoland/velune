package app.velune.android

import android.content.Context
import android.net.wifi.WifiManager

/** One foreground diagnostic owner; releasing this lock does not affect other applications' locks. */
internal class ProbeMulticastLock(context: Context) {
    private val lock = checkNotNull(context.applicationContext.getSystemService(WifiManager::class.java))
        .createMulticastLock("VeluneS1Probe")
        .also { it.setReferenceCounted(false) }

    fun setHeld(held: Boolean): Boolean {
        if (held && !lock.isHeld) lock.acquire()
        if (!held && lock.isHeld) lock.release()
        return lock.isHeld
    }
}
