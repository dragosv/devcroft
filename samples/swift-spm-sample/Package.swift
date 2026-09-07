// swift-tools-version:5.9
import PackageDescription

// Deliberately dependency-free. `Package.resolved` is required by
// devcroft's swift provider only when dependencies are declared, because
// SwiftPM does not write the file for a package that has none — this
// sample is the case that proves the conditional half of that rule, and
// it is also what lets the sample build with no network at all.
let package = Package(
    name: "citytime",
    targets: [.executableTarget(name: "citytime", path: "Sources/citytime")]
)
