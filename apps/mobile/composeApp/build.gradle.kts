import gobley.gradle.Variant
import gobley.gradle.cargo.dsl.appleMobile
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    alias(libs.plugins.kotlin.multiplatform)
    alias(libs.plugins.android.application)
    alias(libs.plugins.compose.multiplatform)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.atomicfu)
    alias(libs.plugins.gobley.cargo)
    alias(libs.plugins.gobley.uniffi)
}

// The shared Rust client lives with the other crates; Gobley builds it for each target and
// generates the Kotlin bindings from it.
cargo {
    packageDirectory = layout.projectDirectory.dir("../../../crates/mothr-client")
    builds.appleMobile {
        variants {
            buildTaskProvider.configure {
                additionalEnvironment.put("IPHONEOS_DEPLOYMENT_TARGET", "16.0")
            }
        }
    }
}

uniffi {
    generateFromLibrary {
        namespace = "mothr_client"
        packageName = "dev.mothr.client"
        variant = Variant.Debug
    }
}

kotlin {
    androidTarget {
        compilerOptions { jvmTarget.set(JvmTarget.JVM_17) }
    }
    listOf(iosArm64(), iosSimulatorArm64()).forEach { target ->
        target.binaries.framework {
            baseName = "ComposeApp"
            isStatic = true
        }
    }

    sourceSets {
        commonMain.dependencies {
            implementation(compose.runtime)
            implementation(compose.foundation)
            implementation(compose.material3)
            implementation(compose.ui)
            implementation(compose.components.resources)
            implementation(libs.compose.backhandler)
            implementation(libs.kotlinx.coroutines.core)
            implementation(libs.markdown.m3)
            implementation(libs.settings)
        }
        androidMain.dependencies {
            implementation(libs.androidx.activity.compose)
            implementation(libs.androidx.core)
            implementation(libs.kotlinx.coroutines.android)
            implementation(libs.play.services.code.scanner)
            // The scanner pulls in a Fragment too old for the ActivityResult API MainActivity uses.
            implementation(libs.androidx.fragment)
        }
    }
}

android {
    namespace = "dev.mothr.mobile"
    compileSdk = 36
    ndkVersion = "28.2.13676358"

    defaultConfig {
        applicationId = "dev.mothr.mobile"
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += setOf("arm64-v8a", "x86_64") }
    }
    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    packaging {
        resources.excludes += "/META-INF/{AL2.0,LGPL2.1}"
    }
}

dependencies {
    // JNA 5.17+ supports 16 KB page sizes (required on newer Android).
    "androidMainImplementation"("${libs.jna.get()}@aar")
}

compose.resources {
    publicResClass = false
    packageOfResClass = "dev.mothr.mobile.resources"
    generateResClass = always
}
