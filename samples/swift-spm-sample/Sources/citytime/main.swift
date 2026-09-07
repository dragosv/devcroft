// **Deliberately macOS-only, and that is the point of the sample.**
//
// devcroft's swift provider refuses a package a closure-tier provider
// could serve — a portable Swift package builds fine from nix or flox,
// where it gets reproducibility, a hook-free activation, and no host-side
// execution of Package.swift. The provider is only justified where none
// of that is available, so the sample has to be a package that genuinely
// cannot build on Linux.
//
// `import AppKit` is exactly that: unguarded, so it cannot compile
// anywhere but an Apple platform. Swapping it for Foundation would make
// this sample portable, and `devcroft up` would then refuse it and point
// at flox — which is the correct behaviour, not a bug.
import AppKit
import Foundation

let now = Date()
let formatter = DateFormatter()
formatter.dateFormat = "yyyy-MM-dd HH:mm:ss"
formatter.timeZone = TimeZone(identifier: "UTC")

print("citytime: \(formatter.string(from: now)) UTC")

// An AppKit query, so the Apple dependency is real rather than an import
// added to satisfy a check. `NSWorkspace` needs no availability gate and
// no run loop, unlike `NSApplication.effectiveAppearance`.
let menuBarOwner = NSWorkspace.shared.frontmostApplication?.localizedName ?? "none"
print("citytime: frontmost application is \(menuBarOwner)")
print("citytime: built and run inside a devcroft sandbox, artifact tier")
