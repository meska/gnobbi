// Shell: MainActivity (NativeActivity) loads libgnobbi.so; a few Java classes handle FCM and notifications. The .so is the cargo-ndk output in artifacts/android/jniLibs/<abi>/ (see build-apk.sh).
plugins {
    id("com.android.application")
}

// google-services.json is not in the repo (PA-1): without it the build still compiles, Firebase just has no config.
if (file("google-services.json").exists()) apply(plugin = "com.google.gms.google-services")

val nativeLibs = rootDir.resolve("../../artifacts/android/jniLibs")

android {
    namespace = "net.meskatech.gnobbi"
    compileSdk = 36
    defaultConfig {
        applicationId = "net.meskatech.gnobbi"
        minSdk = 26
        targetSdk = 36
        // tools/android-build.sh passes the real values: code = git rev-list --count (same build number as iOS), name = workspace version.
        versionCode = (findProperty("gnobbiVersionCode") as String?)?.toInt() ?: 1
        versionName = (findProperty("gnobbiVersionName") as String?) ?: "0.2"
        ndk { abiFilters += listOf("arm64-v8a") }
    }
    // Same application id (and signing key) for both: `play` goes to the Play Store and has no updater; `direct` is the GitHub APK,
    // which updates itself (src/direct adds REQUEST_INSTALL_PACKAGES, the only difference in the manifest).
    flavorDimensions += "dist"
    productFlavors {
        create("play") {
            dimension = "dist"
            buildConfigField("boolean", "SELF_UPDATE", "false")
        }
        create("direct") {
            dimension = "dist"
            buildConfigField("boolean", "SELF_UPDATE", "true")
        }
    }
    buildFeatures.buildConfig = true
    // Optional: sign bundlePlayRelease / assembleDirectRelease here when the upload key is given by env (path, password file, alias), never from the repo.
    // The standard flow leaves the AAB unsigned and tools/android-build.sh signs it on the Mac, so the key never reaches the CT.
    System.getenv("GNOBBI_UPLOAD_STORE")?.let { store ->
        signingConfigs.create("upload") {
            storeFile = file(store)
            storePassword = file(System.getenv("GNOBBI_UPLOAD_PASS_FILE")).readText().trim()
            keyAlias = System.getenv("GNOBBI_UPLOAD_ALIAS") ?: "gnobbi-upload"
        }
        buildTypes.getByName("release").signingConfig = signingConfigs["upload"]
    }
    testOptions.unitTests.isIncludeAndroidResources = true
    sourceSets["main"].jniLibs.srcDir(nativeLibs)
}

// An APK without the .so installs fine and then dies at launch: fail the build instead.
tasks.matching { it.name == "preBuild" }.configureEach {
    doFirst { check(nativeLibs.resolve("arm64-v8a/libgnobbi.so").isFile) { "missing $nativeLibs/arm64-v8a/libgnobbi.so: build it with cargo-ndk first (see packaging/android/build-apk.sh)" } }
}

dependencies {
    implementation(platform("com.google.firebase:firebase-bom:35.2.0"))
    implementation("com.google.firebase:firebase-messaging")
    implementation("androidx.core:core:1.13.1") // FileProvider for «Share»
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.robolectric:robolectric:4.16")
    testImplementation("androidx.test:core:1.6.1")
}
