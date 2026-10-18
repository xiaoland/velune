package app.velune.android

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import app.velune.bindings.BindingNetworkChangeReason
import app.velune.bindings.BindingPlatformNetwork
import app.velune.bindings.BindingPlatformNetworkSnapshot
import java.net.InetAddress

/** Reports platform facts; observing a network does not establish a usable transport path. */
class NetworkMonitor(
    context: Context,
    private val report: (BindingPlatformNetworkSnapshot) -> Unit,
    private val observed: ((BindingPlatformNetworkSnapshot) -> Unit)? = null,
) : AutoCloseable {
    private val manager = context.getSystemService(ConnectivityManager::class.java)
    private val handler = Handler(Looper.getMainLooper())
    private val networks = mutableMapOf<Network, Facts>()
    private var defaultNetwork: Network? = null
    private var started = false
    private var sequence = 0uL
    private var pending: BindingPlatformNetworkSnapshot? = null
    private var flushScheduled = false
    private val flush = Runnable {
        val snapshot = pending
        pending = null
        flushScheduled = false
        snapshot?.let(report)
    }
    private val defaultCallback = callback(default = true)
    private val lanCallback = callback(default = false)

    private data class Facts(
        var defaultObserved: Boolean = false,
        var lanObserved: Boolean = false,
        var wifi: Boolean = false,
        var cellular: Boolean = false,
        var ethernet: Boolean = false,
        var vpn: Boolean = false,
        var internet: Boolean = false,
        var validated: Boolean = false,
        var interfaceName: String? = null,
        var addresses: List<String> = emptyList(),
        var dnsServers: List<String> = emptyList(),
    )

    fun start() {
        requireMainThread()
        if (started) return
        started = true
        try {
            manager.registerDefaultNetworkCallback(defaultCallback, handler)
            // Transports are alternatives. Observe LAN without requiring Internet or validation,
            // including a non-default Ethernet underneath the current VPN/default network.
            val request = NetworkRequest.Builder()
                .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
                .addTransportType(NetworkCapabilities.TRANSPORT_ETHERNET)
                .build()
            manager.registerNetworkCallback(request, lanCallback, handler)
        } catch (error: Exception) {
            close()
            throw error
        }
    }

    fun foreground() {
        requireMainThread()
        if (started) changed(BindingNetworkChangeReason.FOREGROUND)
    }

    private fun callback(default: Boolean) = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            if (!started) return
            val facts = networks.getOrPut(network) { Facts() }
            if (default) {
                defaultNetwork?.takeIf { it != network }?.let { previous ->
                    networks[previous]?.let { old ->
                        old.defaultObserved = false
                        if (!old.lanObserved) networks.remove(previous)
                    }
                }
                defaultNetwork = network
                facts.defaultObserved = true
            } else {
                facts.lanObserved = true
            }
            changed(BindingNetworkChangeReason.AVAILABLE)
        }

        override fun onLost(network: Network) {
            if (!started) return
            networks[network]?.let { facts ->
                if (default) {
                    facts.defaultObserved = false
                    if (defaultNetwork == network) defaultNetwork = null
                } else {
                    facts.lanObserved = false
                }
                if (!facts.defaultObserved && !facts.lanObserved) networks.remove(network)
            }
            changed(BindingNetworkChangeReason.LOST)
        }

        override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) {
            if (!started) return
            val facts = networks[network] ?: return
            facts.wifi = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)
            facts.cellular = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)
            facts.ethernet = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
            facts.vpn = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
            facts.internet = capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            facts.validated = capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
            changed(BindingNetworkChangeReason.CAPABILITIES)
        }

        override fun onLinkPropertiesChanged(network: Network, properties: LinkProperties) {
            if (!started) return
            val facts = networks[network] ?: return
            facts.interfaceName = properties.interfaceName?.takeIf { it.isNotBlank() }
            facts.addresses = properties.linkAddresses.map { address ->
                "${address.address.observedIp()}/${address.prefixLength}"
            }.sorted()
            facts.dnsServers = properties.dnsServers.map { it.observedIp() }.sorted()
            changed(BindingNetworkChangeReason.LINK_PROPERTIES)
        }
    }

    private fun changed(reason: BindingNetworkChangeReason) {
        sequence += 1uL
        val snapshot = BindingPlatformNetworkSnapshot(
            sequence = sequence,
            elapsedRealtimeMs = SystemClock.elapsedRealtime().toULong(),
            reason = reason,
            defaultNetworkId = defaultNetwork?.identifier(),
            networks = networks.entries.sortedBy { it.key.identifier() }.map { (network, facts) ->
                BindingPlatformNetwork(
                    id = network.identifier(),
                    wifi = facts.wifi,
                    cellular = facts.cellular,
                    ethernet = facts.ethernet,
                    vpn = facts.vpn,
                    internet = facts.internet,
                    validated = facts.validated,
                    interfaceName = facts.interfaceName,
                    addresses = facts.addresses,
                    dnsServers = facts.dnsServers,
                )
            },
        )
        // Private probe evidence retains the original callback time before notification coalescing.
        observed?.invoke(snapshot)
        if (!started) return
        pending = snapshot
        if (!flushScheduled) {
            flushScheduled = true
            handler.postDelayed(flush, 100)
        }
    }

    override fun close() {
        requireMainThread()
        if (!started) return
        started = false
        handler.removeCallbacks(flush)
        pending = null
        flushScheduled = false
        try { manager.unregisterNetworkCallback(defaultCallback) } catch (_: IllegalArgumentException) { }
        try { manager.unregisterNetworkCallback(lanCallback) } catch (_: IllegalArgumentException) { }
        networks.clear()
        defaultNetwork = null
    }

    private fun requireMainThread() {
        check(Looper.myLooper() == Looper.getMainLooper()) { "网络观察必须由主线程管理。" }
    }
}

private fun Network.identifier(): String = networkHandle.toULong().toString()

// Java's hostAddress contract is non-null. Scope belongs to the accompanying Network/interface;
// these numeric facts observe interfaces and must never be reused as socket dial addresses.
private fun InetAddress.observedIp(): String = checkNotNull(hostAddress) { "网络接口地址缺失。" }.substringBefore('%')
