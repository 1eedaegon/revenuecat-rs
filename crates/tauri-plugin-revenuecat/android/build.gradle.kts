plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "app.tauri.revenuecat"
    compileSdk = 34

    defaultConfig {
        minSdk = 24
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    // Tauri v2 Android plugin runtime. The Tauri CLI injects this module into the
    // consuming app's Gradle build; adjust the path if the module name differs.
    implementation(project(":tauri-android"))

    implementation("com.android.billingclient:billing-ktx:8.0.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.1")
}
