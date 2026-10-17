plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "app.velune.android"
    compileSdk = 36
    defaultConfig {
        applicationId = "app.velune.android"
        minSdk = 28
        targetSdk = 36
        versionCode = 1
        versionName = "0.1 beta.1"
        ndk { abiFilters += "arm64-v8a" }
    }
    buildFeatures { compose = true }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    sourceSets["main"].apply {
        java.srcDir("../../../target/android-bindings")
        jniLibs.srcDir("../../../target/android-native")
    }
    packaging { jniLibs.useLegacyPackaging = false }
}

dependencies {
    implementation(platform("androidx.compose:compose-bom:2025.10.01"))
    implementation("androidx.activity:activity-compose:1.11.0")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-core")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.4")
    implementation("androidx.lifecycle:lifecycle-viewmodel-ktx:2.9.4")
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    implementation(files(rootProject.file("../../target/android-native/rustls-platform-verifier-0.2.0.aar")))
}
