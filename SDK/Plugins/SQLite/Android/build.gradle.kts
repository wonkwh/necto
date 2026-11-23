// Copyright (c) 2026 Viva Republica, Inc.

plugins {
    id("com.android.library")
    kotlin("android")
    `maven-publish`
}

android {
    namespace = "im.toss.necto.sqlite"
    compileSdk = 35

    defaultConfig {
        // Context.dataDir needs API 24; kept at 26 like the other plugins.
        minSdk = 26
    }

    publishing {
        singleVariant("release") {
            withSourcesJar()
        }
    }

    testOptions {
        unitTests.isIncludeAndroidResources = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin {
    explicitApi()
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

dependencies {
    api(project(":necto-core"))
    implementation(project(":necto-plugin-support"))
    testImplementation(testFixtures(project(":necto-core")))
    testImplementation("junit:junit:4.13.2")
    testImplementation(kotlin("test-junit"))
    testImplementation("org.robolectric:robolectric:4.16.1")
}

val generatePanelResources by tasks.registering(NectoPanelResources::class) {
    panel.set(layout.projectDirectory.dir("../Panel"))
    id.set("sqlite")
    root.set("necto/panels")
    output.set(layout.buildDirectory.dir("generated/panelResources"))
}

androidComponents {
    onVariants { variant ->
        variant.sources.resources?.addGeneratedSourceDirectory(generatePanelResources, NectoPanelResources::output)
    }
}

publishing {
    publications {
        create<MavenPublication>("release") {
            artifactId = project.name
            afterEvaluate { from(components["release"]) }
        }
    }
}
