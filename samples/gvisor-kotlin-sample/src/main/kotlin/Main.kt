// A minimal hello-world server using Ktor (JetBrains' own web framework)
// — demonstrating that the `hardened` isolation tier (add-gvisor-backend)
// is language-agnostic, the same story nix-go-sample already told for
// the *provider* layer. This sample needed nothing devcroft-specific
// beyond the same GRADLE_USER_HOME redirect pattern nix-go-sample's
// GOPATH and nix-flake-sample's CARGO_HOME already established — see
// flake.nix.
import io.ktor.http.*
import io.ktor.server.engine.*
import io.ktor.server.netty.*
import io.ktor.server.response.*
import io.ktor.server.routing.*

fun main() {
    embeddedServer(Netty, port = 8080, host = "0.0.0.0") {
        routing {
            get("/") {
                call.respondText(
                    """{"message":"hello from a devcroft sandbox"}""",
                    ContentType.Application.Json,
                )
            }
            get("/health") {
                call.respondText("ok")
            }
        }
    }.start(wait = true)
}
