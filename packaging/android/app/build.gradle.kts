// Shell: MainActivity (NativeActivity) loads libgnobbi.so; a few Java classes handle FCM and notifications. The .so is the cargo-ndk output in artifacts/android/jniLibs/<abi>/ (see build-apk.sh).
plugins {
    id("com.android.application")
}

// google-services.json is not in the repo (PA-1): without it the build still compiles, Firebase just has no config.
if (file("google-services.json").exists()) apply(plugin = "com.google.gms.google-services")

val nativeLibs = rootDir.resolve("../../artifacts/android/jniLibs")

android {
    namespace = "net.meskatech.gnobbi"
    compileSdk = 35
    defaultConfig {
        applicationId = "net.meskatech.gnobbi"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1"
        ndk { abiFilters += listOf("arm64-v8a") }
    }
    testOptions.unitTests.isIncludeAndroidResources = true
    sourceSets["main"].jniLibs.srcDir(nativeLibs)
}

// An APK without the .so installs fine and then dies at launch: fail the build instead.
tasks.matching { it.name == "preBuild" }.configureEach {
    doFirst { check(nativeLibs.resolve("arm64-v8a/libgnobbi.so").isFile) { "missing $nativeLibs/arm64-v8a/libgnobbi.so: build it with cargo-ndk first (see packaging/android/build-apk.sh)" } }
}

dependencies {
    implementation(platform("com.google.firebase:firebase-bom:34.0.0"))
    implementation("com.google.firebase:firebase-messaging")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.robolectric:robolectric:4.14.1")
    testImplementation("androidx.test:core:1.6.1")
}
