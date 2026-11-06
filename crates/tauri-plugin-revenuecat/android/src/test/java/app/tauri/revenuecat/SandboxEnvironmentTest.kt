package app.tauri.revenuecat

import kotlin.test.Test
import kotlin.test.assertNull

class SandboxEnvironmentTest {
    @Test
    fun `android reports an explicitly unknown sandbox state`() {
        assertNull(androidSandboxEnvironment())
    }
}
