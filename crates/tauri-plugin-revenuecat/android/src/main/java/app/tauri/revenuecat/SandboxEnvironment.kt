package app.tauri.revenuecat

import app.tauri.plugin.JSObject
import org.json.JSONObject

internal fun androidSandboxEnvironment(): Boolean? = null

internal fun unknownSandboxEnvironmentResponse(): JSObject =
    JSObject().apply {
        // Play Billing does not expose a reliable client-side sandbox signal.
        put("isSandbox", androidSandboxEnvironment() ?: JSONObject.NULL)
    }
