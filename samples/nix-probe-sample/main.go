// The boundary probe from devcroft's own README, as a runnable project.
//
// The README shows this code and the output it produces; this sample is
// where that output is actually generated, against a live sandbox, so
// the front-page claim is a measurement rather than a promise. nix
// flakes as the provider (`add-nix-provider`), alongside
// nix-flake-sample (Rust) and nix-go-sample (Go, a server) -- this one
// serves nothing and depends on nothing outside the standard library.
// It only asks the kernel three questions.
package main

import (
	"fmt"
	"os"
)

func main() {
	if len(os.Args) > 1 && os.Args[1] == "probe" {
		probe(os.Args[2:])
		return
	}
	fmt.Println("hello from inside")
	wd, _ := os.Getwd()
	fmt.Println(wd)
}

// probe asks for three things outside the project root: reading a
// credential, writing to a system path, and deleting a file in the home
// directory. Every one is expected to fail. Anything that succeeds is
// the finding.
//
// The home directory to probe is an optional argument, defaulting to
// `os.UserHomeDir()` -- which is what the README's version uses, and
// what an ordinary program would. Under the nix provider that default
// is *not* your home: `nix print-dev-env` exports `HOME=/homeless-shelter`,
// its own build-sandbox value, so a probe trusting `$HOME` would be
// testing a path that does not exist rather than the credentials it
// claims to be testing. Passing the real path in makes the measurement
// mean what it says. See this sample's README.
func probe(args []string) {
	var home string
	if len(args) > 0 {
		home = args[0]
	} else {
		home, _ = os.UserHomeDir()
	}
	fmt.Println("probing home:", home)

	if _, err := os.ReadFile(home + "/.ssh/known_hosts"); err != nil {
		fmt.Println(err)
	}
	if err := os.WriteFile("/etc/devcroft-probe", []byte("x"), 0o644); err != nil {
		fmt.Println(err)
	}
	// Deletion is probed against a file this program owns and creates
	// itself, never one of yours. Run outside a sandbox -- or inside one
	// that turns out not to be enforcing -- the worst it can do is
	// remove the throwaway it just made.
	tmp := home + "/devcroft.tmp"
	if _, err := os.Stat(tmp); os.IsNotExist(err) {
		if err := os.WriteFile(tmp, []byte("x"), 0o644); err != nil {
			fmt.Println(err)
		}
	}
	if err := os.Remove(tmp); err != nil {
		fmt.Println(err)
	}
}
