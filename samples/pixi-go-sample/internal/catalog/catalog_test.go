package catalog

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestLoadSortsByID(t *testing.T) {
	items, err := Load()
	if err != nil {
		t.Fatal(err)
	}
	if len(items) != 3 || items[0].ID != "c-001" || items[2].ID != "t-002" {
		t.Fatalf("unexpected order: %+v", items)
	}
}

func TestFilterIsCaseInsensitive(t *testing.T) {
	items, _ := Load()
	if got := Filter(items, "KITCHEN"); len(got) != 2 {
		t.Fatalf("want 2 kitchen items, got %d", len(got))
	}
	if got := Filter(items, ""); len(got) != 3 {
		t.Fatalf("empty tag must return everything, got %d", len(got))
	}
}

func TestItemsEndpoint(t *testing.T) {
	items, _ := Load()
	rec := httptest.NewRecorder()
	Handler(items).ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "/items?tag=tea", nil))
	if rec.Code != http.StatusOK {
		t.Fatalf("status %d", rec.Code)
	}
	var got []Item
	if err := json.NewDecoder(rec.Body).Decode(&got); err != nil {
		t.Fatal(err)
	}
	if len(got) != 1 || got[0].ID != "t-002" {
		t.Fatalf("unexpected body: %+v", got)
	}
}
