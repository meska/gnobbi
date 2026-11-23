// Shell only: NativeActivity loads libgnobbi.so (no JVM code). The .so is the cargo-ndk output in artifacts/android/jniLibs/<abi>/ (see build-apk.sh).
plugins {
    id("com.android.application")
}

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
    sourceSets["main"].jniLibs.srcDir(nativeLibs)
}

// An APK without the .so installs fine and then dies at launch: fail the build instead.
tasks.matching { it.name == "preBuild" }.configureEach {
    doFirst { check(nativeLibs.resolve("arm64-v8a/libgnobbi.so").isFile) { "missing $nativeLibs/arm64-v8a/libgnobbi.so: build it with cargo-ndk first (see packaging/android/build-apk.sh)" } }
}
