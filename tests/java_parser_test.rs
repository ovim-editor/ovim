use ovim::java::parser::JavaVersion;
use std::path::PathBuf;
use tokio;

#[tokio::test]
async fn test_gradle_toolchain_java_17() {
    let content = r#"
plugins {
    id("java")
}

java {
    toolchain {
        languageVersion = JavaLanguageVersion.of(17)
    }
}
"#;

    let temp_dir = std::env::temp_dir().join("test_gradle1");
    std::fs::create_dir_all(&temp_dir).ok();
    let test_file = temp_dir.join("build.gradle");
    std::fs::write(&test_file, content).unwrap();

    // Test using public API
    let version = ovim::java::parser::detect_java_version(&temp_dir).await;

    std::fs::remove_dir_all(&temp_dir).ok();

    assert!(version.is_ok());
    let config = version.unwrap();
    assert_eq!(config.java_version, JavaVersion::Java17);
}

#[tokio::test]
async fn test_gradle_source_compatibility() {
    let content = r#"
sourceCompatibility = '21'
targetCompatibility = '21'
"#;

    let temp_dir = std::env::temp_dir().join("test_gradle2");
    std::fs::create_dir_all(&temp_dir).ok();
    let test_file = temp_dir.join("build.gradle");
    std::fs::write(&test_file, content).unwrap();

    let version = ovim::java::parser::detect_java_version(&temp_dir).await;

    std::fs::remove_dir_all(&temp_dir).ok();

    assert!(version.is_ok());
    let config = version.unwrap();
    assert_eq!(config.java_version, JavaVersion::Java21);
}

#[tokio::test]
async fn test_gradle_kts_toolchain() {
    let content = r#"
plugins {
    java
}

java {
    toolchain {
        languageVersion.set(JavaLanguageVersion.of(17))
    }
}
"#;

    let temp_dir = std::env::temp_dir().join("test_gradle_kts");
    std::fs::create_dir_all(&temp_dir).ok();
    let test_file = temp_dir.join("build.gradle.kts");
    std::fs::write(&test_file, content).unwrap();

    let version = ovim::java::parser::detect_java_version(&temp_dir).await;

    std::fs::remove_dir_all(&temp_dir).ok();

    assert!(version.is_ok());
    let config = version.unwrap();
    assert_eq!(config.java_version, JavaVersion::Java17);
}

#[tokio::test]
async fn test_maven_compiler_source() {
    let content = r#"
<project>
    <modelVersion>4.0.0</modelVersion>
    <properties>
        <maven.compiler.source>11</maven.compiler.source>
        <maven.compiler.target>11</maven.compiler.target>
    </properties>
</project>
"#;

    let temp_dir = std::env::temp_dir().join("test_maven1");
    std::fs::create_dir_all(&temp_dir).ok();
    let test_file = temp_dir.join("pom.xml");
    std::fs::write(&test_file, content).unwrap();

    let version = ovim::java::parser::detect_java_version(&temp_dir).await;

    std::fs::remove_dir_all(&temp_dir).ok();

    assert!(version.is_ok());
    let config = version.unwrap();
    assert_eq!(config.java_version, JavaVersion::Java11);
}

#[tokio::test]
async fn test_maven_java_version() {
    let content = r#"
<project>
    <properties>
        <java.version>21</java.version>
    </properties>
</project>
"#;

    let temp_dir = std::env::temp_dir().join("test_maven2");
    std::fs::create_dir_all(&temp_dir).ok();
    let test_file = temp_dir.join("pom.xml");
    std::fs::write(&test_file, content).unwrap();

    let version = ovim::java::parser::detect_java_version(&temp_dir).await;

    std::fs::remove_dir_all(&temp_dir).ok();

    assert!(version.is_ok());
    let config = version.unwrap();
    assert_eq!(config.java_version, JavaVersion::Java21);
}

#[tokio::test]
async fn test_default_to_java_17() {
    let temp_dir = std::env::temp_dir().join("no_build_files");
    std::fs::create_dir_all(&temp_dir).ok();

    let version = ovim::java::parser::detect_java_version(&temp_dir).await;

    std::fs::remove_dir(&temp_dir).ok();

    assert!(version.is_ok());
    let config = version.unwrap();
    assert_eq!(config.java_version, JavaVersion::Java17);
}

#[test]
fn test_java_version_from_number() {
    assert_eq!(JavaVersion::from_number(8), Some(JavaVersion::Java8));
    assert_eq!(JavaVersion::from_number(11), Some(JavaVersion::Java11));
    assert_eq!(JavaVersion::from_number(17), Some(JavaVersion::Java17));
    assert_eq!(JavaVersion::from_number(21), Some(JavaVersion::Java21));
    assert_eq!(JavaVersion::from_number(24), Some(JavaVersion::Java24));
    assert_eq!(JavaVersion::from_number(1_8), Some(JavaVersion::Java8)); // Old format
    assert_eq!(JavaVersion::from_number(99), None);
}

#[test]
fn test_java_version_min_jvm() {
    assert_eq!(JavaVersion::Java8.min_jvm_version(), "1.8");
    assert_eq!(JavaVersion::Java11.min_jvm_version(), "11");
    assert_eq!(JavaVersion::Java17.min_jvm_version(), "17");
    assert_eq!(JavaVersion::Java21.min_jvm_version(), "21");
    assert_eq!(JavaVersion::Java24.min_jvm_version(), "21"); // Uses Java 21 JVM
}

#[test]
fn test_java_version_ordering() {
    assert!(JavaVersion::Java8 < JavaVersion::Java11);
    assert!(JavaVersion::Java11 < JavaVersion::Java17);
    assert!(JavaVersion::Java17 < JavaVersion::Java21);
    assert!(JavaVersion::Java21 < JavaVersion::Java24);
}

#[test]
fn test_java_version_as_str() {
    assert_eq!(JavaVersion::Java8.as_str(), "8");
    assert_eq!(JavaVersion::Java11.as_str(), "11");
    assert_eq!(JavaVersion::Java17.as_str(), "17");
    assert_eq!(JavaVersion::Java21.as_str(), "21");
    assert_eq!(JavaVersion::Java24.as_str(), "24");
}
