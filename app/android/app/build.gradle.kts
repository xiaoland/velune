plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

val python = if (System.getProperty("os.name").startsWith("Windows", ignoreCase = true)) "python" else "python3"
val buildNumber = providers.environmentVariable("VELUNE_BUILD_NUMBER").orElse("1").get()
val versionInfo = providers.exec {
    commandLine(
        python,
        rootProject.file("../../scripts/version.py").absolutePath,
        "--format",
        "build-info",
        "--build-number",
        buildNumber,
    )
}.standardOutput.asText.get().trim().split('\t')
check(versionInfo.size == 3) { "scripts/version.py must return SemVer, numeric core, and build number" }
val veluneVersion = versionInfo[0]
val veluneBuildNumber = versionInfo[2].toInt()
dependencyLocking { lockAllConfigurations() }
val releaseKeystorePath = providers.environmentVariable("ANDROID_RELEASE_KEYSTORE_PATH").orNull
val releaseStorePassword = providers.environmentVariable("ANDROID_RELEASE_STORE_PASSWORD").orNull
val releaseKeyAlias = providers.environmentVariable("ANDROID_RELEASE_KEY_ALIAS").orNull
val releaseKeyPassword = providers.environmentVariable("ANDROID_RELEASE_KEY_PASSWORD").orNull
val releaseSigningValues = listOf(releaseKeystorePath, releaseStorePassword, releaseKeyAlias, releaseKeyPassword)
val hasReleaseSigning = releaseSigningValues.all { !it.isNullOrBlank() }
check(releaseSigningValues.all { it.isNullOrBlank() } || hasReleaseSigning) {
    "Android release signing requires the keystore path, store password, key alias, and key password together"
}
if (providers.environmentVariable("VELUNE_REQUIRE_ANDROID_RELEASE_SIGNING").orNull == "true") {
    check(hasReleaseSigning) { "Android release signing material is required for this release build" }
}

android {
    namespace = "app.velune.android"
    compileSdk = 36
    ndkVersion = "30.0.14904198"
    defaultConfig {
        applicationId = providers.gradleProperty("veluneApplicationId").getOrElse("app.velune.android")
        minSdk = 28
        targetSdk = 36
        versionCode = veluneBuildNumber
        versionName = veluneVersion
        ndk { abiFilters += "arm64-v8a" }
    }
    if (hasReleaseSigning) {
        signingConfigs.create("veluneRelease") {
            storeFile = file(requireNotNull(releaseKeystorePath))
            storeType = "PKCS12"
            storePassword = requireNotNull(releaseStorePassword)
            keyAlias = requireNotNull(releaseKeyAlias)
            keyPassword = requireNotNull(releaseKeyPassword)
        }
    }
    buildTypes.getByName("release") {
        if (hasReleaseSigning) signingConfig = signingConfigs.getByName("veluneRelease")
    }
    buildFeatures { compose = true }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    sourceSets["main"].apply {
        java.srcDir("../../../target/android-bindings")
        jniLibs.srcDir("../../../target/android-native")
        // NOTICE.md, the licence texts it cites, and the generated Rust crate notices, shown in “关于与开源许可”.
        assets.srcDir("../licenses")
        assets.srcDir("../../../target/android-licenses")
    }
    packaging {
        jniLibs.useLegacyPackaging = false
        // Android does not load the Java 9 OSGi bundle descriptors in these SDK jars.
        resources.excludes += "META-INF/versions/9/OSGI-INF/MANIFEST.MF"
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
        allWarningsAsErrors.set(true)
    }
}

dependencies {
    implementation(platform("io.opentelemetry:opentelemetry-bom:1.62.0"))
    implementation("io.opentelemetry:opentelemetry-sdk")
    implementation("io.opentelemetry:opentelemetry-exporter-otlp")
    implementation("io.opentelemetry:opentelemetry-extension-kotlin")
    implementation(platform("androidx.compose:compose-bom:2025.10.01"))
    implementation("androidx.activity:activity-compose:1.11.0")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.4")
    implementation("androidx.lifecycle:lifecycle-viewmodel-ktx:2.9.4")
    implementation("androidx.camera:camera-camera2:1.4.2")
    implementation("androidx.camera:camera-lifecycle:1.4.2")
    implementation("androidx.camera:camera-view:1.4.2")
    implementation("com.google.zxing:core:3.5.3")
    implementation("org.commonmark:commonmark:0.24.0")
    implementation("org.commonmark:commonmark-ext-gfm-tables:0.24.0")
    implementation("org.commonmark:commonmark-ext-gfm-strikethrough:0.24.0")
    implementation("org.commonmark:commonmark-ext-autolink:0.24.0")
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    implementation(files(rootProject.file("../../target/android-native/rustls-platform-verifier-0.2.0.aar")))
}
