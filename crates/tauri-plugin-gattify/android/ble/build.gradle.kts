plugins {
  id("com.android.library") version "8.9.1"
  kotlin("android") version "2.1.20"
}

android {
  namespace = "dev.taurible.plugin"
  compileSdk = 35

  defaultConfig {
    minSdk = 26
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
  implementation("androidx.annotation:annotation:1.9.1")
  compileOnly("app.tauri:tauri-android:2.11.0")
}

