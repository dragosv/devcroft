// Package catalog serves a small, embedded product catalog over HTTP.
package catalog

import (
	_ "embed"
	"encoding/json"
	"fmt"
	"net/http"
	"sort"
	"strings"
)

//go:embed items.json
var itemsJSON []byte

// Item is one catalog entry.
type Item struct {
	ID    string   `json:"id"`
	Name  string   `json:"name"`
	Price float64  `json:"price"`
	Tags  []string `json:"tags"`
}

// Load parses the embedded catalog and returns it sorted by ID.
func Load() ([]Item, error) {
	var items []Item
	if err := json.Unmarshal(itemsJSON, &items); err != nil {
		return nil, fmt.Errorf("catalog: %w", err)
	}
	sort.Slice(items, func(i, j int) bool { return items[i].ID < items[j].ID })
	return items, nil
}

// Filter returns the items carrying tag, or all items when tag is empty.
func Filter(items []Item, tag string) []Item {
	if tag == "" {
		return items
	}
	var out []Item
	for _, it := range items {
		for _, t := range it.Tags {
			if strings.EqualFold(t, tag) {
				out = append(out, it)
				break
			}
		}
	}
	return out
}

// Handler serves GET /items?tag=<tag> and GET /healthz.
func Handler(items []Item) http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /healthz", func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte("ok\n"))
	})
	mux.HandleFunc("GET /items", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(Filter(items, r.URL.Query().Get("tag")))
	})
	return mux
}
