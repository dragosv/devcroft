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
// the finding -- and for the third, the file must exist first or the
// failure means nothing.
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
	// Deletion is probed against `devcroft.tmp` -- a throwaway you
	// create yourself, by hand, before running this (see the README).
	// This program never creates it.
	//
	// Two reasons, and the second is the one that matters. It is safe:
	// the only file this can delete is one you made as a target, so run
	// outside a sandbox -- or inside one that turns out not to be
	// enforcing -- the worst case is losing a file you created to lose.
	// And it is honest: an earlier version created the file itself,
	// which cannot work, because creating it in $HOME is refused by the
	// same boundary the deletion is meant to test. The remove then
	// returned ENOENT, and "no such file or directory" is not evidence
	// that deletion was refused -- there was simply nothing there. The
	// file has to already exist for this line to measure anything.
	if err := os.Remove(home + "/devcroft.tmp"); err != nil {
		fmt.Println(err)
	}
}
