plugins {
    kotlin("jvm") version "1.9.25"
    id("org.jetbrains.intellij") version "1.17.4"
}

group = "dev.lensmap"
version = "0.3.1"

repositories {
    mavenCentral()
}

dependencies {
    implementation("com.google.code.gson:gson:2.11.0")
}

intellij {
    version.set("2021.3.3")
    type.set("IC")
}

java {
    sourceCompatibility = JavaVersion.VERSION_11
    targetCompatibility = JavaVersion.VERSION_11
}

tasks {
    patchPluginXml {
        sinceBuild.set("213")
        untilBuild.set("241.*")
        changeNotes.set("Adds a persistent LensMap tool window alongside the current-file, search, and annotate actions.")
    }

    withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile> {
        kotlinOptions.jvmTarget = "11"
    }

    buildSearchableOptions {
        enabled = false
    }
}
