// Depends on nothing beyond Foundation, for the same reason
// `devbox-citytime-sample` depends on nothing beyond `std`: devcroft's
// swift provider runs no host-side step that could fetch dependencies
// inside the sandbox, so a sample with them would need network grants to
// demonstrate a provider feature that has nothing to do with the network.
import Foundation

let now = Date()
let formatter = DateFormatter()
formatter.dateFormat = "yyyy-MM-dd HH:mm:ss"
formatter.timeZone = TimeZone(identifier: "UTC")

print("citytime: \(formatter.string(from: now)) UTC")
print("built and run inside a devcroft sandbox, artifact tier")
