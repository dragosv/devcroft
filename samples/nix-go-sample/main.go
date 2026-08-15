// A minimal hello-world server using Gin — the most-starred Go web
// framework (github.com/gin-gonic/gin) — demonstrating that the nix
// provider (add-nix-provider) is language-agnostic: nix-flake-sample
// covers Rust, this one covers Go, and neither needed anything
// devcroft-specific beyond the same flake.nix + GOPATH/CARGO_HOME
// redirect pattern.
package main

import (
	"net/http"

	"github.com/gin-gonic/gin"
)

func main() {
	r := gin.Default()

	r.GET("/", func(c *gin.Context) {
		c.JSON(http.StatusOK, gin.H{"message": "hello from a devcroft sandbox"})
	})

	r.GET("/health", func(c *gin.Context) {
		c.String(http.StatusOK, "ok")
	})

	r.Run(":8080")
}
