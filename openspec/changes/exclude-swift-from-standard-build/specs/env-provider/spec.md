## ADDED Requirements

### Requirement: Native Swift provider code is absent from standard builds
The standard `devcroft` build SHALL NOT compile or link executable Swift toolchain resolution (`src/provider/swift.rs`) and SHALL NOT autodetect `Package.swift` as a supported environment definition. Keeping the implementation in the repository behind a non-default research Cargo feature SHALL NOT alter the standard build's provider set or dependencies.

#### Scenario: Marker swift and xcrun on PATH are untouched
- **WHEN** `PATH` starts with a directory holding marker-writing `swift` and `xcrun` binaries and `devcroft up` or `devcroft doctor` runs on a `provider = "swift"` project in a standard build
- **THEN** neither marker binary is executed
