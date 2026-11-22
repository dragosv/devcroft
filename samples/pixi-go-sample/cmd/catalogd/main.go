// Command catalogd serves the embedded catalog on a loopback address.
package main

import (
	"flag"
	"log"
	"net/http"
	"time"

	"example.com/catalog/internal/catalog"
)

func main() {
	addr := flag.String("addr", "127.0.0.1:8080", "listen address")
	flag.Parse()
	items, err := catalog.Load()
	if err != nil {
		log.Fatal(err)
	}
	srv := &http.Server{Addr: *addr, Handler: catalog.Handler(items), ReadHeaderTimeout: 5 * time.Second}
	log.Printf("catalogd listening on %s", *addr)
	log.Fatal(srv.ListenAndServe())
}
