// Minimal Kotlin/Ktor web server — see README.md for what this sample
// demonstrates (the `hardened` isolation tier, add-gvisor-backend) and
// why Ktor (JetBrains' own framework, ships the same JDK+Gradle ecosystem
// the other samples' languages don't need) over Spring Boot's much
// heavier dependency graph.

plugins {
    kotlin("jvm") version "2.4.10"
    application
}

group = "dev.devcroft.sample"
version = "0.1.0"

repositories {
    mavenCentral()
}

val ktorVersion = "3.5.2"

dependencies {
    implementation("io.ktor:ktor-server-core-jvm:$ktorVersion")
    implementation("io.ktor:ktor-server-netty-jvm:$ktorVersion")
    implementation("ch.qos.logback:logback-classic:1.6.3")
}

application {
    mainClass.set("MainKt")
}

kotlin {
    jvmToolchain(21)
}

tasks.test {
    useJUnitPlatform()
}
